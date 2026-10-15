use anyhow::{anyhow, Result};
use serde_json::Value;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::client::PmrClient;
use crate::config::which_in_path;
use crate::constants::{
    LAUNCHD_TPL, LOGROTATE_TPL, OPENRC_TPL, PREFIX_MSG, PREFIX_MSG_ERR, PREFIX_MSG_WARNING,
    STOPPED_STATUS, SYSTEMD_ONLINE_TPL, SYSTEMD_TPL, UPSTART_TPL,
};
use crate::paths::PmrPaths;
use crate::process::{Pm2Env, ProcessDescription};
use crate::protocol::RpcRequest;

pub fn write_atomic_file(target: &Path, content: &str) -> Result<()> {
    let tmp = target.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, target)?;
    Ok(())
}

pub fn dump_processes_to_disk(
    paths: &PmrPaths,
    procs: &[ProcessDescription],
    force: bool,
) -> Result<usize> {
    let mut env_arr: Vec<Pm2Env> = procs
        .iter()
        .filter(|p| !p.pm2_env.pmx_module)
        .map(|p| {
            let mut env = p.pm2_env.clone();
            // Strip instances so resurrect launches 1 process per dumped entry
            env.instances = None;
            env.prev_restart_delay = 0;
            env.sync_flattened_env();
            env
        })
        .collect();

    if env_arr.is_empty() && !force && env::var("FORCE").is_err() {
        if paths.dump_file_path.exists() {
            println!(
                "{}Nothing to save (use `pmr save --force` to clear saved process list)",
                PREFIX_MSG_WARNING
            );
            return Ok(0);
        } else {
            write_atomic_file(&paths.dump_file_path, "[]\n")?;
            return Ok(0);
        }
    }

    if paths.dump_file_path.exists() {
        let _ = fs::copy(&paths.dump_file_path, &paths.dump_backup_file_path);
    }

    // Sort by pm_id for deterministic dump order, then strip pm_id and instances for PM2 compatibility
    env_arr.sort_by_key(|e| e.pm_id);
    let mut cleaned_vals: Vec<Value> = Vec::with_capacity(env_arr.len());
    for env in &env_arr {
        let mut val = serde_json::to_value(env)?;
        if let Some(obj) = val.as_object_mut() {
            obj.remove("pm_id");
            obj.remove("instances");
        }
        cleaned_vals.push(val);
    }
    let json_str = serde_json::to_string_pretty(&cleaned_vals)?;
    if let Err(e) = write_atomic_file(&paths.dump_file_path, &json_str) {
        if paths.dump_backup_file_path.exists() {
            let _ = fs::copy(&paths.dump_backup_file_path, &paths.dump_file_path);
        }
        return Err(e);
    }

    Ok(env_arr.len())
}

pub async fn cmd_dump(client: &PmrClient, force: bool) -> Result<()> {
    println!("{}Saving current process list...", PREFIX_MSG);
    let procs = client.list().await?;
    let count = dump_processes_to_disk(&client.paths, &procs, force)?;
    println!(
        "{}Successfully saved {} process(es) in {}",
        PREFIX_MSG,
        count,
        client.paths.dump_file_path.display()
    );
    Ok(())
}

pub fn cmd_cleardump(paths: &PmrPaths) -> Result<()> {
    write_atomic_file(&paths.dump_file_path, "[]\n")?;
    println!(
        "{}Successfully cleared dump file {}",
        PREFIX_MSG,
        paths.dump_file_path.display()
    );
    Ok(())
}

fn parse_dump_str(raw: &str) -> Result<Vec<Pm2Env>> {
    let val: Value = serde_json::from_str(raw)?;
    let arr = val
        .as_array()
        .ok_or_else(|| anyhow!("Dump file must contain a JSON array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        match Pm2Env::from_pm2_value(item.clone()) {
            Ok(env) => out.push(env),
            Err(e) => {
                eprintln!(
                    "{}Skipping malformed dump entry: {}",
                    PREFIX_MSG_WARNING, e
                );
            }
        }
    }
    Ok(out)
}

fn find_pm2_fallback_dump(paths: &PmrPaths) -> Option<String> {
    if env::var("PMR_HOME").is_ok() || env::var("PM2_HOME").is_ok() {
        return None;
    }
    let home = std::env::var("HOME").ok().map(std::path::PathBuf::from)?;
    let pm2_dump = home.join(".pm2").join("dump.pm2");
    if pm2_dump != paths.dump_file_path && pm2_dump.exists() {
        if let Ok(s) = fs::read_to_string(&pm2_dump) {
            return Some(s);
        }
    }
    None
}

async fn resurrect_installed_modules(client: &PmrClient) {
    let all_conf = crate::config::ModuleConfiguration::get_all_sync(&client.paths);
    let Some(mod_db) = all_conf
        .get(crate::constants::MODULE_CONF_PREFIX)
        .and_then(|v| v.as_object())
    else {
        return;
    };
    let current = client.list().await.unwrap_or_default();
    for mod_name in mod_db.keys() {
        if current.iter().any(|p| &p.name == mod_name) {
            continue;
        }
        let folder_name = mod_name.replace('/', "-");
        let candidate_nm = client
            .paths
            .default_module_path
            .join(&folder_name)
            .join("node_modules")
            .join(mod_name);
        let candidate_direct = client.paths.default_module_path.join(&folder_name);
        let mod_path = if candidate_nm.exists() {
            Some(candidate_nm)
        } else if candidate_direct.join("package.json").exists() {
            Some(candidate_direct)
        } else {
            None
        };
        if let Some(dir) = mod_path {
            let _ = crate::modules::cmd_install_module(
                client,
                &dir.to_string_lossy(),
                &crate::config::StartOptions::default(),
            )
            .await;
        }
    }
}

pub async fn cmd_resurrect(client: &PmrClient) -> Result<Vec<ProcessDescription>> {
    println!("{}Resurrecting", PREFIX_MSG);
    resurrect_installed_modules(client).await;

    let raw = if let Ok(s) = fs::read_to_string(&client.paths.dump_file_path) {
        s
    } else if let Ok(s) = fs::read_to_string(&client.paths.dump_backup_file_path) {
        s
    } else if let Some(s) = find_pm2_fallback_dump(&client.paths) {
        s
    } else {
        println!("{}No processes saved in dump file", PREFIX_MSG_WARNING);
        return client.list().await;
    };

    let dumped: Vec<Pm2Env> = match parse_dump_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            if let Ok(s) = fs::read_to_string(&client.paths.dump_backup_file_path) {
                if let Ok(v) = parse_dump_str(&s) {
                    let _ = fs::copy(&client.paths.dump_backup_file_path, &client.paths.dump_file_path);
                    v
                } else {
                    let corrupt_path = client.paths.dump_file_path.with_extension("pm2.corrupt");
                    let _ = fs::rename(&client.paths.dump_file_path, &corrupt_path);
                    return Err(anyhow!(
                        "Corrupted dump file moved to {}",
                        corrupt_path.display()
                    ));
                }
            } else {
                let corrupt_path = client.paths.dump_file_path.with_extension("pm2.corrupt");
                let _ = fs::rename(&client.paths.dump_file_path, &corrupt_path);
                return Err(anyhow!(
                    "Corrupted dump file moved to {}",
                    corrupt_path.display()
                ));
            }
        }
    };

    let current = client.list().await?;
    for mut env in dumped {
        let already_running = current.iter().any(|p| {
            p.name == env.name && p.pm2_env.node_app_instance == env.node_app_instance
        });
        if already_running {
            println!("{}Process {} already running, skipping", PREFIX_MSG, env.name);
            continue;
        }
        println!("{}Restoring processes {}...", PREFIX_MSG, env.name);
        if env.status == STOPPED_STATUS {
            // Preserve stopped status and historical restart_time on resurrect
            env.autostart = false;
        } else {
            env.status = crate::constants::ONLINE_STATUS.to_string();
            env.autostart = true;
            env.restart_time = 0;
            env.unstable_restarts = 0;
        }
        env.instances = Some(1);
        env.sync_flattened_env();
        let _ = client
            .request(&RpcRequest::Prepare {
                env: Box::new(env),
                instances: 1,
            })
            .await?;
    }

    client.list().await
}

pub fn detect_init_system() -> &'static str {
    let candidates = [
        ("systemctl", "systemd"),
        ("update-rc.d", "upstart"),
        ("chkconfig", "systemv"),
        ("rc-update", "openrc"),
        ("launchctl", "launchd"),
        ("sysrc", "rcd"),
        ("rcctl", "rcd-openbsd"),
        ("svcadm", "smf"),
    ];
    for (bin, name) in candidates {
        if which_in_path(bin).is_some() {
            return name;
        }
    }
    if cfg!(target_os = "macos") {
        "launchd"
    } else {
        "systemd"
    }
}

pub fn normalize_startup_platform(platform: Option<&str>) -> Result<&'static str> {
    let Some(p) = platform else {
        return Ok(detect_init_system());
    };
    match p.trim().to_ascii_lowercase().as_str() {
        "systemd" | "ubuntu" | "centos" | "arch" | "oracle" | "fedora" | "debian" | "rhel" => {
            Ok("systemd")
        }
        "upstart" | "ubuntu14" | "ubuntu12" => Ok("upstart"),
        "launchd" | "darwin" | "macos" | "osx" => Ok("launchd"),
        "openrc" | "gentoo" | "alpine" => Ok("openrc"),
        "rcd" | "freebsd" => Ok("rcd"),
        "rcd-openbsd" | "openbsd" => Ok("rcd-openbsd"),
        "systemv" | "amazon" | "sysv" => Ok("systemv"),
        "smf" | "sunos" | "solaris" => Ok("smf"),
        other => {
            eprintln!("{}Platform {} is not supported", PREFIX_MSG_ERR, other);
            Err(anyhow!("Platform {} is not supported", other))
        }
    }
}

fn build_full_path_for_startup(exe_dir: &Path) -> String {
    let exe_dir_str = exe_dir.to_string_lossy().to_string();
    let cur_path = env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
    if cur_path.split(':').any(|p| p == exe_dir_str) {
        cur_path
    } else {
        format!("{}:{}", cur_path, exe_dir_str)
    }
}

pub fn cmd_startup(
    paths: &PmrPaths,
    platform: Option<&str>,
    user: Option<&str>,
    hp: Option<&str>,
    wait_ip: bool,
    service_name: Option<&str>,
) -> Result<()> {
    let detected = normalize_startup_platform(platform)?;
    let cur_uid = unsafe { libc::getuid() };
    let target_user = user
        .map(|s| s.to_string())
        .or_else(|| env::var("USER").ok())
        .or_else(|| env::var("LOGNAME").ok())
        .unwrap_or_else(|| "root".to_string());
    let home_dir = hp
        .map(PathBuf::from)
        .or_else(|| env::var("HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/root"));
    let pmr_home = if hp.is_some() {
        home_dir.join(".pmr")
    } else {
        paths.pm2_home.clone()
    };
    let exe_path = PmrPaths::resolve_pmr_binary();
    let exe_dir = exe_path
        .parent()
        .unwrap_or_else(|| Path::new("/usr/local/bin"));
    let full_path = build_full_path_for_startup(exe_dir);

    println!("{}Init System found: {}", PREFIX_MSG, detected);

    let can_write_without_root = detected == "launchd" && hp.is_some();
    if cur_uid != 0 && !can_write_without_root {
        println!(
            "{}To setup the Startup Script, copy/paste the following command:",
            PREFIX_MSG
        );
        println!(
            "sudo env PATH=$PATH:{} {} startup {} -u {} --hp {}",
            exe_dir.display(),
            exe_path.display(),
            detected,
            target_user,
            home_dir.display()
        );
        return Err(anyhow!("You have to run this with elevated rights"));
    }

    let tpl = match detected {
        "systemd" if wait_ip => SYSTEMD_ONLINE_TPL,
        "systemd" => SYSTEMD_TPL,
        "launchd" => LAUNCHD_TPL,
        "openrc" => OPENRC_TPL,
        _ => UPSTART_TPL,
    };

    let svc_name = service_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if detected == "launchd" {
                format!("io.keymetrics.pmr.{}", target_user)
            } else {
                format!("pmr-{}", target_user)
            }
        });

    let rendered = tpl
        .replace("%PM2_PATH%", &exe_path.to_string_lossy())
        .replace("%NODE_PATH%", &full_path)
        .replace("%USER%", &target_user)
        .replace("%HOME_PATH%", &pmr_home.to_string_lossy())
        .replace("%SERVICE_NAME%", &svc_name);

    let dest = match detected {
        "launchd" => {
            let agents_dir = home_dir.join("Library/LaunchAgents");
            let _ = fs::create_dir_all(&agents_dir);
            agents_dir.join(format!("{}.plist", svc_name))
        }
        "systemd" => PathBuf::from(format!("/etc/systemd/system/{}.service", svc_name)),
        "rcd" => PathBuf::from(format!("/usr/local/etc/rc.d/{}", svc_name)),
        "rcd-openbsd" => PathBuf::from(format!("/etc/rc.d/{}", svc_name)),
        _ => PathBuf::from(format!("/etc/init.d/{}", svc_name)),
    };

    match fs::write(&dest, &rendered) {
        Ok(()) => {
            println!(
                "{}Successfully wrote startup script to {}",
                PREFIX_MSG,
                dest.display()
            );
            if detected == "systemd" {
                let _ = Command::new("systemctl").args(["daemon-reload"]).status();
                let _ = Command::new("systemctl")
                    .args(["enable", &format!("{}.service", svc_name)])
                    .status();
            } else if detected == "launchd" {
                let _ = Command::new("launchctl")
                    .args(["load", "-w", &dest.to_string_lossy()])
                    .status();
            } else {
                let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(0o755));
                if detected == "openrc" {
                    let _ = Command::new("rc-update")
                        .args(["add", &svc_name, "default"])
                        .status();
                } else if detected == "upstart" {
                    let _ = Command::new("update-rc.d")
                        .args([&svc_name, "defaults"])
                        .status();
                } else if detected == "systemv" {
                    let _ = Command::new("chkconfig")
                        .args(["--add", &svc_name])
                        .status();
                }
            }
            Ok(())
        }
        Err(_) => {
            println!(
                "{}To setup the Startup Script, copy/paste the following command:",
                PREFIX_MSG
            );
            println!(
                "sudo env PATH=$PATH:{} {} startup {} -u {} --hp {}",
                exe_dir.display(),
                exe_path.display(),
                detected,
                target_user,
                home_dir.display()
            );
            Err(anyhow!("You have to run this with elevated rights"))
        }
    }
}

pub fn cmd_unstartup(
    paths: &PmrPaths,
    platform: Option<&str>,
    user: Option<&str>,
    hp: Option<&str>,
    service_name: Option<&str>,
) -> Result<()> {
    let detected = normalize_startup_platform(platform)?;
    let cur_uid = unsafe { libc::getuid() };
    let target_user = user
        .map(|s| s.to_string())
        .or_else(|| env::var("USER").ok())
        .or_else(|| env::var("LOGNAME").ok())
        .unwrap_or_else(|| "root".to_string());
    let home_dir = hp
        .map(PathBuf::from)
        .or_else(|| env::var("HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/root"));
    let exe_path = PmrPaths::resolve_pmr_binary();
    let exe_dir = exe_path
        .parent()
        .unwrap_or_else(|| Path::new("/usr/local/bin"));

    let can_write_without_root = detected == "launchd" && hp.is_some();
    if cur_uid != 0 && !can_write_without_root {
        println!(
            "{}To unsetup the Startup Script, copy/paste the following command:",
            PREFIX_MSG
        );
        println!(
            "sudo env PATH=$PATH:{} {} unstartup {} -u {} --hp {}",
            exe_dir.display(),
            exe_path.display(),
            detected,
            target_user,
            home_dir.display()
        );
        let _ = paths;
        return Err(anyhow!("You have to run this with elevated rights"));
    }

    let svc_name = service_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if detected == "launchd" {
                format!("io.keymetrics.pmr.{}", target_user)
            } else {
                format!("pmr-{}", target_user)
            }
        });

    let dest = match detected {
        "launchd" => home_dir
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", svc_name)),
        "systemd" => PathBuf::from(format!("/etc/systemd/system/{}.service", svc_name)),
        "rcd" => PathBuf::from(format!("/usr/local/etc/rc.d/{}", svc_name)),
        "rcd-openbsd" => PathBuf::from(format!("/etc/rc.d/{}", svc_name)),
        _ => PathBuf::from(format!("/etc/init.d/{}", svc_name)),
    };

    if detected == "systemd" {
        let _ = Command::new("systemctl").args(["stop", &svc_name]).status();
        let _ = Command::new("systemctl")
            .args(["disable", &svc_name])
            .status();
    } else if detected == "launchd" {
        let _ = Command::new("launchctl")
            .args(["remove", &svc_name])
            .status();
    }

    if dest.exists() {
        let _ = fs::remove_file(&dest);
        println!("{}Removed startup script {}", PREFIX_MSG, dest.display());
    } else {
        println!(
            "{}No startup script found at {}",
            PREFIX_MSG,
            dest.display()
        );
    }
    Ok(())
}

pub fn cmd_logrotate(paths: &PmrPaths, user: Option<&str>) -> Result<()> {
    let cur_uid = unsafe { libc::getuid() };
    let target_user = user
        .map(|s| s.to_string())
        .or_else(|| env::var("USER").ok())
        .unwrap_or_else(|| "root".to_string());
    let exe_path = PmrPaths::resolve_pmr_binary();
    let exe_dir = exe_path
        .parent()
        .unwrap_or_else(|| Path::new("/usr/local/bin"));

    if cur_uid != 0 {
        eprintln!(
            "{}You have to run this command as root. Execute the following command:",
            PREFIX_MSG
        );
        eprintln!(
            "{}      sudo env PATH=$PATH:{} {} logrotate -u {}",
            PREFIX_MSG,
            exe_dir.display(),
            exe_path.display(),
            target_user
        );
        return Err(anyhow!("You have to run this with elevated rights"));
    }

    let dir = PathBuf::from("/etc/logrotate.d");
    if !dir.exists() {
        eprintln!(
            "{}/etc/logrotate.d does not exist, cannot copy default configuration.",
            PREFIX_MSG_ERR
        );
        return Err(anyhow!("/etc/logrotate.d does not exist"));
    }

    let script = LOGROTATE_TPL
        .replace("%HOME_PATH%", &paths.pm2_home.to_string_lossy())
        .replace("%USER%", &target_user);

    let dest = dir.join(format!("pmr-{}", target_user));
    fs::write(&dest, script)?;
    println!(
        "{}Logrotate configuration added to {}",
        PREFIX_MSG,
        dest.display()
    );
    Ok(())
}
