use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
use regex::Regex;
use std::cmp::Ordering;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_width::UnicodeWidthStr;

use crate::client::PmrClient;
use crate::god::convert_dayjs_to_chrono;
use crate::process::ProcessDescription;
use crate::protocol::RpcRequest;

/// Format byte sizes identically to `UXHelpers.bytesToSize(bytes, precision)`.
pub fn bytes_to_size(bytes: u64, precision: usize) -> String {
    if bytes == 0 {
        return "0b".to_string();
    }
    let k = 1024.0_f64;
    let b = bytes as f64;
    let sizes = ["b", "kb", "mb", "gb", "tb"];
    let i = (b.ln() / k.ln()).floor() as usize;
    let idx = i.min(sizes.len() - 1);
    let val = b / k.powi(idx as i32);
    format!("{:.prec$}{}", val, sizes[idx], prec = precision)
}

/// Format relative uptime identically to `UXHelpers.timeSince(date)`.
pub fn time_since(uptime_ms: u64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    if uptime_ms == 0 || uptime_ms > now_ms {
        return "0s".to_string();
    }
    let seconds = (now_ms - uptime_ms) / 1000;
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
    let default_field = if mini_list { "id" } else { "name" };
    let (field, desc) = match sort_spec {
        Some(spec) => {
            let mut parts = spec.split(':');
            let f = parts.next().unwrap_or(default_field).to_ascii_lowercase();
            let d = parts
                .next()
                .map(|o| o.eq_ignore_ascii_case("desc"))
                .unwrap_or(false);
            (f, d)
        }
        None => (default_field.to_string(), false),
    };

    sorted.sort_by(|a, b| {
        let ord = match field.as_str() {
            "name" => a.name.cmp(&b.name).then_with(|| a.pm_id.cmp(&b.pm_id)),
            "namespace" => a
                .pm2_env
                .namespace
                .cmp(&b.pm2_env.namespace)
                .then_with(|| a.pm_id.cmp(&b.pm_id)),
            "pid" => a.pid.cmp(&b.pid).then_with(|| a.pm_id.cmp(&b.pm_id)),
            "cpu" => a
                .monit
                .cpu
                .partial_cmp(&b.monit.cpu)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.pm_id.cmp(&b.pm_id)),
            "memory" | "mem" => a
                .monit
                .memory
                .cmp(&b.monit.memory)
                .then_with(|| a.pm_id.cmp(&b.pm_id)),
            "uptime" => a
                .pm2_env
                .pm_uptime
                .cmp(&b.pm2_env.pm_uptime)
                .then_with(|| a.pm_id.cmp(&b.pm_id)),
            "restart" | "restarts" => a
                .pm2_env
                .restart_time
                .cmp(&b.pm2_env.restart_time)
                .then_with(|| a.pm_id.cmp(&b.pm_id)),
            "status" => a
                .pm2_env
                .status
                .cmp(&b.pm2_env.status)
                .then_with(|| a.pm_id.cmp(&b.pm_id)),
            "user" => a
                .pm2_env
                .username
                .cmp(&b.pm2_env.username)
                .then_with(|| a.pm_id.cmp(&b.pm_id)),
            "id" | "pm_id" | _ => a.pm_id.cmp(&b.pm_id),
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

    if let Some(mod_conf) = env.axm_options.get("module_conf").and_then(|v| v.as_object()) {
        if !mod_conf.is_empty() {
            let mc_header = "Process configuration";
            if use_color {
                out.push_str(&format!("\x1b[1;7m{}\x1b[0m\n", mc_header));
            } else {
                out.push_str(&format!("{}\n", mc_header));
            }
            let mc_rows: Vec<(String, String)> = mod_conf
                .iter()
                .map(|(k, v)| {
                    let s = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string());
                    (k.clone(), s)
                })
                .collect();
            out.push_str(&render_kv_table(&mc_rows, use_color));
            out.push('\n');
        }
    }

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
            "\n Trigger via: pmr trigger {} <action_name>\n\n",
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

    // Divergent env variables section
    let mut diff_rows = Vec::new();
    for (k, v) in &env.env {
        if let Ok(cur_v) = std::env::var(k) {
            if &cur_v != v {
                diff_rows.push((k.clone(), v.clone()));
            }
        }
    }
    let env_header = " Divergent env variables from local env ";
    if use_color {
        out.push_str(&format!("\x1b[1;7m{}\x1b[0m\n", env_header));
    } else {
        out.push_str(&format!("{}\n", env_header));
    }
    if !diff_rows.is_empty() {
        out.push_str(&render_kv_table(&diff_rows, use_color));
        out.push('\n');
    }
    out.push_str(&format!(
        "\n To see all env variables:\n $ pmr env {}\n",
        env.pm_id
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
    pub use_color: bool,
}

fn format_timestamp_prefix(ts_opt: &Option<String>, use_color: bool) -> String {
    let Some(ref tf) = ts_opt else {
        return String::new();
    };
    let chrono_fmt = if tf.is_empty() {
        "%Y-%m-%d-%H:%M:%S".to_string()
    } else {
        convert_dayjs_to_chrono(tf)
    };
    let rendered = Local::now().format(&chrono_fmt).to_string();
    if use_color {
        format!("\x1b[2;90m{} \x1b[0m", rendered)
    } else {
        format!("{} ", rendered)
    }
}

fn highlight_str(line: &str, highlight: Option<&str>, use_color: bool) -> String {
    match highlight {
        Some(hl) if !hl.is_empty() && use_color => {
            line.replace(hl, &format!("\x1b[100m{}\x1b[0m", hl))
        }
        _ => line.to_string(),
    }
}

pub async fn run_logs(client: &PmrClient, opts: LogOptions) -> Result<()> {
    let list = client.list().await?;
    let target = if opts.target.is_empty() {
        "all"
    } else {
        opts.target.as_str()
    };

    // Validate regex upfront if target is /.../
    let compiled_regex: Option<Regex> =
        if target.starts_with('/') && target.ends_with('/') && target.len() > 2 {
            let pat = &target[1..target.len() - 1];
            Some(Regex::new(pat).map_err(|e| anyhow!("Invalid regex pattern '{}': {}", pat, e))?)
        } else {
            None
        };

    if opts.lines > 0 {
        if !opts.raw && !opts.json && !opts.format {
            if opts.use_color {
                println!(
                    "\x1b[1;90m[TAILING] Tailing last {} lines for [{}] process{} (change the value with --lines option)\x1b[0m",
                    opts.lines,
                    target,
                    if target == "all" { "es" } else { "" }
                );
            } else {
                println!(
                    "[TAILING] Tailing last {} lines for [{}] process{} (change the value with --lines option)",
                    opts.lines,
                    target,
                    if target == "all" { "es" } else { "" }
                );
            }
        }

        if !opts.raw
            && !opts.json
            && !opts.format
            && (target == "all" || target == "PM2" || target == "PMR")
            && !opts.err_only
            && !opts.out_only
        {
            tail_file(
                &client.paths.pm2_log_file_path,
                "PM2",
                "PM2",
                "PM2",
                opts.lines,
                &opts,
            );
        }

        let mut seen = Vec::new();
        for proc in &list {
            let matched = matches_log_target(proc, target, compiled_regex.as_ref());
            if !matched {
                continue;
            }
            let label = format!("{}|{}", proc.pm_id, proc.name);
            let id_str = proc.pm_id.to_string();
            if !opts.err_only {
                let p = &proc.pm2_env.pm_out_log_path;
                if p != "/dev/null" && !seen.contains(p) {
                    seen.push(p.clone());
                    tail_file(
                        Path::new(p),
                        &label,
                        &proc.name,
                        &id_str,
                        opts.lines,
                        &opts,
                    );
                }
            }
            if !opts.out_only {
                let p = &proc.pm2_env.pm_err_log_path;
                if p != "/dev/null" && !seen.contains(p) {
                    seen.push(p.clone());
                    tail_file_with_type(
                        Path::new(p),
                        &label,
                        &proc.name,
                        &id_str,
                        "err",
                        opts.lines,
                        &opts,
                    );
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
            if let Some(ref re) = compiled_regex {
                if !re.is_match(&ev.process.name) {
                    continue;
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
            let line_hl = highlight_str(line, opts.highlight.as_deref(), opts.use_color);

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
                if ev.event == "log:err" {
                    eprintln!("{}", line_hl);
                } else {
                    println!("{}", line_hl);
                }
            } else {
                let ts_prefix = format_timestamp_prefix(&opts.timestamp, opts.use_color);
                let label = if ev.process.name == "PM2" {
                    "PM2".to_string()
                } else {
                    format!("{}|{}", pm_id_str, ev.process.name)
                };
                if opts.use_color {
                    let color = match ev.event.as_str() {
                        "log:err" => "\x1b[31m",
                        "log:PM2" => "\x1b[34m",
                        _ => "\x1b[32m",
                    };
                    println!("{}{}{:<10} |\x1b[0m {}", ts_prefix, color, label, line_hl);
                } else {
                    println!("{}{:<10} | {}", ts_prefix, label, line_hl);
                }
            }
        }
    }

    Ok(())
}

fn matches_log_target(
    proc: &ProcessDescription,
    target: &str,
    compiled_regex: Option<&Regex>,
) -> bool {
    if target == "all" {
        return true;
    }
    if let Some(re) = compiled_regex {
        return re.is_match(&proc.name);
    }
    proc.name == target
        || proc.pm_id.to_string() == target
        || proc.pm2_env.namespace == target
}

fn tail_file(
    path: &Path,
    label: &str,
    app_name: &str,
    pm_id_str: &str,
    lines_count: usize,
    opts: &LogOptions,
) {
    let stype = if label == "PM2" { "PM2" } else { "out" };
    tail_file_with_type(path, label, app_name, pm_id_str, stype, lines_count, opts);
}

fn tail_file_with_type(
    path: &Path,
    label: &str,
    app_name: &str,
    pm_id_str: &str,
    stream_type: &str,
    lines_count: usize,
    opts: &LogOptions,
) {
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    let content = String::from_utf8_lossy(&bytes);
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(lines_count);
    let tail_slice = &all_lines[start..];

    if !opts.raw && !opts.json && !opts.format {
        if opts.use_color {
            println!(
                "\x1b[90m{} last {} lines:\x1b[0m",
                path.display(),
                lines_count
            );
        } else {
            println!("{} last {} lines:", path.display(), lines_count);
        }
    }

    let color = match stream_type {
        "err" => "\x1b[31m",
        "PM2" => "\x1b[34m",
        _ => "\x1b[32m",
    };

    for &line in tail_slice {
        let rendered = highlight_str(line, opts.highlight.as_deref(), opts.use_color);
        if opts.json && !opts.nostream {
            let pid_val = pm_id_str
                .parse::<u32>()
                .map(|n| serde_json::json!(n))
                .unwrap_or_else(|_| serde_json::json!(pm_id_str));
            let obj = serde_json::json!({
                "message": line,
                "timestamp": Utc::now().to_rfc3339(),
                "type": if stream_type == "err" { "err" } else { "out" },
                "process_id": pid_val,
                "app_name": app_name,
            });
            println!("{}", obj);
        } else if opts.format && !opts.nostream {
            let ts = Local::now().format("%Y-%m-%d-%H:%M:%S%z");
            let ltype = if stream_type == "err" { "error" } else { "out" };
            println!(
                "timestamp={} app={} id={} type={} message={}",
                ts, app_name, pm_id_str, ltype, rendered
            );
        } else if opts.raw {
            println!("{}", rendered);
        } else {
            let ts_prefix = format_timestamp_prefix(&opts.timestamp, opts.use_color);
            if opts.use_color {
                println!("{}{}{:<10} |\x1b[0m {}", ts_prefix, color, label, rendered);
            } else {
                println!("{}{:<10} | {}", ts_prefix, label, rendered);
            }
        }
    }
    if !opts.raw && (!opts.json || opts.nostream) && (!opts.format || opts.nostream) {
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
        if !resp.data.is_null() {
            let d = &resp.data;
            println!("{}", title("Daemon"));
            println!(
                "{}",
                field(
                    "pm2/pmr version",
                    d.get("pm2_version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("N/A")
                )
            );
            println!(
                "{}",
                field(
                    "node_path",
                    d.get("node_path").and_then(|v| v.as_str()).unwrap_or("N/A")
                )
            );
            println!(
                "{}",
                field(
                    "user",
                    d.get("user").and_then(|v| v.as_str()).unwrap_or("N/A")
                )
            );
            println!(
                "{}",
                field(
                    "uid",
                    &d.get("uid")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        .to_string()
                )
            );
            println!(
                "{}",
                field(
                    "gid",
                    &d.get("gid")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        .to_string()
                )
            );
            println!("{}", sep);
        }
    }

    if let Ok(resp) = client.request(&RpcRequest::GetSystemData).await {
        if !resp.data.is_null() {
            let d = &resp.data;
            println!("{}", title("System info"));
            if let Some(sys) = d.get("system") {
                println!(
                    "{}",
                    field(
                        "hostname",
                        sys.get("hostname")
                            .and_then(|v| v.as_str())
                            .unwrap_or("N/A")
                    )
                );
                println!(
                    "{}",
                    field(
                        "os",
                        &format!(
                            "{} {}",
                            sys.get("os_name").and_then(|v| v.as_str()).unwrap_or(""),
                            sys.get("os_version").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    )
                );
                println!(
                    "{}",
                    field(
                        "uptime",
                        &format!(
                            "{}s",
                            sys.get("uptime").and_then(|v| v.as_u64()).unwrap_or(0)
                        )
                    )
                );
            }
            if let Some(cpu) = d.get("cpu") {
                println!(
                    "{}",
                    field(
                        "cpus",
                        &format!(
                            "{}x {}",
                            cpu.get("cores").and_then(|v| v.as_u64()).unwrap_or(0),
                            cpu.get("brand").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    )
                );
            }
            if let Some(mem) = d.get("memory") {
                let total = mem.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let avail = mem.get("available").and_then(|v| v.as_u64()).unwrap_or(0);
                println!("{}", field("totalmem", &bytes_to_size(total, 1)));
                println!("{}", field("freemem", &bytes_to_size(avail, 1)));
            }
            println!("{}", sep);
        }
    }

    let procs = client.list().await.unwrap_or_default();
    println!("{}", title(&format!("Process list ({})", procs.len())));
    println!("{}", render_process_list(&procs, false, None, false));
    println!("```");
    Ok(())
}

pub fn render_tree(val: &serde_json::Value, prefix: &str, is_root: bool) -> String {
    let mut out = String::new();
    match val {
        serde_json::Value::Object(map) => {
            if is_root {
                out.push_str("├─ system_info\n");
            }
            let len = map.len();
            for (idx, (k, v)) in map.iter().enumerate() {
                let is_last = idx + 1 == len;
                let branch = if is_last { "└─ " } else { "├─ " };
                let next_prefix = if is_last {
                    format!("{}   ", prefix)
                } else {
                    format!("{}│  ", prefix)
                };
                match v {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        out.push_str(&format!("{}{}{}\n", prefix, branch, k));
                        out.push_str(&render_tree(v, &next_prefix, false));
                    }
                    _ => {
                        let s = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string());
                        out.push_str(&format!("{}{}{}: {}\n", prefix, branch, k, s));
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            let len = arr.len();
            for (idx, item) in arr.iter().enumerate() {
                let is_last = idx + 1 == len;
                let branch = if is_last { "└─ " } else { "├─ " };
                let next_prefix = if is_last {
                    format!("{}   ", prefix)
                } else {
                    format!("{}│  ", prefix)
                };
                match item {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        out.push_str(&format!("{}{}[{}]\n", prefix, branch, idx));
                        out.push_str(&render_tree(item, &next_prefix, false));
                    }
                    _ => {
                        let s = item.as_str().map(|s| s.to_string()).unwrap_or_else(|| item.to_string());
                        out.push_str(&format!("{}{}{}\n", prefix, branch, s));
                    }
                }
            }
        }
        _ => {
            let s = val.as_str().map(|s| s.to_string()).unwrap_or_else(|| val.to_string());
            out.push_str(&format!("{}{}\n", prefix, s));
        }
    }
    out
}
