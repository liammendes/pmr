use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::client::PmrClient;
use crate::config::{is_config_file, parse_config_file, StartOptions};
use crate::constants::{PREFIX_MSG, PREFIX_MSG_ERR};
use crate::process::ProcessDescription;

pub fn print_deploy_help() {
    println!(
        r#"
  Usage: pmr deploy <configuration_file> <environment> <command>

  Commands:
    setup                run remote setup commands
    update               update deploy to the latest release
    revert [n]           revert to [n]th last deployment or 1
    curr[ent]            output current release commit
    prev[ious]           output previous release commit
    exec|run <cmd>       execute the given <cmd>
    list                 list previous deploy commits
    [ref]                deploy to [ref], the "ref" setting, or latest tag
"#
    );
}

pub fn cmd_deploy(args: &[String]) -> Result<()> {
    if args.is_empty() || args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
        print_deploy_help();
        return Ok(());
    }

    let default_files = [
        "ecosystem.config.js",
        "ecosystem.config.cjs",
        "ecosystem.config.mjs",
        "ecosystem.json",
        "ecosystem.json5",
        "ecosystem.yml",
        "ecosystem.yaml",
        "ecosystem.toml",
        "pm2.config.js",
        "process.json",
        "package.json",
    ];

    let (file_path, env_name, sub_args) =
        if is_config_file(&args[0]).is_some() || Path::new(&args[0]).is_file() {
            if args.len() < 2 {
                print_deploy_help();
                return Err(anyhow!("Missing <environment> argument for deploy"));
            }
            (PathBuf::from(&args[0]), args[1].clone(), &args[2..])
        } else {
            let found = default_files
                .iter()
                .map(PathBuf::from)
                .find(|p| p.exists())
                .ok_or_else(|| {
                    anyhow!("No ecosystem configuration file found in current directory")
                })?;
            (found, args[0].clone(), &args[1..])
        };

    let eco = parse_config_file(&file_path)?;
    let env_conf = eco.deploy.get(&env_name).ok_or_else(|| {
        eprintln!(
            "{}{} environment is not defined in {} file",
            PREFIX_MSG_ERR,
            env_name,
            file_path.display()
        );
        anyhow!("Environment {} not defined", env_name)
    })?;

    let action = sub_args.first().map(|s| s.as_str()).unwrap_or("update");
    if action == "config" {
        println!("{}", serde_json::to_string_pretty(&env_conf)?);
        return Ok(());
    }

    println!(
        "{}--> Deploying to {} environment (action: {})",
        PREFIX_MSG, env_name, action
    );

    let is_deploy_action = matches!(action, "update" | "ref");
    if is_deploy_action {
        if let Some(ref pre_local) = env_conf.pre_deploy_local {
            if !pre_local.trim().is_empty() {
                println!("{}Executing pre-deploy-local: {}", PREFIX_MSG, pre_local);
                let status = Command::new("sh").arg("-c").arg(pre_local).status()?;
                if !status.success() {
                    return Err(anyhow!("pre-deploy-local command failed"));
                }
            }
        }
    }

    let hosts: Vec<String> = match env_conf.host {
        Some(Value::String(ref h)) => vec![h.clone()],
        Some(Value::Array(ref arr)) => arr
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Object(obj) => obj
                    .get("host")
                    .and_then(|h| h.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    if hosts.is_empty() {
        return Err(anyhow!(
            "No deploy host specified for environment {}",
            env_name
        ));
    }

    let remote_path = env_conf.path.as_deref().unwrap_or("/var/www/app");
    let repo = env_conf.repo.as_deref().unwrap_or("");
    let git_ref = if action == "ref" {
        sub_args
            .get(1)
            .map(|s| s.as_str())
            .or(env_conf.git_ref.as_deref())
            .unwrap_or("origin/main")
    } else {
        env_conf.git_ref.as_deref().unwrap_or("origin/main")
    };

    let mut env_exports = String::new();
    for (k, v) in &env_conf.env {
        let escaped = v.replace('\'', "'\\''");
        env_exports.push_str(&format!("export {}='{}'; ", k, escaped));
    }

    let default_post = format!(
        "pmr startOrRestart {} --env {}",
        file_path.display(),
        env_name
    );
    let post_deploy = env_conf.post_deploy.as_deref().unwrap_or(&default_post);
    let pre_deploy = env_conf.pre_deploy.as_deref().unwrap_or("true");
    let pre_setup = env_conf.pre_setup.as_deref().unwrap_or("true");
    let post_setup = env_conf.post_setup.as_deref().unwrap_or("true");

    let remote_cmd = match action {
        "setup" => format!(
            "{env}{pre_setup} && mkdir -p {path}/source {path}/shared && ln -sfn {path}/source {path}/current && cd {path}/source && git clone {repo} . && {post_setup}",
            env = env_exports,
            pre_setup = pre_setup,
            path = remote_path,
            repo = repo,
            post_setup = post_setup
        ),
        "curr" | "current" => format!("cd {}/source && git rev-parse HEAD", remote_path),
        "prev" | "previous" => format!("cd {}/source && git rev-parse HEAD~1", remote_path),
        "list" => format!(
            "cd {}/source && git log --oneline -n 10",
            remote_path
        ),
        "revert" => {
            let n: usize = sub_args
                .get(1)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            format!(
                "{env}cd {path}/source && git reset --hard HEAD~{n} && {post}",
                env = env_exports,
                path = remote_path,
                n = n,
                post = post_deploy
            )
        }
        "exec" | "run" => {
            let extra = sub_args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            if extra.is_empty() {
                return Err(anyhow!("Missing command for `deploy exec`"));
            }
            format!(
                "{env}cd {path}/source && {extra}",
                env = env_exports,
                path = remote_path,
                extra = extra
            )
        }
        "update" | "ref" => format!(
            "{env}mkdir -p {path}/source {path}/shared && ln -sfn {path}/source {path}/current && cd {path}/source && {pre} && git fetch --all && git reset --hard {git_ref} && {post}",
            env = env_exports,
            path = remote_path,
            pre = pre_deploy,
            git_ref = git_ref,
            post = post_deploy
        ),
        other => {
            return Err(anyhow!(
                "Unknown deploy command '{}'. Valid commands: setup, update, revert [n], curr, prev, exec <cmd>, list, ref <ref>",
                other
            ));
        }
    };

    let mut ssh_base_args: Vec<String> = Vec::new();
    if let Some(ref key_path) = env_conf.key {
        ssh_base_args.push("-i".to_string());
        ssh_base_args.push(key_path.clone());
    }
    if let Some(ref port_val) = env_conf.port {
        let p_str = match port_val {
            Value::Number(n) => Some(n.to_string()),
            Value::String(s) => Some(s.clone()),
            _ => None,
        };
        if let Some(p) = p_str {
            ssh_base_args.push("-p".to_string());
            ssh_base_args.push(p);
        }
    }
    match env_conf.ssh_options {
        Some(Value::String(ref opt)) => {
            ssh_base_args.push("-o".to_string());
            ssh_base_args.push(opt.clone());
        }
        Some(Value::Array(ref arr)) => {
            for item in arr {
                if let Some(opt) = item.as_str() {
                    ssh_base_args.push("-o".to_string());
                    ssh_base_args.push(opt.to_string());
                }
            }
        }
        _ => {}
    }

    for host in hosts {
        println!("{}--> on host {}", PREFIX_MSG, host);
        let target = match env_conf.user.as_deref() {
            Some(u) if !u.is_empty() => format!("{}@{}", u, host),
            _ => host.clone(),
        };
        let status = Command::new("ssh")
            .args(&ssh_base_args)
            .arg(&target)
            .arg(&remote_cmd)
            .status()?;
        if !status.success() {
            return Err(anyhow!("SSH deploy to {} failed", target));
        }
    }

    println!("{}--> Success", PREFIX_MSG);
    Ok(())
}

pub async fn cmd_git_version_action(
    client: &PmrClient,
    action: &str,
    app_name: &str,
    commit_id: Option<&str>,
) -> Result<Vec<ProcessDescription>> {
    let procs = client.list().await?;
    let proc = procs
        .iter()
        .find(|p| p.name == app_name || p.pm_id.to_string() == app_name)
        .ok_or_else(|| anyhow!("Process {} not found", app_name))?;

    let repo_dir = proc
        .pm2_env
        .versioning
        .as_ref()
        .map(|v| PathBuf::from(&v.repo_path))
        .unwrap_or_else(|| PathBuf::from(&proc.pm2_env.pm_cwd));

    if !Path::new(&repo_dir).exists() {
        return Err(anyhow!(
            "Repository directory {} does not exist",
            repo_dir.display()
        ));
    }

    match action {
        "pull" => {
            if let Some(cid) = commit_id {
                println!(
                    "{}Checking out commit {} for {}...",
                    PREFIX_MSG, cid, app_name
                );
                let _ = Command::new("git")
                    .args(["fetch", "--all"])
                    .current_dir(&repo_dir)
                    .status();
                let status = Command::new("git")
                    .args(["checkout", cid])
                    .current_dir(&repo_dir)
                    .status()?;
                if !status.success() {
                    return Err(anyhow!("Failed to checkout commit {}", cid));
                }
            } else {
                println!("{}Pulling latest changes for {}...", PREFIX_MSG, app_name);
                let status = Command::new("git")
                    .args(["pull"])
                    .current_dir(&repo_dir)
                    .status()?;
                if !status.success() {
                    return Err(anyhow!("git pull failed for {}", app_name));
                }
            }
        }
        "backward" => {
            println!("{}Moving {} backward one commit...", PREFIX_MSG, app_name);
            let status = Command::new("git")
                .args(["checkout", "HEAD~1"])
                .current_dir(&repo_dir)
                .status()?;
            if !status.success() {
                return Err(anyhow!("git checkout HEAD~1 failed for {}", app_name));
            }
        }
        "forward" => {
            println!(
                "{}Moving {} forward to latest branch tip...",
                PREFIX_MSG, app_name
            );
            let branch = proc
                .pm2_env
                .versioning
                .as_ref()
                .map(|v| v.branch.as_str())
                .unwrap_or("main");
            let status = Command::new("git")
                .args(["checkout", branch])
                .current_dir(&repo_dir)
                .status()?;
            if !status.success() {
                return Err(anyhow!("git checkout {} failed", branch));
            }
            let _ = Command::new("git")
                .args(["pull"])
                .current_dir(&repo_dir)
                .status();
        }
        _ => {}
    }

    client
        .reload(app_name, &StartOptions::default(), false)
        .await
}
