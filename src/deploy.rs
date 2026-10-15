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
    if args.is_empty() || args[0] == "help" {
        print_deploy_help();
        return Ok(());
    }

    let default_files = [
        "ecosystem.config.js",
        "ecosystem.json",
        "ecosystem.yaml",
        "ecosystem.yml",
        "ecosystem.config.cjs",
        "ecosystem.config.mjs",
        "ecosystem.json5",
        "ecosystem.toml",
    ];

    let (file_path, env_name, sub_args) = if is_config_file(&args[0]).is_some() {
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
            .ok_or_else(|| anyhow!("No ecosystem configuration file found in current directory"))?;
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
    println!(
        "{}--> Deploying to {} environment (action: {})",
        PREFIX_MSG, env_name, action
    );

    if let Some(ref pre_local) = env_conf.pre_deploy_local {
        if !pre_local.trim().is_empty() {
            println!("{}Executing pre-deploy-local: {}", PREFIX_MSG, pre_local);
            let status = Command::new("sh").arg("-c").arg(pre_local).status()?;
            if !status.success() {
                return Err(anyhow!("pre-deploy-local command failed"));
            }
        }
    }

    let hosts: Vec<String> = match env_conf.host {
        Some(Value::String(ref h)) => vec![h.clone()],
        Some(Value::Array(ref arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    };

    let user = env_conf.user.as_deref().unwrap_or("root");
    let remote_path = env_conf.path.as_deref().unwrap_or("/var/www/app");
    let repo = env_conf.repo.as_deref().unwrap_or("");
    let git_ref = env_conf.git_ref.as_deref().unwrap_or("origin/main");
    let default_post = format!(
        "pmr startOrRestart {} --env {}",
        file_path.display(),
        env_name
    );
    let post_deploy = env_conf
        .post_deploy
        .as_deref()
        .unwrap_or(&default_post);

    for host in hosts {
        println!("{}--> on host {}", PREFIX_MSG, host);
        let remote_cmd = match action {
            "setup" => format!(
                "mkdir -p {path}/source {path}/shared && cd {path}/source && git clone {repo} . && {post_setup}",
                path = remote_path,
                repo = repo,
                post_setup = env_conf.post_setup.as_deref().unwrap_or("true")
            ),
            "curr" | "current" => format!("cd {}/source && git rev-parse HEAD", remote_path),
            "prev" | "previous" => format!("cd {}/source && git rev-parse HEAD~1", remote_path),
            "revert" => format!(
                "cd {}/source && git reset --hard HEAD~1 && {}",
                remote_path, post_deploy
            ),
            "exec" | "run" => {
                let extra = sub_args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
                format!("cd {}/source && {}", remote_path, extra)
            }
            _ => format!(
                "cd {}/source && git fetch --all && git reset --hard {} && {}",
                remote_path, git_ref, post_deploy
            ),
        };

        let target = format!("{}@{}", user, host);
        let status = Command::new("ssh")
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
        return Err(anyhow!("Repository directory {} does not exist", repo_dir.display()));
    }

    match action {
        "pull" => {
            if let Some(cid) = commit_id {
                println!("{}Checking out commit {} for {}...", PREFIX_MSG, cid, app_name);
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
            println!("{}Moving {} forward to latest branch tip...", PREFIX_MSG, app_name);
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
