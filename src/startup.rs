use anyhow::{anyhow, Result};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::client::PmrClient;
use crate::config::which_in_path;
use crate::constants::{
    LAUNCHD_TPL, LOGROTATE_TPL, OPENRC_TPL, PREFIX_MSG, PREFIX_MSG_ERR, PREFIX_MSG_WARNING,
    SYSTEMD_ONLINE_TPL, SYSTEMD_TPL, UPSTART_TPL,
};
use crate::paths::PmrPaths;
use crate::process::{Pm2Env, ProcessDescription};
use crate::protocol::RpcRequest;

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
            fs::write(&paths.dump_file_path, "[]\n")?;
            return Ok(0);
        }
    }

    if paths.dump_file_path.exists() {
        let _ = fs::copy(&paths.dump_file_path, &paths.dump_backup_file_path);
    }

    // Sort by pm_id for deterministic dump order
    env_arr.sort_by_key(|e| e.pm_id);
    let json_str = serde_json::to_string_pretty(&env_arr)?;
    if let Err(e) = fs::write(&paths.dump_file_path, json_str) {
        if paths.dump_backup_file_path.exists() {
            let _ = fs::copy(&paths.dump_backup_file_path, &paths.dump_file_path);
        }
        return Err(e.into());
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
    fs::write(&paths.dump_file_path, "[]\n")?;
    println!(
        "{}Successfully cleared dump file {}",
        PREFIX_MSG,
        paths.dump_file_path.display()
    );
    Ok(())
}

pub async fn cmd_resurrect(client: &PmrClient) -> Result<Vec<ProcessDescription>> {
    println!("{}Resurrecting", PREFIX_MSG);
    let raw = if let Ok(s) = fs::read_to_string(&client.paths.dump_file_path) {
        s
    } else if let Ok(s) = fs::read_to_string(&client.paths.dump_backup_file_path) {
        s
    } else {
        println!("{}No processes saved in dump file", PREFIX_MSG_WARNING);
        return client.list().await;
    };

    let dumped: Vec<Pm2Env> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            if let Ok(s) = fs::read_to_string(&client.paths.dump_backup_file_path) {
                serde_json::from_str(&s)?
            } else {
                let _ = fs::remove_file(&client.paths.dump_file_path);
                return Err(anyhow!("Corrupted dump file removed"));
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
        env.status = crate::constants::ONLINE_STATUS.to_string();
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

pub fn cmd_startup(
    paths: &PmrPaths,
    platform: Option<&str>,
    user: Option<&str>,
    hp: Option<&str>,
    wait_ip: bool,
    service_name: Option<&str>,
) -> Result<()> {
    let detected = platform.unwrap_or_else(|| detect_init_system());
    let cur_uid = unsafe { libc::getuid() };
    let target_user = user
        .map(|s| s.to_string())
        .or_else(|| env::var("USER").ok())
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
    let exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("pmr"));
    let exe_dir = exe_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/usr/local/bin"));

    println!("{}Init System found: {}", PREFIX_MSG, detected);

    // On non-root (except launchd User LaunchAgent if writable), print the sudo command just like PM2!
    if cur_uid != 0 && detected != "launchd" {
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
        return Ok(());
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
        .unwrap_or_else(|| format!("pmr-{}", target_user));

    let rendered = tpl
        .replace("%PM2_PATH%", &exe_path.to_string_lossy())
        .replace("%NODE_PATH%", &exe_dir.to_string_lossy())
        .replace("%USER%", &target_user)
        .replace("%HOME_PATH%", &pmr_home.to_string_lossy())
        .replace("%SERVICE_NAME%", &svc_name);

    let dest = match detected {
        "launchd" => {
            let agents_dir = home_dir.join("Library/LaunchAgents");
            let _ = fs::create_dir_all(&agents_dir);
            agents_dir.join(format!("io.keymetrics.pmr.{}.plist", target_user))
        }
        "systemd" => PathBuf::from(format!("/etc/systemd/system/{}.service", svc_name)),
        "openrc" => PathBuf::from(format!("/etc/init.d/{}", svc_name)),
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
            }
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
        }
    }

    Ok(())
}

pub fn cmd_unstartup(
    paths: &PmrPaths,
    platform: Option<&str>,
    user: Option<&str>,
    hp: Option<&str>,
    service_name: Option<&str>,
) -> Result<()> {
    let detected = platform.unwrap_or_else(|| detect_init_system());
    let cur_uid = unsafe { libc::getuid() };
    let target_user = user
        .map(|s| s.to_string())
        .or_else(|| env::var("USER").ok())
        .unwrap_or_else(|| "root".to_string());
    let home_dir = hp
        .map(PathBuf::from)
        .or_else(|| env::var("HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/root"));
    let exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("pmr"));
    let exe_dir = exe_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/usr/local/bin"));

    if cur_uid != 0 && detected != "launchd" {
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
        return Ok(());
    }

    let svc_name = service_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("pmr-{}", target_user));

    let dest = match detected {
        "launchd" => home_dir
            .join("Library/LaunchAgents")
            .join(format!("io.keymetrics.pmr.{}.plist", target_user)),
        "systemd" => PathBuf::from(format!("/etc/systemd/system/{}.service", svc_name)),
        _ => PathBuf::from(format!("/etc/init.d/{}", svc_name)),
    };

    if dest.exists() {
        let _ = fs::remove_file(&dest);
        println!(
            "{}Removed startup script {}",
            PREFIX_MSG,
            dest.display()
        );
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
    let exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("pmr"));
    let exe_dir = exe_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/usr/local/bin"));

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
