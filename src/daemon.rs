use anyhow::Result;
use serde_json::{json, Value};
use std::env;
use std::fs::{self, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use sysinfo::{Disks, Networks, System};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, Mutex, Notify};

use crate::constants::PMR_VERSION;
use crate::god::{
    delete_process_id, duplicate_process_id, handle_node_ipc_message, prepare_and_launch,
    reload_process_id, restart_process_id, scale_application, setup_watcher_for_process,
    spawn_worker_and_cron_loops, start_process_id, stop_process_id, God,
};
use crate::paths::PmrPaths;
use crate::protocol::{BusEvent, RpcRequest, RpcResponse};
use crate::startup::dump_processes_to_disk;

pub async fn run_daemon(paths: PmrPaths) -> Result<()> {
    paths.init_file_structure()?;

    // Acquire exclusive non-blocking flock on daemon.lock before touching sockets
    let lock_path = paths.pm2_home.join("daemon.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    #[cfg(unix)]
    {
        let rc = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            // Another daemon instance already holds the lock for this PMR_HOME
            return Ok(());
        }
    }

    let pid = std::process::id();
    fs::write(&paths.pm2_pid_file_path, pid.to_string())?;

    let _ = fs::remove_file(&paths.daemon_rpc_port);
    let _ = fs::remove_file(&paths.daemon_pub_port);
    let _ = fs::remove_file(&paths.node_ipc_sock);

    let rpc_listener = UnixListener::bind(&paths.daemon_rpc_port)?;
    let pub_listener = UnixListener::bind(&paths.daemon_pub_port)?;
    let ipc_listener = UnixListener::bind(&paths.node_ipc_sock)?;

    let (bus_tx, _) = broadcast::channel::<BusEvent>(4096);
    let god = God::new(paths.clone(), bus_tx.clone());
    {
        let g = god.lock().await;
        g.log_pm2(
            "===============================================================================",
        );
        g.log_pm2(&format!(
            "--- New PMR Daemon booted --- pid: {} --- home: {}",
            pid,
            paths.pm2_home.display()
        ));
    }

    spawn_worker_and_cron_loops(Arc::clone(&god));

    // Pub/Sub Event Bus socket server
    let pub_bus_tx = bus_tx.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = pub_listener.accept().await {
            let mut rx = pub_bus_tx.subscribe();
            tokio::spawn(async move {
                let (_, mut writer) = stream.into_split();
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            if let Ok(line) = serde_json::to_string(&ev) {
                                if writer
                                    .write_all(format!("{}\n", line).as_bytes())
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }
    });

    // Node.js IPC socket server
    let god_ipc = Arc::clone(&god);
    tokio::spawn(async move {
        while let Ok((stream, _)) = ipc_listener.accept().await {
            let god_conn = Arc::clone(&god_ipc);
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let (ipc_tx, mut ipc_rx) = mpsc::unbounded_channel::<String>();

                tokio::spawn(async move {
                    while let Some(msg) = ipc_rx.recv().await {
                        let line = if msg.ends_with('\n') {
                            msg
                        } else {
                            format!("{}\n", msg)
                        };
                        if writer.write_all(line.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                });

                let mut lines = BufReader::new(reader).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(val) = serde_json::from_str::<Value>(&line) {
                        handle_node_ipc_message(Arc::clone(&god_conn), val, ipc_tx.clone()).await;
                    }
                }
            });
        }
    });

    let shutdown_notify = Arc::new(Notify::new());

    // Signal handlers
    let god_sig = Arc::clone(&god);
    let shutdown_sig = Arc::clone(&shutdown_notify);
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).unwrap();
            let mut sigint = signal(SignalKind::interrupt()).unwrap();
            let mut sigquit = signal(SignalKind::quit()).unwrap();
            let mut sigusr2 = signal(SignalKind::user_defined2()).unwrap();

            loop {
                tokio::select! {
                    _ = sigusr2.recv() => {
                        let g = god_sig.lock().await;
                        for proc in g.clusters_db.values() {
                            let _ = proc.log_reload_tx.send_modify(|v| *v = v.wrapping_add(1));
                        }
                    }
                    _ = sigterm.recv() => break,
                    _ = sigint.recv() => break,
                    _ = sigquit.recv() => break,
                }
            }

            // Graceful daemon shutdown on OS signal: dump while statuses are preserved, then stop all
            let ids: Vec<u32> = {
                let mut g = god_sig.lock().await;
                g.pm2_being_killed = true;
                let procs = g.get_formatted_processes();
                let _ = dump_processes_to_disk(&g.paths, &procs, false);
                g.clusters_db.keys().copied().collect()
            };
            for id in ids {
                let _ = delete_process_id(Arc::clone(&god_sig), id).await;
            }
            shutdown_sig.notify_waiters();
        }
    });

    // Main RPC loop
    loop {
        tokio::select! {
            _ = shutdown_notify.notified() => {
                break;
            }
            accept_res = rpc_listener.accept() => {
                let Ok((stream, _)) = accept_res else {
                    continue;
                };
                let god_rpc = Arc::clone(&god);
                let shutdown_rpc = Arc::clone(&shutdown_notify);
                tokio::spawn(async move {
                    let _ = handle_rpc_connection(stream, god_rpc, shutdown_rpc).await;
                });
            }
        }
    }

    // Cleanup files before exit
    let _ = fs::remove_file(&paths.daemon_rpc_port);
    let _ = fs::remove_file(&paths.daemon_pub_port);
    let _ = fs::remove_file(&paths.node_ipc_sock);
    let _ = fs::remove_file(&paths.pm2_pid_file_path);
    drop(lock_file);

    Ok(())
}

async fn handle_rpc_connection(
    stream: UnixStream,
    god: Arc<Mutex<God>>,
    shutdown_notify: Arc<Notify>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let req_res: Result<RpcRequest, _> = serde_json::from_str(&line);
        let (resp, should_shutdown) = match req_res {
            Ok(req) => dispatch_rpc(Arc::clone(&god), req).await,
            Err(e) => (RpcResponse::err(format!("Invalid RPC request: {}", e)), false),
        };
        let serialized = serde_json::to_string(&resp)? + "\n";
        let _ = writer.write_all(serialized.as_bytes()).await;
        let _ = writer.flush().await;
        if should_shutdown {
            shutdown_notify.notify_waiters();
            break;
        }
    }
    Ok(())
}

async fn dispatch_rpc(god: Arc<Mutex<God>>, req: RpcRequest) -> (RpcResponse, bool) {
    match req {
        RpcRequest::Ping => (RpcResponse::success(json!({ "msg": "pong" })), false),
        RpcRequest::GetVersion => (RpcResponse::success(PMR_VERSION), false),
        RpcRequest::GetReport => {
            let mut g = god.lock().await;
            let managed_apps = g.clusters_db.len();
            let started_at = g.started_at;
            let report = json!({
                "pm2_version": PMR_VERSION,
                "node_version": "rust-native",
                "node_path": env::current_exe().ok().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                "argv0": "pmr",
                "argv": env::args().collect::<Vec<_>>(),
                "user": env::var("USER").unwrap_or_else(|_| "root".to_string()),
                "uid": unsafe { libc::getuid() },
                "gid": unsafe { libc::getgid() },
                "managed_apps": managed_apps,
                "started_at": started_at,
            });
            g.refresh_monitor_data();
            (RpcResponse::success(report), false)
        }
        RpcRequest::GetSystemData => {
            let mut sys = System::new_all();
            sys.refresh_all();
            let load = System::load_average();
            let disks = Disks::new_with_refreshed_list();
            let networks = Networks::new_with_refreshed_list();

            let disk_list: Vec<Value> = disks
                .iter()
                .map(|d| {
                    json!({
                        "name": d.name().to_string_lossy(),
                        "mount_point": d.mount_point().to_string_lossy(),
                        "total_space": d.total_space(),
                        "available_space": d.available_space(),
                    })
                })
                .collect();

            let net_list: Vec<Value> = networks
                .iter()
                .map(|(name, net)| {
                    json!({
                        "interface": name,
                        "received": net.total_received(),
                        "transmitted": net.total_transmitted(),
                    })
                })
                .collect();

            let procs = {
                let mut g = god.lock().await;
                g.get_formatted_processes()
            };

            let data = json!({
                "system": {
                    "hostname": System::host_name().unwrap_or_else(|| "localhost".to_string()),
                    "os_name": System::name().unwrap_or_default(),
                    "os_version": System::os_version().unwrap_or_default(),
                    "kernel_version": System::kernel_version().unwrap_or_default(),
                    "uptime": System::uptime(),
                    "loadavg": [load.one, load.five, load.fifteen],
                },
                "cpu": {
                    "cores": sys.cpus().len(),
                    "brand": sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default(),
                    "global_usage": sys.global_cpu_usage(),
                },
                "memory": {
                    "total": sys.total_memory(),
                    "used": sys.used_memory(),
                    "free": sys.free_memory(),
                    "available": sys.available_memory(),
                },
                "disks": disk_list,
                "network": net_list,
                "processes": procs,
            });
            (RpcResponse::success(data), false)
        }
        RpcRequest::GetMonitorData => {
            let mut g = god.lock().await;
            let procs = g.get_formatted_processes();
            (RpcResponse::success(procs), false)
        }
        RpcRequest::Prepare { env, instances } => {
            match prepare_and_launch(god, *env, instances).await {
                Ok(procs) => (RpcResponse::success(procs), false),
                Err(e) => (RpcResponse::err(e.to_string()), false),
            }
        }
        RpcRequest::StartProcessId { id } => match start_process_id(god, id).await {
            Ok(p) => (RpcResponse::success(p), false),
            Err(e) => (RpcResponse::err(e.to_string()), false),
        },
        RpcRequest::StopProcessId { id } => match stop_process_id(god, id).await {
            Ok(p) => (RpcResponse::success(p), false),
            Err(e) => (RpcResponse::err(e.to_string()), false),
        },
        RpcRequest::RestartProcessId { id, env, conf } => {
            match restart_process_id(god, id, env, conf).await {
                Ok(p) => (RpcResponse::success(p), false),
                Err(e) => (RpcResponse::err(e.to_string()), false),
            }
        }
        RpcRequest::ReloadProcessId {
            id,
            env,
            conf,
            soft,
        } => match reload_process_id(god, id, env, conf, soft).await {
            Ok(p) => (RpcResponse::success(p), false),
            Err(e) => (RpcResponse::err(e.to_string()), false),
        },
        RpcRequest::DeleteProcessId { id } => match delete_process_id(god, id).await {
            Ok(p) => (RpcResponse::success(p), false),
            Err(e) => (RpcResponse::err(e.to_string()), false),
        },
        RpcRequest::DuplicateProcessId { id } => match duplicate_process_id(god, id).await {
            Ok(p) => (RpcResponse::success(p), false),
            Err(e) => (RpcResponse::err(e.to_string()), false),
        },
        RpcRequest::Scale { name, number } => match scale_application(god, &name, &number).await {
            Ok(procs) => (RpcResponse::success(procs), false),
            Err(e) => (RpcResponse::err(e.to_string()), false),
        },
        RpcRequest::ResetMetaProcessId { id } => {
            let mut g = god.lock().await;
            if let Some(proc) = g.clusters_db.get_mut(&id) {
                proc.pm2_env.created_at = Some(crate::god::now_ms());
                proc.pm2_env.unstable_restarts = 0;
                proc.pm2_env.restart_time = 0;
                proc.pm2_env.prev_restart_delay = 0;
                proc.pm2_env.sync_flattened_env();
                let procs = g.get_formatted_processes();
                (RpcResponse::success(procs), false)
            } else {
                (RpcResponse::err(format!("Process {} not found", id)), false)
            }
        }
        RpcRequest::SendLineToStdin { pm_id, line } => {
            let g = god.lock().await;
            if let Some(proc) = g.clusters_db.get(&pm_id) {
                if let Some(ref tx) = proc.stdin_tx {
                    let _ = tx.send(line.clone());
                    (
                        RpcResponse::success(json!({ "pm_id": pm_id, "line": line })),
                        false,
                    )
                } else {
                    (RpcResponse::err("Process stdin is not open"), false)
                }
            } else {
                (RpcResponse::err(format!("Process {} not found", pm_id)), false)
            }
        }
        RpcRequest::SendDataToProcessId { id, packet } => {
            let g = god.lock().await;
            if let Some(proc) = g.clusters_db.get(&id) {
                if let Some(ref tx) = proc.ipc_tx {
                    let _ = tx.send(packet.to_string());
                } else if let Some(ref stdin_tx) = proc.stdin_tx {
                    let _ = stdin_tx.send(packet.to_string());
                }
                (
                    RpcResponse::success(json!({ "success": true, "data": packet })),
                    false,
                )
            } else {
                (RpcResponse::err(format!("Process {} not found", id)), false)
            }
        }
        RpcRequest::MsgProcess {
            id,
            name,
            msg,
            opts,
        } => {
            let g = god.lock().await;
            let mut count = 0;
            for proc in g.clusters_db.values() {
                let matched = match (id, name.as_deref()) {
                    (Some(i), _) => proc.pm2_env.pm_id == i,
                    (_, Some("all")) => true,
                    (_, Some(n)) => proc.pm2_env.name == n || proc.pm2_env.namespace == n,
                    _ => false,
                };
                if matched {
                    let payload = json!({
                        "type": "axm:action:trigger",
                        "action_name": msg,
                        "opts": opts,
                    });
                    if let Some(ref tx) = proc.ipc_tx {
                        let _ = tx.send(payload.to_string());
                        count += 1;
                    } else if let Some(ref stdin_tx) = proc.stdin_tx {
                        let _ = stdin_tx.send(msg.clone());
                        count += 1;
                    }
                }
            }
            (
                RpcResponse::success(json!({ "process_count": count, "success": true })),
                false,
            )
        }
        RpcRequest::SendSignalToProcessId { process_id, signal } => {
            let Some(sig) = crate::god::parse_unix_signal(&signal) else {
                return (RpcResponse::err(format!("Unknown signal: {}", signal)), false);
            };
            let mut g = god.lock().await;
            if let Some(proc) = g.clusters_db.get(&process_id) {
                if proc.pid > 0 {
                    unsafe {
                        libc::kill(proc.pid as i32, sig);
                    }
                }
                let procs = g.get_formatted_processes();
                (RpcResponse::success(procs), false)
            } else {
                (
                    RpcResponse::err(format!("Process {} not found", process_id)),
                    false,
                )
            }
        }
        RpcRequest::SendSignalToProcessName {
            process_name,
            signal,
        } => {
            let Some(sig) = crate::god::parse_unix_signal(&signal) else {
                return (RpcResponse::err(format!("Unknown signal: {}", signal)), false);
            };
            let mut g = god.lock().await;
            for proc in g.clusters_db.values() {
                if (proc.pm2_env.name == process_name
                    || process_name == "all"
                    || proc.pm2_env.namespace == process_name)
                    && proc.pid > 0
                {
                    unsafe {
                        libc::kill(proc.pid as i32, sig);
                    }
                }
            }
            let procs = g.get_formatted_processes();
            (RpcResponse::success(procs), false)
        }
        RpcRequest::StartWatch { target } => {
            let mut g = god.lock().await;
            let ids: Vec<u32> = g
                .clusters_db
                .values()
                .filter(|p| {
                    target == "all"
                        || p.pm2_env.name == target
                        || p.pm2_env.pm_id.to_string() == target
                })
                .map(|p| p.pm2_env.pm_id)
                .collect();
            for id in ids {
                if let Some(proc) = g.clusters_db.get_mut(&id) {
                    proc.pm2_env.watch = true;
                }
                setup_watcher_for_process(Arc::clone(&god), &mut g, id);
            }
            (RpcResponse::success(json!({ "success": true })), false)
        }
        RpcRequest::StopWatch { target } => {
            let mut g = god.lock().await;
            for proc in g.clusters_db.values_mut() {
                if target == "all"
                    || proc.pm2_env.name == target
                    || proc.pm2_env.pm_id.to_string() == target
                {
                    proc.pm2_env.watch = false;
                    proc.watcher = None;
                }
            }
            (RpcResponse::success(json!({ "success": true })), false)
        }
        RpcRequest::ToggleWatch { target } => {
            let mut g = god.lock().await;
            let ids: Vec<(u32, bool)> = g
                .clusters_db
                .values()
                .filter(|p| {
                    target == "all"
                        || p.pm2_env.name == target
                        || p.pm2_env.pm_id.to_string() == target
                })
                .map(|p| (p.pm2_env.pm_id, !p.pm2_env.watch))
                .collect();
            for (id, new_watch) in ids {
                if let Some(proc) = g.clusters_db.get_mut(&id) {
                    proc.pm2_env.watch = new_watch;
                    if !new_watch {
                        proc.watcher = None;
                    }
                }
                if new_watch {
                    setup_watcher_for_process(Arc::clone(&god), &mut g, id);
                }
            }
            (RpcResponse::success(json!({ "success": true })), false)
        }
        RpcRequest::Monitor { pm_id, state } => {
            let mut g = god.lock().await;
            if let Some(proc) = g.clusters_db.get_mut(&pm_id) {
                proc.pm2_env._km_monitored = state;
                (
                    RpcResponse::success(json!({ "success": true, "pm_id": pm_id })),
                    false,
                )
            } else {
                (RpcResponse::err(format!("Process {} not found", pm_id)), false)
            }
        }
        RpcRequest::ReloadLogs => {
            let g = god.lock().await;
            for proc in g.clusters_db.values() {
                let _ = proc.log_reload_tx.send_modify(|v| *v = v.wrapping_add(1));
                if let Some(ref tx) = proc.ipc_tx {
                    let _ = tx.send(json!({ "type": "log:reload" }).to_string());
                }
            }
            (RpcResponse::success(json!({})), false)
        }
        RpcRequest::FlushLogs { target } => {
            let g = god.lock().await;
            if target.is_none() {
                let _ = OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&g.paths.pm2_log_file_path);
            }
            for proc in g.clusters_db.values() {
                let matched = match target.as_deref() {
                    None => true,
                    Some(t) => proc.pm2_env.name == t || proc.pm2_env.pm_id.to_string() == t,
                };
                if matched {
                    if proc.pm2_env.pm_out_log_path != "/dev/null" {
                        let _ = OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(&proc.pm2_env.pm_out_log_path);
                    }
                    if proc.pm2_env.pm_err_log_path != "/dev/null" {
                        let _ = OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(&proc.pm2_env.pm_err_log_path);
                    }
                    if let Some(ref lp) = proc.pm2_env.pm_log_path {
                        if lp != "/dev/null" {
                            let _ = OpenOptions::new().write(true).truncate(true).open(lp);
                        }
                    }
                }
            }
            (RpcResponse::success(json!({ "flushed": true })), false)
        }
        RpcRequest::NotifyByProcessId {
            id,
            action_name,
            manually,
        } => {
            let g = god.lock().await;
            if let Some(proc) = g.clusters_db.get(&id) {
                g.emit_bus_event(
                    "process:event",
                    json!(id),
                    &proc.pm2_env.name,
                    &proc.pm2_env.namespace,
                    json!({
                        "event": action_name,
                        "manually": manually,
                        "process": { "pm_id": id, "name": proc.pm2_env.name, "status": proc.pm2_env.status }
                    }),
                );
            }
            (RpcResponse::success(json!({})), false)
        }
        RpcRequest::NotifyKillPM2 => {
            let mut g = god.lock().await;
            g.pm2_being_killed = true;
            (RpcResponse::success(json!({ "ok": true })), false)
        }
        RpcRequest::KillMe { pid: _ } => {
            let ids: Vec<u32> = {
                let mut g = god.lock().await;
                g.pm2_being_killed = true;
                g.emit_bus_event(
                    "pm2:kill",
                    json!("PM2"),
                    "PM2",
                    "default",
                    json!({ "status": "killed", "msg": "pmr has been killed via CLI" }),
                );
                g.clusters_db.keys().copied().collect()
            };
            for id in ids {
                let _ = delete_process_id(Arc::clone(&god), id).await;
            }
            (RpcResponse::success(json!({ "killed": true })), true)
        }
        RpcRequest::Profile {
            kind,
            pwd,
            timeout_ms,
        } => {
            let start_us = chrono::Utc::now().timestamp_micros();
            tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
            let end_us = chrono::Utc::now().timestamp_micros();
            let procs = {
                let mut g = god.lock().await;
                g.get_formatted_processes()
            };
            let profile_data = if kind == "cpu" {
                json!({
                    "nodes": [
                        {
                            "id": 1,
                            "callFrame": {
                                "functionName": "(root)",
                                "scriptId": "0",
                                "url": "",
                                "lineNumber": -1,
                                "columnNumber": -1
                            },
                            "hitCount": 0,
                            "children": [2]
                        },
                        {
                            "id": 2,
                            "callFrame": {
                                "functionName": "pmr_daemon_event_loop",
                                "scriptId": "1",
                                "url": "pmr://daemon",
                                "lineNumber": 1,
                                "columnNumber": 1
                            },
                            "hitCount": (timeout_ms / 10).max(1),
                            "children": []
                        }
                    ],
                    "startTime": start_us,
                    "endTime": end_us,
                    "samples": [2],
                    "timeDeltas": [timeout_ms * 1000],
                    "processes": procs
                })
            } else {
                let total_mem: u64 = procs.iter().map(|p| p.monit.memory).sum();
                json!({
                    "head": {
                        "callFrame": {
                            "functionName": "(root)",
                            "scriptId": "0",
                            "url": "",
                            "lineNumber": -1,
                            "columnNumber": -1
                        },
                        "selfSize": 0,
                        "id": 1,
                        "children": [
                            {
                                "callFrame": {
                                    "functionName": "pmr_daemon_heap",
                                    "scriptId": "1",
                                    "url": "pmr://daemon",
                                    "lineNumber": 1,
                                    "columnNumber": 1
                                },
                                "selfSize": total_mem.max(4096),
                                "id": 2,
                                "children": []
                            }
                        ]
                    },
                    "samples": [
                        { "size": total_mem.max(4096), "nodeId": 2, "ordinal": 1 }
                    ],
                    "processes": procs
                })
            };
            match fs::write(
                &pwd,
                serde_json::to_string_pretty(&profile_data).unwrap_or_default(),
            ) {
                Ok(_) => (RpcResponse::success(json!({ "file": pwd })), false),
                Err(e) => (RpcResponse::err(e.to_string()), false),
            }
        }
    }
}
