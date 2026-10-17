use anyhow::{anyhow, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, IsTerminal, Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use crate::client::PmrClient;
use crate::process::Pm2Env;
use crate::protocol::{BusEvent, RpcRequest};
use crate::ux::LogOptions;

const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_LINE_BYTES: usize = 4096;

pub struct LogSelector {
    target: String,
    regex: Option<Regex>,
}

impl LogSelector {
    pub fn new(target: &str) -> Result<Self> {
        let regex = if target.starts_with('/') && target.ends_with('/') && target.len() > 2 {
            Some(Regex::new(&target[1..target.len() - 1])?)
        } else {
            None
        };
        Ok(Self {
            target: target.to_string(),
            regex,
        })
    }

    pub fn matches(&self, id: u32, name: &str, namespace: &str) -> bool {
        if self.target == "all" || self.target.is_empty() {
            return true;
        }
        if let Some(regex) = &self.regex {
            return regex.is_match(name);
        }
        self.target == name || self.target == namespace || self.target.parse::<u32>() == Ok(id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveProcess {
    #[serde(default)]
    pub history_id: Option<String>,
    pub pm_id: u32,
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub unique_id: String,
    pub created_at: u64,
    pub out_path: String,
    pub err_path: String,
}

impl From<&Pm2Env> for LiveProcess {
    fn from(env: &Pm2Env) -> Self {
        Self {
            history_id: None,
            pm_id: env.pm_id,
            name: env.name.clone(),
            namespace: env.namespace.clone(),
            status: env.status.clone(),
            exit_code: env.exit_code,
            unique_id: env.unique_id.clone(),
            created_at: env.created_at.unwrap_or(0),
            out_path: env.pm_out_log_path.clone(),
            err_path: env.pm_err_log_path.clone(),
        }
    }
}

struct LiveState {
    process: Option<LiveProcess>,
    lines: VecDeque<(bool, String)>,
    bytes: usize,
    max_lines: usize,
    selection_version: u64,
}

impl LiveState {
    fn new(max_lines: usize) -> Self {
        Self {
            process: None,
            lines: VecDeque::new(),
            bytes: 0,
            max_lines,
            selection_version: 0,
        }
    }

    fn clear_lines(&mut self) {
        self.lines.clear();
        self.bytes = 0;
    }

    fn push(&mut self, is_err: bool, text: &str) {
        let mut end = text.len().min(MAX_LINE_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        // Strip terminal control characters from child output before rendering it.
        let text: String = text[..end]
            .chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .collect();
        self.bytes += text.len() + 1;
        self.lines.push_back((is_err, text));
        while self.lines.len() > self.max_lines || self.bytes > MAX_OUTPUT_BYTES {
            if let Some((_, old)) = self.lines.pop_front() {
                self.bytes -= old.len() + 1;
            }
        }
    }

    fn apply(&mut self, event: &BusEvent, selector: &LogSelector, opts: &LogOptions) {
        let Some(id) = event
            .process
            .pm_id
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
        else {
            return;
        };
        if !selector.matches(id, &event.process.name, &event.process.namespace) {
            return;
        }
        let lifecycle = event.event == "process:event";
        let log = event.event == "log:out" || event.event == "log:err";
        if !lifecycle && !log {
            return;
        }
        let action = event
            .data
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let switch = self.process.as_ref().is_none_or(|p| {
            if p.history_id.is_some() {
                lifecycle && matches!(action, "start" | "online") && event.at > p.created_at
            } else {
                id > p.pm_id || (p.status == "deleted" && lifecycle && action == "online")
            }
        });
        if switch {
            self.selection_version = self.selection_version.wrapping_add(1);
            self.process = Some(LiveProcess {
                history_id: None,
                pm_id: id,
                name: event.process.name.clone(),
                namespace: event.process.namespace.clone(),
                status: "online".into(),
                exit_code: None,
                unique_id: String::new(),
                created_at: event.at,
                out_path: String::new(),
                err_path: String::new(),
            });
            self.clear_lines();
        }
        let Some(process) = self
            .process
            .as_mut()
            .filter(|p| p.pm_id == id && p.history_id.is_none())
        else {
            return;
        };
        if lifecycle {
            if action == "delete" {
                process.status = "deleted".into();
            } else if let Some(status) = event
                .data
                .pointer("/process/status")
                .and_then(|v| v.as_str())
            {
                process.status = status.into();
                if status == "online" {
                    process.exit_code = None;
                }
            }
            if let Some(code) = event
                .data
                .pointer("/process/exit_code")
                .and_then(|v| v.as_i64())
            {
                process.exit_code = Some(code as i32);
            }
        } else {
            let is_err = event.event == "log:err";
            if (opts.out_only && is_err) || (opts.err_only && !is_err) {
                return;
            }
            if let Some(data) = event.data.get("data").and_then(|v| v.as_str()) {
                for line in data.lines() {
                    self.push(is_err, line);
                }
            }
        }
    }

    fn seed(&mut self, process: LiveProcess, opts: &LogOptions) {
        if self.process.as_ref().is_none_or(|current| {
            current.pm_id != process.pm_id
                || (!current.unique_id.is_empty() && current.unique_id != process.unique_id)
        }) {
            self.selection_version = self.selection_version.wrapping_add(1);
        }
        self.clear_lines();
        let mut paths = Vec::new();
        if !opts.err_only {
            paths.push((false, &process.out_path));
        }
        if !opts.out_only && (opts.err_only || process.err_path != process.out_path) {
            paths.push((true, &process.err_path));
        }
        paths.sort_by_key(|(_, path)| std::fs::metadata(path).ok().and_then(|m| m.modified().ok()));
        for (is_err, path) in paths {
            for line in read_tail(path, self.max_lines) {
                self.push(is_err, &line);
            }
        }
        self.process = Some(process);
    }

    fn refresh_snapshot(&mut self, process: LiveProcess, opts: &LogOptions, recover: bool) {
        // A short-lived latest process may already have been deleted. Even if its
        // delete event was dropped, do not replace its result with an older job.
        let current_is_newer = self.process.as_ref().is_some_and(|current| {
            (current.pm_id != process.pm_id || current.history_id != process.history_id)
                && (current.created_at > process.created_at
                    || (current.created_at == process.created_at && current.pm_id > process.pm_id))
        });
        if !current_is_newer {
            let changed = self.process.as_ref().is_none_or(|current| {
                current.pm_id != process.pm_id
                    || (!current.unique_id.is_empty() && current.unique_id != process.unique_id)
            });
            if changed || recover || self.lines.is_empty() {
                self.seed(process, opts);
            } else {
                // Keep the stdout/stderr event order and the user's scroll position.
                self.process = Some(process);
            }
        }
    }
}

pub(crate) fn read_tail(path: &str, max_lines: usize) -> Vec<String> {
    if path.is_empty() || path == "/dev/null" {
        return Vec::new();
    }
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(metadata) = file.metadata() else {
        return Vec::new();
    };
    if !metadata.is_file() {
        return Vec::new();
    }
    let start = metadata.len().saturating_sub(MAX_OUTPUT_BYTES as u64);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file
        .take(MAX_OUTPUT_BYTES as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Vec::new();
    }
    let content = String::from_utf8_lossy(&bytes);
    let tail: Vec<_> = content
        .lines()
        .rev()
        .take(max_lines)
        .map(str::to_string)
        .collect();
    tail.into_iter().rev().collect()
}

async fn latest_process(client: &PmrClient, target: &str) -> Result<Option<LiveProcess>> {
    let response = client
        .request_with_timeout(
            &RpcRequest::GetLatestLogProcess {
                target: target.into(),
            },
            Duration::from_secs(2),
        )
        .await?;
    if !response.ok {
        return Err(anyhow!(response.error.unwrap_or_default()));
    }
    Ok(serde_json::from_value(response.data)?)
}

async fn refresh_live(
    state: &mut LiveState,
    client: &PmrClient,
    process: LiveProcess,
    opts: &LogOptions,
    recover: bool,
) -> Result<()> {
    let history_id = process.history_id.clone();
    state.refresh_snapshot(process, opts, recover);
    if let Some(id) = history_id {
        if state.process.as_ref().and_then(|p| p.history_id.as_ref()) != Some(&id) {
            return Ok(());
        }
        let stream = if opts.err_only {
            Some("err".into())
        } else if opts.out_only {
            Some("out".into())
        } else {
            None
        };
        let output = crate::history_cli::request(
            client,
            crate::history::Request::Output {
                id,
                lines: opts.lines.min(100_000),
                stream,
            },
        )
        .await?;
        state.clear_lines();
        for line in output["text"].as_str().unwrap_or_default().lines() {
            state.push(opts.err_only, line);
        }
    }
    Ok(())
}

struct ScreenGuard;
impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
    }
}

pub async fn run_live_logs(client: &PmrClient, opts: LogOptions) -> Result<()> {
    if !io::stdout().is_terminal() {
        return Err(anyhow!("logs --live needs an interactive terminal"));
    }
    if opts.lines == 0 {
        return Err(anyhow!("logs --live requires --lines greater than 0"));
    }
    let selector = LogSelector::new(&opts.target)?;
    let mut events = client.subscribe_bus_bounded().await?;
    let mut state = LiveState::new(opts.lines);
    let (initial, optimized) = match latest_process(client, &opts.target).await {
        Ok(process) => (process, true),
        Err(err) if err.to_string().contains("unknown variant") => {
            // Compatibility with an already-running older daemon: one inventory read only.
            let process = client
                .list()
                .await?
                .into_iter()
                .filter(|p| selector.matches(p.pm_id, &p.name, &p.pm2_env.namespace))
                .max_by_key(|p| p.pm_id)
                .map(|p| LiveProcess::from(&p.pm2_env));
            (process, false)
        }
        Err(err) => return Err(err),
    };
    if let Some(process) = initial {
        refresh_live(&mut state, client, process, &opts, true).await?;
    }
    let mut recover_logs = false;
    // Apply events that arrived during lookup; never drain an unbounded burst.
    for _ in 0..128 {
        let Ok(event) = events.try_recv() else { break };
        if event.event == "pmr:gap" {
            recover_logs = true;
        }
        state.apply(&event, &selector, &opts);
    }
    if let Some(process) = state.process.clone().filter(|p| !p.out_path.is_empty()) {
        state.seed(process, &opts);
    }
    enable_raw_mode()?;
    let _guard = ScreenGuard;
    execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_resync = Instant::now();
    let mut scroll_offset = 0usize;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        let previous_selection = state.selection_version;
        #[cfg(unix)]
        tokio::select! {
            _ = interval.tick() => {},
            _ = terminate.recv() => break,
        }
        #[cfg(not(unix))]
        interval.tick().await;
        for _ in 0..128 {
            match events.try_recv() {
                Ok(event) if event.event == "pmr:gap" => recover_logs = true,
                Ok(event) => state.apply(&event, &selector, &opts),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    return Err(anyhow!("PMR event connection closed"))
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            }
        }
        if optimized && (recover_logs || last_resync.elapsed() >= Duration::from_secs(2)) {
            // Repair dropped events using a single indexed record and bounded file tails.
            if let Some(process) = latest_process(client, &opts.target).await? {
                refresh_live(&mut state, client, process, &opts, recover_logs).await?;
            }
            recover_logs = false;
            last_resync = Instant::now();
        }
        if state.selection_version != previous_selection {
            scroll_offset = 0;
        }
        terminal.draw(|frame| {
            let areas = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(frame.area());
            let header = state.process.as_ref().map_or_else(
                || "Waiting for a matching process...".to_string(),
                |p| {
                    format!(
                        "{} | {} | {}{}",
                        p.history_id.clone().unwrap_or_else(|| p.pm_id.to_string()),
                        p.name,
                        p.status,
                        p.exit_code
                            .map_or(String::new(), |code| format!(" | exit {code}"))
                    )
                },
            );
            frame.render_widget(
                Paragraph::new(header).block(
                    Block::default()
                        .title(" Latest process ")
                        .borders(Borders::ALL),
                ),
                areas[0],
            );
            let visible = areas[1].height.saturating_sub(2) as usize;
            scroll_offset = scroll_offset.min(state.lines.len().saturating_sub(visible));
            let end = state.lines.len().saturating_sub(scroll_offset);
            let lines: Vec<_> = state
                .lines
                .iter()
                .skip(end.saturating_sub(visible))
                .take(visible)
                .map(|(err, text)| {
                    Line::from(vec![
                        Span::styled(
                            if *err { "err | " } else { "out | " },
                            Style::default().fg(if *err { Color::Red } else { Color::Green }),
                        ),
                        Span::raw(text.as_str()),
                    ])
                })
                .collect();
            frame.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .title(" Latest output ")
                        .borders(Borders::ALL),
                ),
                areas[1],
            );
            frame.render_widget(
                Paragraph::new("Live: latest command | Up/Down/PgUp/PgDn: scroll | End: follow | q / Ctrl-C: exit"),
                areas[2],
            );
        })?;
        while event::poll(Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    return Ok(());
                }
                match key.code {
                    KeyCode::Up => scroll_offset = scroll_offset.saturating_add(1),
                    KeyCode::PageUp => scroll_offset = scroll_offset.saturating_add(10),
                    KeyCode::Down => scroll_offset = scroll_offset.saturating_sub(1),
                    KeyCode::PageDown => scroll_offset = scroll_offset.saturating_sub(10),
                    KeyCode::Home => scroll_offset = state.lines.len(),
                    KeyCode::End => scroll_offset = 0,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::BusProcessRef;
    use serde_json::json;
    use std::io::Write;

    fn options() -> LogOptions {
        LogOptions {
            target: "all".into(),
            lines: 15,
            limit: 5,
            sort_desc: true,
            raw: false,
            json: false,
            format: false,
            err_only: false,
            out_only: false,
            timestamp: None,
            nostream: false,
            highlight: None,
            use_color: false,
        }
    }

    fn log(id: u32, message: &str) -> BusEvent {
        BusEvent {
            event: "log:out".into(),
            at: u64::from(id),
            process: BusProcessRef {
                pm_id: json!(id),
                name: format!("job-{id}"),
                namespace: "jobs".into(),
                rev: None,
            },
            data: json!({"data": message}),
            status: None,
            manually: None,
        }
    }

    #[test]
    fn durable_results_ignore_older_service_events_and_snapshots() {
        let opts = options();
        let mut state = LiveState::new(5);
        let run = LiveProcess {
            history_id: Some("new".into()),
            pm_id: 0,
            name: "job".into(),
            namespace: "jobs".into(),
            status: "succeeded".into(),
            exit_code: Some(0),
            unique_id: "new".into(),
            created_at: 100,
            out_path: String::new(),
            err_path: String::new(),
        };
        state.seed(run.clone(), &opts);
        state.push(false, "retained");
        state.apply(
            &log(999, "old service"),
            &LogSelector::new("all").unwrap(),
            &opts,
        );
        assert_eq!(
            state.process.as_ref().unwrap().history_id.as_deref(),
            Some("new")
        );
        let mut older = run;
        older.history_id = Some("old".into());
        older.unique_id = "old".into();
        older.created_at = 10;
        state.refresh_snapshot(older, &opts, true);
        assert_eq!(state.lines[0].1, "retained");
    }

    #[test]
    fn a_million_sequential_events_keep_only_the_latest_result() {
        let mut state = LiveState::new(5);
        let selector = LogSelector::new("all").unwrap();
        let opts = options();
        let mut event = log(0, "latest result");
        for id in 0..1_000_000u32 {
            event.process.pm_id = json!(id);
            event.at = u64::from(id);
            state.apply(&event, &selector, &opts);
        }
        assert_eq!(state.process.as_ref().unwrap().pm_id, 999_999);
        assert_eq!(state.lines.len(), 1);
        state.apply(&log(0, "older output"), &selector, &opts);
        assert_eq!(state.lines[0].1, "latest result");
        assert!(state.bytes <= MAX_OUTPUT_BYTES);
    }

    #[test]
    fn output_is_bounded_by_lines_and_bytes_and_accepts_utf8() {
        let mut state = LiveState::new(usize::MAX);
        for _ in 0..1000 {
            state.push(false, &"🦀".repeat(10_000));
        }
        assert!(state.bytes <= MAX_OUTPUT_BYTES);
        assert!(state.lines.len() < 20);
        let mut state = LiveState::new(2);
        for line in ["first", "second", "third"] {
            state.push(false, line);
        }
        assert_eq!(
            state
                .lines
                .iter()
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>(),
            vec!["second", "third"]
        );
    }

    #[test]
    fn exit_delete_and_id_reuse_keep_the_latest_result() {
        let mut state = LiveState::new(5);
        let selector = LogSelector::new("all").unwrap();
        let opts = options();
        let mut event = log(10, "finished");
        state.apply(&event, &selector, &opts);
        event.event = "process:event".into();
        event.data = json!({"event":"exit", "process":{"status":"stopped", "exit_code":3}});
        state.apply(&event, &selector, &opts);
        assert_eq!(state.process.as_ref().unwrap().exit_code, Some(3));
        event.data = json!({"event":"delete"});
        state.apply(&event, &selector, &opts);
        assert_eq!(state.lines[0].1, "finished");
        let mut next = log(0, "new");
        next.event = "process:event".into();
        next.data = json!({"event":"online", "process":{"status":"online"}});
        state.apply(&next, &selector, &opts);
        state.apply(&log(0, "new"), &selector, &opts);
        assert_eq!(state.process.as_ref().unwrap().pm_id, 0);
        assert_eq!(state.lines.len(), 1);
        assert_eq!(state.lines[0].1, "new");
    }

    #[test]
    fn snapshot_recovery_never_replaces_a_newer_result_with_an_older_process() {
        let opts = options();
        let mut state = LiveState::new(5);
        state.apply(
            &log(10, "latest result"),
            &LogSelector::new("all").unwrap(),
            &opts,
        );
        let mut older = state.process.clone().unwrap();
        older.pm_id = 1;
        older.created_at = 1;
        state.refresh_snapshot(older, &opts, true);
        assert_eq!(state.process.as_ref().unwrap().pm_id, 10);
        assert_eq!(state.lines[0].1, "latest result");
    }

    #[test]
    fn metadata_refresh_preserves_event_order_and_selection_but_gaps_reload_output() {
        let opts = options();
        let selector = LogSelector::new("all").unwrap();
        let mut state = LiveState::new(5);
        state.apply(&log(1, "first stdout"), &selector, &opts);
        let mut err = log(1, "second stderr");
        err.event = "log:err".into();
        state.apply(&err, &selector, &opts);
        state.apply(&log(1, "third stdout"), &selector, &opts);
        let version = state.selection_version;
        let mut snapshot = state.process.clone().unwrap();
        snapshot.unique_id = "snapshot-id".into();
        snapshot.created_at = 0; // The online event and creation timestamp differ.
        snapshot.status = "stopped".into();
        state.refresh_snapshot(snapshot.clone(), &opts, false);
        assert_eq!(state.selection_version, version);
        assert_eq!(
            state
                .lines
                .iter()
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>(),
            vec!["first stdout", "second stderr", "third stdout"]
        );
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "recovered output").unwrap();
        snapshot.out_path = file.path().to_string_lossy().into_owned();
        state.refresh_snapshot(snapshot, &opts, true);
        assert_eq!(state.selection_version, version);
        assert_eq!(state.lines[0].1, "recovered output");
    }

    #[test]
    fn selectors_and_stream_filters_are_applied() {
        for target in ["all", "7", "job-7", "jobs", "/job-[7-9]/"] {
            assert!(LogSelector::new(target)
                .unwrap()
                .matches(7, "job-7", "jobs"));
        }
        assert!(LogSelector::new("/[/").is_err());
        let mut state = LiveState::new(5);
        let selector = LogSelector::new("jobs").unwrap();
        let mut opts = options();
        opts.err_only = true;
        state.apply(&log(0, "hidden stdout"), &selector, &opts);
        assert!(state.lines.is_empty());
        let mut event = log(0, "shown stderr");
        event.event = "log:err".into();
        state.apply(&event, &selector, &opts);
        assert_eq!(state.lines[0], (true, "shown stderr".into()));
    }

    #[test]
    fn a_gigabyte_log_only_reads_the_bounded_tail() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.as_file_mut().set_len(1024 * 1024 * 1024).unwrap();
        file.as_file_mut().seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"\nfirst\nlast\n").unwrap();
        assert_eq!(
            read_tail(file.path().to_str().unwrap(), 2),
            vec!["first", "last"]
        );
    }
}
