use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event as CEvent, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::client::PmrClient;
use crate::process::ProcessDescription;
use crate::ux::{bytes_to_size, process_sort_order, sort_processes, time_since};

struct TerminalCleanupGuard;

impl Drop for TerminalCleanupGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
    }
}

/// Interactive 4-pane dashboard (`pmr monit` / `pmr dashboard`), matching `lib/API/Dashboard.js`.
pub async fn run_dashboard(client: &PmrClient, sort_spec: &str) -> Result<()> {
    enable_raw_mode()?;
    let _cleanup_guard = TerminalCleanupGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;

    // Install panic hook to guarantee terminal restoration on panic
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        prev_hook(info);
    }));

    let stop_flag = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    {
        let stop_sig = Arc::clone(&stop_flag);
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
                let _ = sigterm.recv().await;
                stop_sig.store(true, Ordering::SeqCst);
            }
        });
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_dashboard_loop(&mut terminal, client, &stop_flag, sort_spec).await;

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show);
    let _ = terminal.show_cursor();

    res
}

async fn run_dashboard_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    client: &PmrClient,
    stop_flag: &Arc<AtomicBool>,
    sort_spec: &str,
) -> Result<()> {
    let mut sort_spec = sort_spec.to_string();
    let mut processes: Vec<ProcessDescription> = client.list().await.unwrap_or_default();
    sort_processes(&mut processes, Some(&sort_spec), "id");
    let mut selected_idx: usize = 0;
    let mut active_pane: usize = 0; // 0: list, 1: logs, 2: metrics, 3: metadata
    let mut scroll_offsets: [u16; 4] = [0; 4];
    let mut logs_by_id: BTreeMap<u32, VecDeque<(bool, String)>> = BTreeMap::new();

    let mut bus_rx = client.subscribe_bus_bounded().await.ok();
    let mut last_refresh = Instant::now();

    loop {
        if stop_flag.load(Ordering::SeqCst) {
            break;
        }

        // Only the selected process needs an output cache. Read bounded file tails
        // when selection changes instead of loading every process's entire logs.
        if let Some(p) = processes.get(selected_idx) {
            if !logs_by_id.contains_key(&p.pm_id) {
                logs_by_id.clear();
                let mut q = VecDeque::new();
                for (is_err, path, count) in [
                    (false, &p.pm2_env.pm_out_log_path, 100),
                    (true, &p.pm2_env.pm_err_log_path, 50),
                ] {
                    for line in crate::live::read_tail(path, count) {
                        q.push_back((is_err, line));
                    }
                }
                logs_by_id.insert(p.pm_id, q);
            }
        } else {
            logs_by_id.clear();
        }

        if let Some(ref mut rx) = bus_rx {
            for _ in 0..128 {
                let Ok(ev) = rx.try_recv() else { break };
                if ev.event == "log:out" || ev.event == "log:err" {
                    if let Some(id) = ev.process.pm_id.as_u64().map(|n| n as u32) {
                        let is_err = ev.event == "log:err";
                        if let Some(data) = ev.data.get("data").and_then(|d| d.as_str()) {
                            let is_selected = processes
                                .get(selected_idx)
                                .map(|p| p.pm_id == id)
                                .unwrap_or(false);
                            if !is_selected {
                                continue;
                            }
                            let q = logs_by_id.entry(id).or_default();
                            for l in data.lines() {
                                let mut end = l.len().min(4096);
                                while !l.is_char_boundary(end) {
                                    end -= 1;
                                }
                                q.push_back((is_err, l[..end].to_string()));
                                if q.len() > 400 {
                                    q.pop_front();
                                } else if is_selected && scroll_offsets[1] > 0 {
                                    scroll_offsets[1] = scroll_offsets[1].saturating_add(1);
                                }
                            }
                        }
                    }
                }
            }
        }

        if last_refresh.elapsed() >= Duration::from_millis(800) {
            if let Ok(new_list) = client.list().await {
                if refresh_processes(&mut processes, &mut selected_idx, new_list, &sort_spec) {
                    scroll_offsets[1] = 0;
                }
            }
            last_refresh = Instant::now();
        }

        let selected_proc = processes.get(selected_idx).cloned();

        terminal.draw(|f| {
            let size = f.area();
            let root_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(68),
                    Constraint::Percentage(27),
                    Constraint::Length(1),
                ])
                .split(size);

            let top_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
                .split(root_chunks[0]);

            let bottom_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
                .split(root_chunks[1]);

            let pane_border = |idx: usize| -> Style {
                if active_pane == idx {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                }
            };

            // 1. Top-Left: Process List
            let items: Vec<ListItem> = processes
                .iter()
                .map(|p| {
                    let mem_mb = (p.monit.memory as f64) / 1_048_576.0;
                    let status_color = match p.pm2_env.status.as_str() {
                        "online" => Color::Green,
                        "launching" | "waiting restart" => Color::Yellow,
                        _ => Color::Red,
                    };
                    let line = Line::from(vec![
                        Span::styled(
                            format!("[{:>2}] ", p.pm_id),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{:<14} ", p.name),
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!(
                            "Mem: {:>5.1} MB  CPU: {:>4.1}%  ",
                            mem_mb, p.monit.cpu
                        )),
                        Span::styled(
                            p.pm2_env.status.clone(),
                            Style::default()
                                .fg(status_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]);
                    ListItem::new(line)
                })
                .collect();

            let mut list_state = ListState::default();
            if !processes.is_empty() {
                list_state.select(Some(selected_idx));
            }

            let proc_list = List::new(items)
                .block(
                    Block::default()
                        .title(" Process List ")
                        .title_bottom(sort_control_label(&sort_spec))
                        .borders(Borders::ALL)
                        .border_style(pane_border(0)),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                );
            f.render_stateful_widget(proc_list, top_chunks[0], &mut list_state);

            // 2. Top-Right: Logs
            let log_title = match selected_proc {
                Some(ref sp) => format!(" {} Logs ", sp.name),
                None => " Logs ".to_string(),
            };
            let log_lines: Vec<Line> = selected_proc
                .as_ref()
                .and_then(|sp| logs_by_id.get(&sp.pm_id))
                .map(|q| {
                    let max_rows = top_chunks[1].height.saturating_sub(2) as usize;
                    let total_skip = q
                        .len()
                        .saturating_sub(max_rows)
                        .saturating_sub(scroll_offsets[1] as usize);
                    q.iter()
                        .skip(total_skip)
                        .take(max_rows)
                        .map(|(is_err, text)| {
                            let prefix_color = if *is_err { Color::Red } else { Color::Green };
                            let name = selected_proc
                                .as_ref()
                                .map(|p| p.name.as_str())
                                .unwrap_or("");
                            Line::from(vec![
                                Span::styled(
                                    format!("{} > ", name),
                                    Style::default().fg(prefix_color),
                                ),
                                Span::raw(text.clone()),
                            ])
                        })
                        .collect()
                })
                .unwrap_or_default();

            let logs_widget = Paragraph::new(log_lines)
                .block(
                    Block::default()
                        .title(log_title)
                        .borders(Borders::ALL)
                        .border_style(pane_border(1)),
                )
                .wrap(Wrap { trim: false });
            f.render_widget(logs_widget, top_chunks[1]);

            // 3. Bottom-Left: Custom Metrics
            let mut metric_lines = Vec::new();
            if let Some(ref sp) = selected_proc {
                metric_lines.push(Line::from(vec![
                    Span::styled("Memory usage: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(
                        bytes_to_size(sp.monit.memory, 1),
                        Style::default().fg(Color::Cyan),
                    ),
                ]));
                metric_lines.push(Line::from(vec![
                    Span::styled("CPU usage   : ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!("{}%", sp.monit.cpu),
                        Style::default().fg(Color::Green),
                    ),
                ]));
                for (k, m) in &sp.pm2_env.axm_monitor {
                    let v = m
                        .value
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| m.value.to_string());
                    metric_lines.push(Line::from(vec![
                        Span::styled(
                            format!("{}: ", k),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!("{} {}", v, m.unit)),
                    ]));
                }
            }
            let metrics_widget = Paragraph::new(metric_lines)
                .scroll((scroll_offsets[2], 0))
                .block(
                    Block::default()
                        .title(" Custom Metrics ")
                        .borders(Borders::ALL)
                        .border_style(pane_border(2)),
                );
            f.render_widget(metrics_widget, bottom_chunks[0]);

            // 4. Bottom-Right: Metadata
            let mut meta_lines = Vec::new();
            if let Some(ref sp) = selected_proc {
                let e = &sp.pm2_env;
                let uptime = if e.status == "online" && e.pm_uptime > 0 {
                    time_since(e.pm_uptime)
                } else {
                    "0".to_string()
                };
                let pairs = [
                    ("App Name", e.name.clone()),
                    ("Namespace", e.namespace.clone()),
                    ("Version", e.version.clone()),
                    ("Restarts", e.restart_time.to_string()),
                    ("Uptime", uptime),
                    ("Script path", e.pm_exec_path.clone()),
                    ("Script args", e.args.join(" ")),
                    ("Interpreter", e.exec_interpreter.clone()),
                    ("Exec mode", e.exec_mode.clone()),
                    ("Exec cwd", e.pm_cwd.clone()),
                    (
                        "Watch & reload",
                        if e.watch {
                            "✔".to_string()
                        } else {
                            "✘".to_string()
                        },
                    ),
                ];
                for (k, v) in pairs {
                    meta_lines.push(Line::from(vec![
                        Span::styled(
                            format!("{:<16}: ", k),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(v),
                    ]));
                }
            }
            let meta_widget = Paragraph::new(meta_lines)
                .scroll((scroll_offsets[3], 0))
                .block(
                    Block::default()
                        .title(" Metadata ")
                        .borders(Borders::ALL)
                        .border_style(pane_border(3)),
                );
            f.render_widget(meta_widget, bottom_chunks[1]);

            // 5. Status Bar
            let help_line = Line::from(vec![
                Span::raw(
                    " left/right/tab: boards | up/down: select/scroll | o: sort | r: restart | s: stop | q/Ctrl-C: exit ",
                ),
                Span::styled(
                    "  PMR Dashboard ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            f.render_widget(Paragraph::new(help_line), root_chunks[2]);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let CEvent::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('o') => {
                        toggle_process_sort(&mut processes, &mut selected_idx, &mut sort_spec);
                    }
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Left => {
                        active_pane = if active_pane == 0 { 3 } else { active_pane - 1 };
                    }
                    KeyCode::Right | KeyCode::Tab => {
                        active_pane = (active_pane + 1) % 4;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if active_pane == 0 {
                            if !processes.is_empty() && selected_idx > 0 {
                                selected_idx -= 1;
                                scroll_offsets[1] = 0;
                            }
                        } else if active_pane == 1 {
                            scroll_offsets[1] = scroll_offsets[1].saturating_add(1);
                        } else {
                            scroll_offsets[active_pane] =
                                scroll_offsets[active_pane].saturating_sub(1);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if active_pane == 0 {
                            if !processes.is_empty() && selected_idx + 1 < processes.len() {
                                selected_idx += 1;
                                scroll_offsets[1] = 0;
                            }
                        } else if active_pane == 1 {
                            scroll_offsets[1] = scroll_offsets[1].saturating_sub(1);
                        } else {
                            scroll_offsets[active_pane] =
                                scroll_offsets[active_pane].saturating_add(1);
                        }
                    }
                    KeyCode::PageUp => {
                        if active_pane == 1 {
                            scroll_offsets[1] = scroll_offsets[1].saturating_add(10);
                        } else if active_pane > 1 {
                            scroll_offsets[active_pane] =
                                scroll_offsets[active_pane].saturating_sub(5);
                        }
                    }
                    KeyCode::PageDown => {
                        if active_pane == 1 {
                            scroll_offsets[1] = scroll_offsets[1].saturating_sub(10);
                        } else if active_pane > 1 {
                            scroll_offsets[active_pane] =
                                scroll_offsets[active_pane].saturating_add(5);
                        }
                    }
                    KeyCode::Char('r') => {
                        if let Some(ref sp) = selected_proc {
                            let _ = client
                                .restart(&sp.pm_id.to_string(), &Default::default())
                                .await;
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Some(ref sp) = selected_proc {
                            let _ = client
                                .stop(&sp.pm_id.to_string(), &Default::default())
                                .await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Keep selection attached to a process even when sorting moves its row.
/// Return whether the selected process changed (for example, after deletion).
fn refresh_processes(
    processes: &mut Vec<ProcessDescription>,
    selected_idx: &mut usize,
    mut new_list: Vec<ProcessDescription>,
    sort_spec: &str,
) -> bool {
    let selected_id = processes.get(*selected_idx).map(|p| p.pm_id);
    sort_processes(&mut new_list, Some(sort_spec), "id");
    *selected_idx = selected_id
        .and_then(|id| new_list.iter().position(|p| p.pm_id == id))
        .unwrap_or_else(|| (*selected_idx).min(new_list.len().saturating_sub(1)));
    let changed = new_list.get(*selected_idx).map(|p| p.pm_id) != selected_id;
    *processes = new_list;
    changed
}

fn sort_control_label(sort_spec: &str) -> String {
    let (field, desc) = process_sort_order(Some(sort_spec), "id");
    format!(
        " Sort: {} {} [o] ",
        field,
        if desc { "desc" } else { "asc" }
    )
}

fn toggle_process_sort(
    processes: &mut [ProcessDescription],
    selected_idx: &mut usize,
    sort_spec: &mut String,
) {
    let (field, desc) = process_sort_order(Some(sort_spec), "id");
    *sort_spec = format!("{}:{}", field, if desc { "asc" } else { "desc" });
    let selected_id = processes.get(*selected_idx).map(|p| p.pm_id);
    sort_processes(processes, Some(sort_spec), "id");
    *selected_idx = selected_id
        .and_then(|id| processes.iter().position(|p| p.pm_id == id))
        .unwrap_or(0);
}

/// Legacy terminal bar monitor (`pmr imonit`), matching `lib/API/Monit.js`.
pub async fn run_imonit(client: &PmrClient, use_color: bool) -> Result<()> {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_memory();
    let total_mem = sys.total_memory().max(1);

    loop {
        let procs = client.list().await?;
        print!("\x1b[2J\x1b[H");
        if use_color {
            println!("\x1b[32m⌬ PMR \x1b[39mmonitoring\x1b[96m (Press Ctrl-C to exit)\x1b[39m\n");
        } else {
            println!("PMR monitoring (Press Ctrl-C to exit)\n");
        }
        for p in &procs {
            let cpu_pct = p.monit.cpu.clamp(0.0, 100.0);
            let cpu_filled = ((cpu_pct / 100.0) * 30.0).round() as usize;
            let cpu_bar = if use_color {
                format!(
                    "\x1b[44m{}\x1b[49m{}",
                    " ".repeat(cpu_filled),
                    "-".repeat(30usize.saturating_sub(cpu_filled))
                )
            } else {
                format!(
                    "{}{}",
                    "#".repeat(cpu_filled),
                    "-".repeat(30usize.saturating_sub(cpu_filled))
                )
            };

            let mem_ratio = ((p.monit.memory as f64) / (total_mem as f64 / 5.0)).clamp(0.0, 1.0);
            let mem_filled = (mem_ratio * 30.0).round() as usize;
            let mem_bar = if use_color {
                format!(
                    "\x1b[41m{}\x1b[49m{}",
                    " ".repeat(mem_filled),
                    "-".repeat(30usize.saturating_sub(mem_filled))
                )
            } else {
                format!(
                    "{}{}",
                    "#".repeat(mem_filled),
                    "-".repeat(30usize.saturating_sub(mem_filled))
                )
            };

            println!(
                " ● {:<20} [{:>2}] [{}] [{}]",
                p.name, p.pm_id, p.pm2_env.exec_mode, p.pm2_env.status
            );
            println!("   CPU: [{}] {:>5.1}%", cpu_bar, p.monit.cpu);
            println!(
                "   MEM: [{}] {}\n",
                mem_bar,
                bytes_to_size(p.monit.memory, 1)
            );
        }
        io::stdout().flush()?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{Monit, Pm2Env};

    fn process(id: u32) -> ProcessDescription {
        ProcessDescription {
            pm_id: id,
            pid: 0,
            name: format!("worker-{id}"),
            pm2_env: Pm2Env::from_pm2_value(serde_json::json!({
                "pm_id": id,
                "name": format!("worker-{id}"),
                "pm_exec_path": "/bin/sh",
                "pm_cwd": "/tmp",
                "pm_out_log_path": "/dev/null",
                "pm_err_log_path": "/dev/null",
                "pm_pid_path": "/dev/null"
            }))
            .unwrap(),
            monit: Monit::default(),
        }
    }

    #[test]
    fn dashboard_sort_shortcut_preserves_selection_in_both_directions() {
        let mut processes = vec![process(3), process(2), process(1)];
        let mut selected = 0;
        let mut sort = "desc".to_string();
        assert_eq!(sort_control_label(&sort), " Sort: id desc [o] ");
        toggle_process_sort(&mut processes, &mut selected, &mut sort);
        assert_eq!(
            processes.iter().map(|p| p.pm_id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(selected, 2);
        assert_eq!(sort_control_label(&sort), " Sort: id asc [o] ");
        toggle_process_sort(&mut processes, &mut selected, &mut sort);
        assert_eq!(
            processes.iter().map(|p| p.pm_id).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert_eq!(selected, 0);
    }

    #[test]
    fn dashboard_refresh_keeps_sort_and_tracks_selected_id() {
        let mut processes = vec![process(2), process(1)];
        let mut selected = 0;
        assert!(!refresh_processes(
            &mut processes,
            &mut selected,
            vec![process(1), process(2), process(3)],
            "desc"
        ));
        assert_eq!(
            processes.iter().map(|p| p.pm_id).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert_eq!(selected, 1);
        assert!(refresh_processes(
            &mut processes,
            &mut selected,
            vec![process(3), process(1)],
            "desc"
        ));
        assert_eq!(processes[selected].pm_id, 1);
        assert!(refresh_processes(
            &mut processes,
            &mut selected,
            vec![],
            "desc"
        ));
        assert_eq!(selected, 0);
    }

    #[test]
    fn dashboard_shortcut_reverses_field_sort_and_handles_empty_list() {
        let mut processes = vec![process(1), process(2)];
        processes[0].monit.memory = 100;
        processes[1].monit.memory = 200;
        let mut selected = 0;
        let mut sort = "memory:asc".to_string();
        toggle_process_sort(&mut processes, &mut selected, &mut sort);
        assert_eq!(sort, "memory:desc");
        assert_eq!(processes[0].pm_id, 2);
        assert_eq!(processes[selected].pm_id, 1);
        processes.clear();
        toggle_process_sort(&mut processes, &mut selected, &mut sort);
        assert_eq!(selected, 0);
    }
}
