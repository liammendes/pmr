//! Core constants, defaults, interpreter mappings, and templates for PMR (PM2 in Rust).

pub const PMR_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const ONLINE_STATUS: &str = "online";
pub const STOPPED_STATUS: &str = "stopped";
pub const STOPPING_STATUS: &str = "stopping";
pub const WAITING_RESTART: &str = "waiting restart";
pub const LAUNCHING_STATUS: &str = "launching";
pub const ERRORED_STATUS: &str = "errored";
pub const ONE_LAUNCH_STATUS: &str = "one-launch-status";

pub const CLUSTER_MODE_ID: &str = "cluster_mode";
pub const FORK_MODE_ID: &str = "fork_mode";

pub const SUCCESS_EXIT: i32 = 0;
pub const ERROR_EXIT: i32 = 1;

pub const EXP_BACKOFF_RESET_TIMER_MS: u64 = 30_000;
pub const RELOAD_LOCK_TIMEOUT_MS: u64 = 30_000;
pub const GRACEFUL_TIMEOUT_MS: u64 = 8_000;
pub const GRACEFUL_LISTEN_TIMEOUT_MS: u64 = 3_000;
pub const WORKER_INTERVAL_MS: u64 = 5_000;
pub const KILL_TIMEOUT_MS: u64 = 1_600;
pub const KILL_RETRY_TIME_MS: u64 = 100;
pub const DEFAULT_MIN_UPTIME_MS: u64 = 1_000;
pub const DEFAULT_MAX_RESTARTS: u32 = 16;
pub const DEFAULT_WEB_PORT: u16 = 9615;
pub const DEFAULT_SERVE_PORT: u16 = 8080;
pub const DEFAULT_CONCURRENT_ACTIONS: usize = 2;

pub const APP_CONF_DEFAULT_FILE: &str = "ecosystem.config.js";
pub const DEFAULT_LOG_DATE_FORMAT: &str = "YYYY-MM-DDTHH:mm:ss";

pub const PREFIX_MSG: &str = "\x1b[36m[PMR]\x1b[0m ";
pub const PREFIX_MSG_ERR: &str = "\x1b[31m[PMR][ERROR]\x1b[0m ";
pub const PREFIX_MSG_MOD: &str = "\x1b[1;32m[PMR][Module]\x1b[0m ";
pub const PREFIX_MSG_MOD_ERR: &str = "\x1b[31m[PMR][Module][ERROR]\x1b[0m ";
pub const PREFIX_MSG_WARNING: &str = "\x1b[33m[PMR][WARN]\x1b[0m ";
pub const PREFIX_MSG_SUCCESS: &str = "\x1b[36m[PMR]\x1b[0m ";

pub const PMR_BANNER: &str = r#"
                        ____  __  _______ 
                       / __ \/  |/  / __ \
                      / /_/ / /|_/ / /_/ /
                     / ____/ /  / / _, _/ 
                    /_/   /_/  /_/_/ |_|  

          Runtime Edition - Production Process Manager in Rust
"#;

/// Resolve default interpreter for a file extension (matching PM2's `interpreter.json`).
pub fn interpreter_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "sh" => Some("bash"),
        "py" => Some("python3"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "pl" => Some("perl"),
        "js" | "cjs" | "mjs" => Some("node"),
        "coffee" => Some("coffee"),
        "ls" => Some("lsc"),
        "ts" | "tsx" => Some("bun"),
        _ => None,
    }
}

pub const ECOSYSTEM_SIMPLE_TPL: &str = r#"module.exports = {
  apps : [{
    name   : "app1",
    script : "./app.js"
  }]
}
"#;

pub const ECOSYSTEM_TPL: &str = r#"module.exports = {
  apps : [{
    script: 'index.js',
    watch: '.'
  }, {
    script: './service-worker/',
    watch: ['./service-worker']
  }],

  deploy : {
    production : {
      user : 'SSH_USERNAME',
      host : 'SSH_HOSTMACHINE',
      ref  : 'origin/master',
      repo : 'GIT_REPOSITORY',
      path : 'DESTINATION_PATH',
      'pre-deploy-local': '',
      'post-deploy' : 'npm install && pmr reload ecosystem.config.js --env production',
      'pre-setup': ''
    }
  }
};
"#;

pub const SYSTEMD_TPL: &str = r#"[Unit]
Description=PMR process manager
Documentation=https://pm2.keymetrics.io/
After=network.target

[Service]
Type=forking
User=%USER%
LimitNOFILE=infinity
LimitNPROC=infinity
LimitCORE=infinity
Environment=PATH=%NODE_PATH%:/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin
Environment=PMR_HOME=%HOME_PATH%
Environment=PM2_HOME=%HOME_PATH%
PIDFile=%HOME_PATH%/pm2.pid
Restart=on-failure

ExecStart=%PM2_PATH% resurrect
ExecReload=%PM2_PATH% reload all
ExecStop=%PM2_PATH% kill

[Install]
WantedBy=multi-user.target
"#;

pub const SYSTEMD_ONLINE_TPL: &str = r#"[Unit]
Description=PMR process manager
Documentation=https://pm2.keymetrics.io/
Wants=network-online.target
After=network.target network-online.target

[Service]
Type=forking
User=%USER%
LimitNOFILE=infinity
LimitNPROC=infinity
LimitCORE=infinity
Environment=PATH=%NODE_PATH%:/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin
Environment=PMR_HOME=%HOME_PATH%
Environment=PM2_HOME=%HOME_PATH%
PIDFile=%HOME_PATH%/pm2.pid
Restart=on-failure

ExecStart=%PM2_PATH% resurrect
ExecReload=%PM2_PATH% reload all
ExecStop=%PM2_PATH% kill

[Install]
WantedBy=multi-user.target
"#;

pub const LAUNCHD_TPL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>%SERVICE_NAME%</string>
    <key>UserName</key>
    <string>%USER%</string>
    <key>KeepAlive</key>
    <true/>
    <key>ProgramArguments</key>
    <array>
      <string>/bin/sh</string>
      <string>-c</string>
      <string>%PM2_PATH% resurrect</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>OnDemand</key>
    <false/>
    <key>LaunchOnlyOnce</key>
    <true/>
    <key>EnvironmentVariables</key>
    <dict>
      <key>PATH</key>
      <string>%NODE_PATH%:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
      <key>PMR_HOME</key>
      <string>%HOME_PATH%</string>
      <key>PM2_HOME</key>
      <string>%HOME_PATH%</string>
    </dict>
    <key>StandardErrorPath</key>
    <string>/tmp/%SERVICE_NAME%.err</string>
    <key>StandardOutPath</key>
    <string>/tmp/%SERVICE_NAME%.out</string>
  </dict>
</plist>
"#;

pub const UPSTART_TPL: &str = r#"#!/bin/bash
### BEGIN INIT INFO
# Provides:          %SERVICE_NAME%
# Required-Start:    $local_fs $remote_fs
# Required-Stop:     $local_fs $remote_fs
# Should-Start:      $network
# Should-Stop:       $network
# Default-Start:     2 3 4 5
# Default-Stop:      0 1 6
# Short-Description: PMR init script
# Description:       PMR is the next gen process manager
### END INIT INFO

NAME=%SERVICE_NAME%
PM2=%PM2_PATH%
USER=%USER%
export PATH=%NODE_PATH%:$PATH
export PMR_HOME="%HOME_PATH%"
export PM2_HOME="%HOME_PATH%"

case "$1" in
    start)
        su - $USER -c "PATH=$PATH; PMR_HOME=$PMR_HOME $PM2 resurrect"
        ;;
    stop)
        su - $USER -c "PATH=$PATH; PMR_HOME=$PMR_HOME $PM2 dump && $PM2 kill"
        ;;
    restart|reload)
        su - $USER -c "PATH=$PATH; PMR_HOME=$PMR_HOME $PM2 reload all"
        ;;
    status)
        su - $USER -c "PATH=$PATH; PMR_HOME=$PMR_HOME $PM2 list"
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|reload|status}"
        exit 1
        ;;
esac
"#;

pub const OPENRC_TPL: &str = r#"#!/sbin/openrc-run

name="PMR process manager"
description="PMR next gen process manager"
command="%PM2_PATH%"
command_user="%USER%"
export PMR_HOME="%HOME_PATH%"
export PM2_HOME="%HOME_PATH%"
export PATH=%NODE_PATH%:$PATH

depend() {
    need net localmount
}

start() {
    ebegin "Starting PMR"
    su - ${command_user} -c "PATH=$PATH PMR_HOME=$PMR_HOME ${command} resurrect"
    eend $?
}

stop() {
    ebegin "Stopping PMR"
    su - ${command_user} -c "PATH=$PATH PMR_HOME=$PMR_HOME ${command} dump && ${command} kill"
    eend $?
}

reload() {
    ebegin "Reloading PMR"
    su - ${command_user} -c "PATH=$PATH PMR_HOME=$PMR_HOME ${command} reload all"
    eend $?
}
"#;

pub const LOGROTATE_TPL: &str = r#"%HOME_PATH%/logs/*.log %HOME_PATH%/pmr.log %HOME_PATH%/pm2.log {
        su %USER% %USER%
        rotate 12
        weekly
        missingok
        notifempty
        compress
        delaycompress
        create 0640 %USER% %USER%
        copytruncate
}
"#;

pub const NODE_PRELOAD_SHIM_JS: &str = r#"
(function() {
  const expectedParentPid = Number(process.env.PMR_PARENT_PID || 0);
  const ipcSockPath = process.env.PMR_NODE_IPC_SOCK;

  // Strip our preload shim from NODE_OPTIONS and clear PMR_NODE_IPC_SOCK / PMR_PARENT_PID
  // so grandchild Node.js processes spawned by user apps do not inherit the shim!
  if (process.env.NODE_OPTIONS) {
    process.env.NODE_OPTIONS = process.env.NODE_OPTIONS
      .replace(/--require\s+"[^"]*pmr-preload\.cjs"/g, '')
      .replace(/--require\s+[^\s]*pmr-preload\.cjs/g, '')
      .trim();
    if (!process.env.NODE_OPTIONS) {
      delete process.env.NODE_OPTIONS;
    }
  }
  delete process.env.PMR_NODE_IPC_SOCK;
  delete process.env.PMR_PARENT_PID;

  if (expectedParentPid > 0 && process.ppid !== expectedParentPid) {
    return;
  }

  const net = require('net');
  const cluster = require('cluster');
  const isCluster = process.env.exec_mode === 'cluster_mode';
  const pmId = Number(process.env.pm_id || 0);
  const instanceId = Number(process.env.NODE_APP_INSTANCE || 0);

  if (process.env.PROCESS_TITLE) {
    process.title = process.env.PROCESS_TITLE;
  } else if (process.env.pm_exec_path) {
    process.title = 'node ' + process.env.pm_exec_path;
  }

  if (isCluster) {
    try {
      Object.defineProperty(cluster, 'isWorker', { value: true, configurable: true, writable: true });
      Object.defineProperty(cluster, 'isPrimary', { value: false, configurable: true, writable: true });
      Object.defineProperty(cluster, 'isMaster', { value: false, configurable: true, writable: true });
      if (!cluster.worker) {
        cluster.worker = {
          id: instanceId + 1,
          process: process,
          send: function(...a) { return process.send ? process.send(...a) : false; },
          isDead: function() { return false; },
          isConnected: function() { return Boolean(process.connected); }
        };
      }
    } catch (e) {}
  }

  let ipcClient = null;
  let ipcConnected = false;
  let pendingMsgs = [];
  let clusterAckCallbacks = [];

  function sendToDaemon(payload) {
    payload.pid = process.pid;
    payload.pm_id = pmId;
    const line = JSON.stringify(payload) + '\n';
    if (ipcClient && ipcConnected) {
      ipcClient.write(line);
    } else if (ipcSockPath) {
      pendingMsgs.push(line);
    }
  }

  if (ipcSockPath) {
    try {
      ipcClient = net.createConnection(ipcSockPath, () => {
        ipcConnected = true;
        ipcClient.unref();
        sendToDaemon({
          type: 'internal:hello',
          node_version: process.versions.node
        });
        while (pendingMsgs.length > 0) {
          ipcClient.write(pendingMsgs.shift());
        }
      });
      ipcClient.unref();
      let buf = '';
      ipcClient.on('data', (chunk) => {
        buf += chunk.toString();
        let idx;
        while ((idx = buf.indexOf('\n')) !== -1) {
          const line = buf.slice(0, idx).trim();
          buf = buf.slice(idx + 1);
          if (!line) continue;
          try {
            const msg = JSON.parse(line);
            if (msg && msg.__pmr_cluster_ack) {
              while (clusterAckCallbacks.length > 0) {
                const cb = clusterAckCallbacks.shift();
                cb();
              }
            } else if (msg && msg.type === 'axm:action:trigger') {
              const actionName = msg.action_name;
              const param = msg.opts;
              if (actionName === 'km:inspect' || actionName === 'internal:inspect') {
                try {
                  const inspector = require('inspector');
                  if (inspector.url()) {
                    inspector.close();
                    sendToDaemon({ type: 'process:msg', raw: { type: 'axm:reply', data: { action_name: actionName, return: '' } } });
                  } else {
                    inspector.open();
                    sendToDaemon({ type: 'process:msg', raw: { type: 'axm:reply', data: { action_name: actionName, return: inspector.url() || 'enabled' } } });
                  }
                } catch (err) {
                  sendToDaemon({ type: 'process:msg', raw: { type: 'axm:reply', data: { action_name: actionName, return: null, error: String(err) } } });
                }
              } else {
                process.emit('message', actionName, param);
              }
            } else if (msg && msg.__pmr_raw !== undefined) {
              process.emit('message', msg.__pmr_raw, undefined);
            } else {
              process.emit('message', msg, undefined);
            }
          } catch (e) {
            process.emit('message', line, undefined);
          }
        }
      });
      ipcClient.on('error', () => {});
    } catch (e) {}
  }

  if (typeof process.send !== 'function') {
    process.connected = true;
    process.disconnect = function() {
      process.connected = false;
      if (ipcClient) {
        try { ipcClient.end(); } catch (e) {}
      }
    };
    process.send = function(msg, sendHandle, options, callback) {
      if (typeof sendHandle === 'function') {
        callback = sendHandle;
      } else if (typeof options === 'function') {
        callback = options;
      }
      sendToDaemon({
        type: 'process:msg',
        raw: msg
      });
      if (typeof callback === 'function') process.nextTick(callback, null);
      return true;
    };
  }

  const origListen = net.Server.prototype.listen;
  const origAddress = net.Server.prototype.address;

  net.Server.prototype.listen = function(...args) {
    let requestedPort = null;
    let requestedHost = null;
    let userCb = null;

    if (args.length > 0) {
      if (typeof args[0] === 'number' || (typeof args[0] === 'string' && /^\d+$/.test(args[0]))) {
        requestedPort = Number(args[0]);
        if (args.length > 1 && typeof args[1] === 'string') {
          requestedHost = args[1];
        }
      } else if (args[0] && typeof args[0] === 'object') {
        if (typeof args[0].port === 'number' || (typeof args[0].port === 'string' && /^\d+$/.test(args[0].port))) {
          requestedPort = Number(args[0].port);
        }
        if (typeof args[0].host === 'string') {
          requestedHost = args[0].host;
        }
      }
    }

    if (isCluster && requestedPort && requestedPort > 0) {
      if (typeof args[args.length - 1] === 'function') {
        userCb = args.pop();
      }
      this.__pmr_requested_port = requestedPort;
      this.address = function() {
        const addr = origAddress.call(this);
        if (addr && typeof addr === 'object') {
          return Object.assign({}, addr, { port: requestedPort });
        }
        return addr;
      };

      this.once('listening', () => {
        const realAddr = origAddress.call(this);
        const workerPort = realAddr && typeof realAddr === 'object' ? realAddr.port : requestedPort;
        if (userCb) {
          clusterAckCallbacks.push(() => userCb.call(this));
        }
        sendToDaemon({
          type: 'listening',
          requested_port: requestedPort,
          requested_host: requestedHost,
          worker_port: workerPort,
          data: realAddr
        });
      });

      // Bind worker to loopback ephemeral port 0 with exclusive: true so Node does not call cluster._getServer
      args = [{ port: 0, host: '127.0.0.1', exclusive: true }];
      return origListen.apply(this, args);
    }

    this.once('listening', () => {
      try {
        const addr = origAddress.call(this);
        sendToDaemon({
          type: 'listening',
          data: addr
        });
      } catch (e) {}
    });
    const wasPrimary = cluster.isPrimary;
    try {
      if (isCluster) {
        Object.defineProperty(cluster, 'isPrimary', { value: true, configurable: true, writable: true });
      }
      return origListen.apply(this, args);
    } finally {
      if (isCluster) {
        Object.defineProperty(cluster, 'isPrimary', { value: wasPrimary, configurable: true, writable: true });
      }
    }
  };
})();
"#;

pub const MODULE_CONF_PREFIX: &str = "module-db-v2";
