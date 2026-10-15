use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use regex::Regex;
use serde_json::Value;
use std::cmp::Ordering;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use unicode_width::UnicodeWidthStr;

use crate::client::PmrClient;
use crate::constants::PMR_VERSION;
use crate::god::now_ms;
use crate::process::ProcessDescription;
use crate::protocol::RpcRequest;

pub fn bytes_to_size(bytes: u64, precision: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;

    let b = bytes as f64;
    if b < KB {
        format!("{}b", bytes)
    } else if b < MB {
        format!("{:.prec$}kb", b / KB, prec = precision)
    } else if b < GB {
        format!("{:.prec$}mb", b / MB, prec = precision)
    } else if b < TB {
        format!("{:.prec$}gb", b / GB, prec = precision)
    } else {
        format!("{:.prec$}tb", b / TB, prec = precision)
    }
}

pub fn time_since(epoch_ms: u64) -> String {
    let now = now_ms();
    let seconds = now.saturating_sub(epoch_ms) / 1000;

    let years = seconds / 31_536_000;
    if years > 1 {
        return format!("{}Y", years);
    }
    let months = seconds / 2_592_000;
    if months > 1 {
        return format!("{}M", months);
    }
    let days = seconds / 86_400;
    if days > 1 {
        return format!("{}D", days);
    }
    let hours = seconds / 3_600;
    if hours > 1 {
        return format!("{}h", hours);
    }
    let minutes = seconds / 60;
    if minutes > 1 {
        return format!("{}m", minutes);
    }
    format!("{}s", seconds)
}

pub fn color_status(status: &str, use_color: bool) -> String {
    if !use_color {
        return status.to_string();
    }
    match status {
        "online" | "running" => "\x1b[1;32monline\x1b[0m".to_string(),
        "restarting" => "\x1b[1;33mrestart\x1b[0m".to_string(),
        "waiting restart" => "\x1b[1;33mwaiting restart\x1b[0m".to_string(),
        "created" => "\x1b[1;33mcreated\x1b[0m".to_string(),
        "launching" => "\x1b[1;34mlaunching\x1b[0m".to_string(),
        other => format!("\x1b[1;31m{}\x1b[0m", other),
    }
}

/// Strip ANSI escape codes to measure visible display width.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for ch in chars.by_ref() {
                    if ('@'..='~').contains(&ch) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn visible_width(s: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(s).as_str())
}

fn pad_cell(s: &str, width: usize) -> String {
    let vis = visible_width(s);
    if vis > width {
        let plain = strip_ansi(s);
        let mut truncated = String::new();
        let mut cur_w = 0;
        for ch in plain.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
            if cur_w + cw + 1 > width {
                truncated.push('…');
                cur_w += 1;
                break;
            }
            truncated.push(ch);
            cur_w += cw;
        }
        let rem = width.saturating_sub(cur_w);
        format!(" {}{} ", truncated, " ".repeat(rem))
    } else {
        let pad = width - vis;
        format!(" {}{} ", s, " ".repeat(pad))
    }
}

pub fn render_table(headers: &[&str], rows: &[Vec<String>], use_color: bool) -> String {
    let col_count = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| visible_width(h).max(2)).collect();

    for row in rows {
        for (i, cell) in row.iter().enumerate().take(col_count) {
            let w = visible_width(cell);
            if w > widths[i] {
                widths[i] = w;
            }
        }
    }

    let border_start = if use_color { "\x1b[90m" } else { "" };
    let border_end = if use_color { "\x1b[0m" } else { "" };
    let head_start = if use_color { "\x1b[1;36m" } else { "" };
    let head_end = if use_color { "\x1b[0m" } else { "" };

    let mut out = String::new();

    // Top border: ┌────┬────┐
    out.push_str(border_start);
    out.push('┌');
    for (i, &w) in widths.iter().enumerate() {
        out.push_str(&"─".repeat(w + 2));
        out.push(if i + 1 == col_count { '┐' } else { '┬' });
    }
    out.push_str(border_end);
    out.push('\n');

    // Header row
    out.push_str(border_start);
    out.push('│');
    out.push_str(border_end);
    for (i, &h) in headers.iter().enumerate() {
        let colored_h = format!("{}{}{}", head_start, h, head_end);
        out.push_str(&pad_cell(&colored_h, widths[i]));
        out.push_str(border_start);
        out.push('│');
        out.push_str(border_end);
    }
    out.push('\n');

    // Header separator: ├────┼────┤
    if !rows.is_empty() {
        out.push_str(border_start);
        out.push('├');
        for (i, &w) in widths.iter().enumerate() {
            out.push_str(&"─".repeat(w + 2));
            out.push(if i + 1 == col_count { '┤' } else { '┼' });
        }
        out.push_str(border_end);
        out.push('\n');
    }

    // Data rows (compact: no separator between data rows, matching cli-tableau compact: true)
    for row in rows {
        out.push_str(border_start);
        out.push('│');
        out.push_str(border_end);
        for (i, &w) in widths.iter().enumerate() {
            let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
            out.push_str(&pad_cell(cell, w));
            out.push_str(border_start);
            out.push('│');
            out.push_str(border_end);
        }
        out.push('\n');
    }

    // Bottom border: └────┴────┘
    out.push_str(border_start);
    out.push('└');
    for (i, &w) in widths.iter().enumerate() {
        out.push_str(&"─".repeat(w + 2));
        out.push(if i + 1 == col_count { '┘' } else { '┴' });
    }
    out.push_str(border_end);

    out
}

pub fn render_kv_table(pairs: &[(String, String)], use_color: bool) -> String {
    let mut k_width = 4;
    let mut v_width = 4;
    for (k, v) in pairs {
        k_width = k_width.max(visible_width(k));
        v_width = v_width.max(visible_width(v));
    }

    let border_start = if use_color { "\x1b[90m" } else { "" };
    let border_end = if use_color { "\x1b[0m" } else { "" };
    let key_start = if use_color { "\x1b[1;36m" } else { "" };
    let key_end = if use_color { "\x1b[0m" } else { "" };

    let mut out = String::new();
    out.push_str(border_start);
    out.push('┌');
    out.push_str(&"─".repeat(k_width + 2));
    out.push('┬');
    out.push_str(&"─".repeat(v_width + 2));
    out.push('┐');
    out.push_str(border_end);
    out.push('\n');

    for (k, v) in pairs {
        let colored_k = format!("{}{}{}", key_start, k, key_end);
        out.push_str(border_start);
        out.push('│');
        out.push_str(border_end);
        out.push_str(&pad_cell(&colored_k, k_width));
        out.push_str(border_start);
        out.push('│');
        out.push_str(border_end);
        out.push_str(&pad_cell(v, v_width));
        out.push_str(border_start);
        out.push('│');
        out.push_str(border_end);
        out.push('\n');
    }

    out.push_str(border_start);
    out.push('└');
    out.push_str(&"─".repeat(k_width + 2));
    out.push('┴');
    out.push_str(&"─".repeat(v_width + 2));
    out.push('┘');
    out.push_str(border_end);
    out
}

pub fn render_process_list(
    list: &[ProcessDescription],
    mini_list: bool,
    sort_spec: Option<&str>,
    use_color: bool,
) -> String {
    let mut sorted = list.to_vec();
    let (field, desc) = match sort_spec {
        Some(spec) => {
            let mut parts = spec.split(':');
            let f = parts.next().unwrap_or("id").to_ascii_lowercase();
            let d = parts
                .next()
                .map(|o| o.eq_ignore_ascii_case("desc"))
                .unwrap_or(false);
            (f, d)
        }
        None => ("id".to_string(), false),
    };

    sorted.sort_by(|a, b| {
        let ord = match field.as_str() {
            "name" => a.name.cmp(&b.name).then_with(|| a.pm_id.cmp(&b.pm_id)),
            "namespace" => a
                .pm2_env
                .namespace
                .cmp(&b.pm2_env.namespace)
                .then_with(|| a.pm_id.cmp(&b.pm_id)),
            "pid" => a.pid.cmp(&b.pid),
            "cpu" => a
                .monit
                .cpu
                .partial_cmp(&b.monit.cpu)
                .unwrap_or(Ordering::Equal),
            "memory" | "mem" => a.monit.memory.cmp(&b.monit.memory),
            "uptime" => a.pm2_env.pm_uptime.cmp(&b.pm2_env.pm_uptime),
            "status" => a.pm2_env.status.cmp(&b.pm2_env.status),
            _ => a.pm_id.cmp(&b.pm_id),
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });

    if mini_list {
        let mut out = String::new();
        for l in &sorted {
            let mode = l
                .pm2_env
                .exec_mode
                .strip_suffix("_mode")
                .unwrap_or(&l.pm2_env.exec_mode);
            let uptime = if l.pm2_env.status == "online" && l.pm2_env.pm_uptime > 0 {
                time_since(l.pm2_env.pm_uptime)
            } else {
                "0".to_string()
            };
            out.push_str(&format!(
                "+--- {}\nnamespace : {}\nversion : {}\npid : {}\npm2 id : {}\nstatus : {}\nmode : {}\nrestarted : {}\nuptime : {}\nmemory usage : {}\nerror log : {}\nwatching : {}\nPID file : {}\n\n",
                l.name,
                l.pm2_env.namespace,
                l.pm2_env.version,
                l.pid,
                l.pm_id,
                l.pm2_env.status,
                mode,
                l.pm2_env.restart_time,
                uptime,
                bytes_to_size(l.monit.memory, 1),
                l.pm2_env.pm_err_log_path,
                if l.pm2_env.watch { "yes" } else { "no" },
                l.pm2_env.pm_pid_path
            ));
        }
        return out;
    }

    let headers = [
        "id",
        "name",
        "namespace",
        "version",
        "mode",
        "pid",
        "uptime",
        "↺",
        "status",
        "cpu",
        "mem",
        "user",
        "watching",
    ];

    let mut app_rows = Vec::new();
    let mut mod_rows = Vec::new();

    for l in &sorted {
        let id_str = if use_color {
            format!("\x1b[1;36m{}\x1b[0m", l.pm_id)
        } else {
            l.pm_id.to_string()
        };
        let mut disp_name = l.name.clone();
        if l.pm2_env._km_monitored {
            disp_name = if use_color {
                format!("\x1b[1;32m◉\x1b[0m {}", disp_name)
            } else {
                format!("◉ {}", disp_name)
            };
        }
        let mode_str = if l.pm2_env.exec_mode == "cluster_mode" {
            if use_color {
                "\x1b[1;34mcluster\x1b[0m".to_string()
            } else {
                "cluster".to_string()
            }
        } else if use_color {
            "\x1b[1;7mfork\x1b[0m".to_string()
        } else {
            "fork".to_string()
        };
        let uptime_str = if l.pm2_env.status == "online" && l.pm2_env.pm_uptime > 0 {
            time_since(l.pm2_env.pm_uptime)
        } else {
            "0".to_string()
        };
        let status_str = color_status(&l.pm2_env.status, use_color);
        let cpu_str = format!("{}%", l.monit.cpu);
        let mem_str = bytes_to_size(l.monit.memory, 1);
        let user_str = if use_color {
            format!("\x1b[1m{}\x1b[0m", l.pm2_env.username)
        } else {
            l.pm2_env.username.clone()
        };
        let watch_str = if l.pm2_env.watch {
            if use_color {
                "\x1b[1;32menabled\x1b[0m".to_string()
            } else {
                "enabled".to_string()
            }
        } else if use_color {
            "\x1b[90mdisabled\x1b[0m".to_string()
        } else {
            "disabled".to_string()
        };

        if l.pm2_env.pmx_module {
            mod_rows.push(vec![
                id_str,
                disp_name,
                l.pm2_env.version.clone(),
                l.pid.to_string(),
                status_str,
                l.pm2_env.restart_time.to_string(),
                cpu_str,
                mem_str,
                user_str,
            ]);
        } else {
            app_rows.push(vec![
                id_str,
                disp_name,
                l.pm2_env.namespace.clone(),
                l.pm2_env.version.clone(),
                mode_str,
                l.pid.to_string(),
                uptime_str,
                l.pm2_env.restart_time.to_string(),
                status_str,
                cpu_str,
                mem_str,
                user_str,
                watch_str,
            ]);
        }
    }

    let mut out = render_table(&headers, &app_rows, use_color);
    if !mod_rows.is_empty() {
        let mod_headers = [
            "id", "module", "version", "pid", "status", "↺", "cpu", "mem", "user",
        ];
        out.push_str("\n\n");
        if use_color {
            out.push_str("\x1b[1mModules\x1b[0m\n");
        } else {
            out.push_str("Modules\n");
        }
        out.push_str(&render_table(&mod_headers, &mod_rows, use_color));
    }
    out
}

pub fn render_describe(proc: &ProcessDescription, use_color: bool) -> String {
    let env = &proc.pm2_env;
    let mut out = String::new();

    let banner = format!(
        " Describing process with id {} - name {} ",
        env.pm_id, env.name
    );
    if use_color {
        out.push_str(&format!("\x1b[1;7m{}\x1b[0m\n", banner));
    } else {
        out.push_str(&format!("{}\n", banner));
    }

    let uptime = if env.status == "online" && env.pm_uptime > 0 {
        time_since(env.pm_uptime)
    } else {
        "0".to_string()
    };
    let created_at = env
        .created_at
        .and_then(|ms| DateTime::<Utc>::from_timestamp_millis(ms as i64))
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "N/A".to_string());

    let mut rows = vec![
        ("status".to_string(), color_status(&env.status, use_color)),
        ("name".to_string(), env.name.clone()),
        ("namespace".to_string(), env.namespace.clone()),
        ("version".to_string(), env.version.clone()),
        ("restarts".to_string(), env.restart_time.to_string()),
    ];
    if let Some(ref cron) = env.cron_restart {
        rows.push(("cron restart".to_string(), cron.clone()));
    }
    rows.push(("uptime".to_string(), uptime));
    if let Some(ref log_p) = env.pm_log_path {
        rows.push(("entire log path".to_string(), log_p.clone()));
    }
    rows.extend(vec![
        ("script path".to_string(), env.pm_exec_path.clone()),
        (
            "script args".to_string(),
            if env.args.is_empty() {
                "N/A".to_string()
            } else {
                env.args.join(" ")
            },
        ),
        ("error log path".to_string(), env.pm_err_log_path.clone()),
        ("out log path".to_string(), env.pm_out_log_path.clone()),
        ("pid path".to_string(), env.pm_pid_path.clone()),
        ("interpreter".to_string(), env.exec_interpreter.clone()),
        (
            "interpreter args".to_string(),
            if env.node_args.is_empty() {
                "N/A".to_string()
            } else {
                env.node_args.join(" ")
            },
        ),
        ("script id".to_string(), env.pm_id.to_string()),
        ("exec cwd".to_string(), env.pm_cwd.clone()),
        ("exec mode".to_string(), env.exec_mode.clone()),
        (
            "node.js version".to_string(),
            env.node_version.clone().unwrap_or_else(|| "N/A".to_string()),
        ),
        (
            "node env".to_string(),
            env.env
                .get("NODE_ENV")
                .cloned()
                .unwrap_or_else(|| "N/A".to_string()),
        ),
        (
            "watch & reload".to_string(),
            if env.watch {
                if use_color {
                    "\x1b[1;32m✔\x1b[0m".to_string()
                } else {
                    "✔".to_string()
                }
            } else {
                "✘".to_string()
            },
        ),
        (
            "unstable restarts".to_string(),
            env.unstable_restarts.to_string(),
        ),
        ("created at".to_string(), created_at),
    ]);

    out.push_str(&render_kv_table(&rows, use_color));
    out.push('\n');

    if let Some(ref ver) = env.versioning {
        let rev_header = " Revision control metadata ";
        if use_color {
            out.push_str(&format!("\x1b[1;7m{}\x1b[0m\n", rev_header));
        } else {
            out.push_str(&format!("{}\n", rev_header));
        }
        let ver_rows = vec![
            ("revision control".to_string(), ver.vcs_type.clone()),
            ("remote url".to_string(), ver.url.clone()),
            ("repository root".to_string(), ver.repo_path.clone()),
            (
                "last update".to_string(),
                ver.update_time.clone().unwrap_or_else(|| "N/A".to_string()),
            ),
            ("revision".to_string(), ver.revision.clone()),
            (
                "comment".to_string(),
                ver.comment.chars().take(60).collect::<String>(),
            ),
            ("branch".to_string(), ver.branch.clone()),
        ];
        out.push_str(&render_kv_table(&ver_rows, use_color));
        out.push('\n');
    }

    if !env.axm_actions.is_empty() {
        let act_header = " Actions available ";
        if use_color {
            out.push_str(&format!("\x1b[1;7m{}\x1b[0m\n", act_header));
        } else {
            out.push_str(&format!("{}\n", act_header));
        }
        let act_rows: Vec<(String, String)> = env
            .axm_actions
            .iter()
            .map(|a| (a.action_name.clone(), a.action_type.clone()))
            .collect();
        out.push_str(&render_kv_table(&act_rows, use_color));
        out.push_str(&format!(
            "\n Trigger via: pmr trigger {} <action_name>\n",
            env.name
        ));
    }

    if !env.axm_monitor.is_empty() {
        let mon_header = " Code metrics value ";
        if use_color {
            out.push_str(&format!("\x1b[1;7m{}\x1b[0m\n", mon_header));
        } else {
            out.push_str(&format!("{}\n", mon_header));
        }
        let mon_rows: Vec<(String, String)> = env
            .axm_monitor
            .iter()
            .map(|(k, m)| {
                let val_str = m
                    .value
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| m.value.to_string());
                let full = if m.unit.is_empty() {
                    val_str
                } else {
                    format!("{} {}", val_str, m.unit)
                };
                (k.clone(), full)
            })
            .collect();
        out.push_str(&render_kv_table(&mon_rows, use_color));
        out.push('\n');
    }

    // Divergent env variables
    let mut diff_rows = Vec::new();
    for (k, v) in &env.env {
        if env::var(k).ok().as_ref() != Some(v) {
            let truncated: String = v.chars().take(80).collect();
            diff_rows.push((k.clone(), truncated));
        }
    }
    if !diff_rows.is_empty() {
        let env_header = " Divergent env variables from local env ";
        if use_color {
            out.push_str(&format!("\x1b[1;7m{}\x1b[0m\n", env_header));
        } else {
            out.push_str(&format!("{}\n", env_header));
        }
        out.push_str(&render_kv_table(&diff_rows, use_color));
        out.push('\n');
    }

    out.push_str(&format!(
        "\n Add your own code metrics: http://bit.ly/code-metrics\n Use `pmr logs {} [--lines 1000]` to display logs\n Use `pmr env {}` to display environment variables\n Use `pmr monit` to monitor CPU and Memory usage\n",
        env.name, env.pm_id
    ));
    out
}

pub struct LogOptions {
    pub target: String,
    pub lines: usize,
    pub raw: bool,
    pub json: bool,
    pub format: bool,
    pub err_only: bool,
    pub out_only: bool,
    pub timestamp: Option<String>,
    pub nostream: bool,
    pub highlight: Option<String>,
}

pub async fn run_logs(client: &PmrClient, opts: LogOptions) -> Result<()> {
    let list = client.list().await?;
    let target = if opts.target.is_empty() {
        "all"
    } else {
        opts.target.as_str()
    };

    if opts.lines > 0 && !opts.json && !opts.format {
        if !opts.raw {
            println!(
                "\x1b[1;90m[TAILING] Tailing last {} lines for [{}] process{} (change the value with --lines option)\x1b[0m",
                opts.lines,
                target,
                if target == "all" { "es" } else { "" }
            );
        }

        if !opts.raw && (target == "all" || target == "PM2" || target == "PMR") && !opts.err_only && !opts.out_only {
            tail_file(&client.paths.pm2_log_file_path, "PM2", "PM2", opts.lines, opts.raw, opts.highlight.as_deref());
        }

        let mut seen = Vec::new();
        for proc in &list {
            let matched = matches_log_target(proc, target);
            if !matched {
                continue;
            }
            let label = format!("{}|{}", proc.pm_id, proc.name);
            if !opts.err_only {
                let p = &proc.pm2_env.pm_out_log_path;
                if p != "/dev/null" && !seen.contains(p) {
                    seen.push(p.clone());
                    tail_file(Path::new(p), &label, "out", opts.lines, opts.raw, opts.highlight.as_deref());
                }
            }
            if !opts.out_only {
                let p = &proc.pm2_env.pm_err_log_path;
                if p != "/dev/null" && !seen.contains(p) {
                    seen.push(p.clone());
                    tail_file(Path::new(p), &label, "err", opts.lines, opts.raw, opts.highlight.as_deref());
                }
            }
        }
    }

    if opts.nostream {
        return Ok(());
    }

    let mut rx = client.subscribe_bus().await?;
    while let Some(ev) = rx.recv().await {
        if ev.event != "log:out" && ev.event != "log:err" && ev.event != "log:PM2" {
            if opts.json && ev.event == "process:event" {
                println!(
                    "{}",
                    serde_json::json!({
                        "timestamp": Utc::now().to_rfc3339(),
                        "type": "process_event",
                        "app_name": ev.process.name,
                        "data": ev.data,
                    })
                );
            }
            continue;
        }

        if opts.err_only && ev.event != "log:err" {
            continue;
        }
        if opts.out_only && ev.event != "log:out" {
            continue;
        }

        let pm_id_str = ev
            .process
            .pm_id
            .as_u64()
            .map(|n| n.to_string())
            .or_else(|| ev.process.pm_id.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        if target != "all" {
            if target.starts_with('/') && target.ends_with('/') && target.len() > 2 {
                let pat = &target[1..target.len() - 1];
                if let Ok(re) = Regex::new(pat) {
                    if !re.is_match(&ev.process.name) {
                        continue;
                    }
                }
            } else if ev.process.name != target
                && pm_id_str != target
                && ev.process.namespace != target
            {
                continue;
            }
        }

        let data_str = ev
            .data
            .get("data")
            .and_then(|d| d.as_str())
            .unwrap_or("");

        for line in data_str.lines() {
            let line_hl = if let Some(ref hl) = opts.highlight {
                line.replace(hl, &format!("\x1b[100m{}\x1b[0m", hl))
            } else {
                line.to_string()
            };

            if opts.json {
                let obj = serde_json::json!({
                    "message": line,
                    "timestamp": Utc::now().to_rfc3339(),
                    "type": if ev.event == "log:err" { "err" } else { "out" },
                    "process_id": ev.process.pm_id,
                    "app_name": ev.process.name,
                });
                println!("{}", obj);
            } else if opts.format {
                let ts = Local::now().format("%Y-%m-%d-%H:%M:%S%z");
                let ltype = if ev.event == "log:err" { "error" } else { "out" };
                println!(
                    "timestamp={} app={} id={} type={} message={}",
                    ts, ev.process.name, pm_id_str, ltype, line_hl
                );
            } else if opts.raw {
                println!("{}", line_hl);
            } else {
                let ts_prefix = if let Some(ref tf) = opts.timestamp {
                    let fmt = if tf.is_empty() {
                        "%Y-%m-%d-%H:%M:%S"
                    } else {
                        tf.as_str()
                    };
                    format!("\x1b[2;90m{} \x1b[0m", Local::now().format(fmt))
                } else {
                    String::new()
                };
                let label = if ev.process.name == "PM2" {
                    "PM2".to_string()
                } else {
                    format!("{}|{}", pm_id_str, ev.process.name)
                };
                let color = match ev.event.as_str() {
                    "log:err" => "\x1b[31m",
                    "log:PM2" => "\x1b[34m",
                    _ => "\x1b[32m",
                };
                println!("{}{}{:<10} |\x1b[0m {}", ts_prefix, color, label, line_hl);
            }
        }
    }

    Ok(())
}

fn matches_log_target(proc: &ProcessDescription, target: &str) -> bool {
    if target == "all" {
        return true;
    }
    if proc.name == target
        || proc.pm_id.to_string() == target
        || proc.pm2_env.namespace == target
    {
        return true;
    }
    if target.starts_with('/') && target.ends_with('/') && target.len() > 2 {
        let pat = &target[1..target.len() - 1];
        if let Ok(re) = Regex::new(pat) {
            return re.is_match(&proc.name);
        }
    }
    false
}

fn tail_file(
    path: &Path,
    label: &str,
    stream_type: &str,
    lines_count: usize,
    raw: bool,
    highlight: Option<&str>,
) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(lines_count);
    let tail_slice = &all_lines[start..];

    if !raw {
        println!(
            "\x1b[90m{} last {} lines:\x1b[0m",
            path.display(),
            lines_count
        );
    }

    let color = match stream_type {
        "err" => "\x1b[31m",
        "PM2" => "\x1b[34m",
        _ => "\x1b[32m",
    };

    for &line in tail_slice {
        let rendered = if let Some(hl) = highlight {
            line.replace(hl, &format!("\x1b[100m{}\x1b[0m", hl))
        } else {
            line.to_string()
        };
        if raw {
            println!("{}", rendered);
        } else {
            println!("{}{:<10} |\x1b[0m {}", color, label, rendered);
        }
    }
    if !raw {
        println!();
    }
}

pub async fn print_report(client: &PmrClient) -> Result<()> {
    let sep = "=".repeat(79);
    let title = |t: &str| -> String {
        let prefix = format!("--- {} ", t);
        let pad = 79usize.saturating_sub(visible_width(&prefix));
        format!("{}{}", prefix, "-".repeat(pad))
    };
    let field = |k: &str, v: &str| -> String { format!("{:<20} : {}", k, v) };

    println!("\n\n\n```");
    println!("{}", title("PMR / PM2 report"));
    println!("{}", field("Date", &Local::now().to_rfc2822()));
    println!("{}", sep);

    if let Ok(resp) = client.request(&RpcRequest::GetReport).await {
        if resp.ok {
            println!("{}", title("\x1b[1;34mDaemon\x1b[0m"));
            let d = &resp.data;
            println!(
                "{}",
                field(
                    "pmrd version",
                    d.get("pm2_version").and_then(|v| v.as_str()).unwrap_or(PMR_VERSION)
                )
            );
            println!(
                "{}",
                field(
                    "binary path",
                    d.get("node_path").and_then(|v| v.as_str()).unwrap_or("")
                )
            );
            println!(
                "{}",
                field("user", d.get("user").and_then(|v| v.as_str()).unwrap_or(""))
            );
            println!(
                "{}",
                field(
                    "uid",
                    &d.get("uid").and_then(|v| v.as_u64()).unwrap_or(0).to_string()
                )
            );
            println!(
                "{}",
                field(
                    "gid",
                    &d.get("gid").and_then(|v| v.as_u64()).unwrap_or(0).to_string()
                )
            );
            let started = d.get("started_at").and_then(|v| v.as_u64()).unwrap_or(now_ms());
            let uptime_min = now_ms().saturating_sub(started) / 60_000;
            println!("{}", field("uptime", &format!("{}min", uptime_min)));
        }
    }

    println!("{}", sep);
    println!("{}", title("\x1b[1;34mCLI\x1b[0m"));
    println!("{}", field("local pmr", PMR_VERSION));
    println!(
        "{}",
        field(
            "argv",
            &env::args().collect::<Vec<_>>().join(" ")
        )
    );
    println!(
        "{}",
        field("user", &env::var("USER").unwrap_or_else(|_| "root".to_string()))
    );

    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    println!("{}", sep);
    println!("{}", title("\x1b[1;34mSystem info\x1b[0m"));
    println!("{}", field("arch", env::consts::ARCH));
    println!("{}", field("platform", env::consts::OS));
    println!(
        "{}",
        field(
            "cpus nb",
            &sys.cpus().len().to_string()
        )
    );
    println!("{}", field("freemem", &bytes_to_size(sys.available_memory(), 1)));
    println!("{}", field("totalmem", &bytes_to_size(sys.total_memory(), 1)));
    println!("{}", field("home", &client.paths.pm2_home.display().to_string()));

    println!("{}", sep);
    println!("{}", title("\x1b[1;34mPMR list\x1b[0m"));
    if let Ok(list) = client.list().await {
        println!("{}", render_process_list(&list, false, None, true));
    }

    println!("{}", sep);
    println!("{}", title("\x1b[1;34mDaemon logs\x1b[0m"));
    tail_file(&client.paths.pm2_log_file_path, "PM2", "PM2", 20, false, None);
    println!("```\n\n");
    let _ = std::io::stdout().flush();
    Ok(())
}

pub fn render_tree(val: &Value, prefix: &str, is_last: bool) -> String {
    let mut out = String::new();
    match val {
        Value::Object(map) => {
            let len = map.len();
            for (idx, (k, v)) in map.iter().enumerate() {
                let last = idx + 1 == len;
                let branch = if last { "└─ " } else { "├─ " };
                let next_prefix = format!("{}{}", prefix, if last { "   " } else { "│  " });
                if v.is_object() || v.is_array() {
                    out.push_str(&format!("{}{}{}\n", prefix, branch, k));
                    out.push_str(&render_tree(v, &next_prefix, last));
                } else {
                    let s = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string());
                    out.push_str(&format!("{}{}{}: {}\n", prefix, branch, k, s));
                }
            }
        }
        Value::Array(arr) => {
            let len = arr.len();
            for (idx, v) in arr.iter().enumerate() {
                let last = idx + 1 == len;
                let branch = if last { "└─ " } else { "├─ " };
                let next_prefix = format!("{}{}", prefix, if last { "   " } else { "│  " });
                if v.is_object() || v.is_array() {
                    out.push_str(&format!("{}{}[{}]\n", prefix, branch, idx));
                    out.push_str(&render_tree(v, &next_prefix, last));
                } else {
                    out.push_str(&format!("{}{}{}\n", prefix, branch, v));
                }
            }
        }
        other => {
            let _ = is_last;
            out.push_str(&format!("{}{}\n", prefix, other));
        }
    }
    out
}
