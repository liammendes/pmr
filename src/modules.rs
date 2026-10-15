use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::client::PmrClient;
use crate::config::{AppConfig, ModuleConfiguration, StartOptions};
use crate::constants::{MODULE_CONF_PREFIX, PREFIX_MSG_MOD, PREFIX_MSG_MOD_ERR};
use crate::process::ProcessDescription;
use crate::protocol::RpcRequest;

/// Port of PM2 `NPM.getCanonicModuleName`: preserves `@scope/pkg` while stripping version/tag/git suffixes.
pub fn get_canonic_module_name(spec: &str) -> String {
    let without_git = spec.strip_suffix(".git").unwrap_or(spec);
    if let Some(rest) = without_git.strip_prefix('@') {
        // Scoped package like `@scope/pkg` or `@scope/pkg@1.0.0`
        if let Some((scope_and_pkg, _ver)) = rest.split_once('@') {
            return format!("@{}", scope_and_pkg);
        }
        return format!("@{}", rest);
    }
    if without_git.contains('/') {
        let last = without_git.rsplit('/').next().unwrap_or(without_git);
        return last.split('@').next().unwrap_or(last).to_string();
    }
    without_git
        .split('@')
        .next()
        .unwrap_or(without_git)
        .to_string()
}

pub async fn cmd_install_module(
    client: &PmrClient,
    module_spec: &str,
    opts: &StartOptions,
) -> Result<Vec<ProcessDescription>> {
    println!("{}Installing module {}...", PREFIX_MSG_MOD, module_spec);

    let local_path = Path::new(module_spec);
    let mod_dir = if local_path.exists() && local_path.is_dir() {
        fs::canonicalize(local_path)?
    } else {
        let canonic = get_canonic_module_name(module_spec);
        let folder_name = canonic.replace('/', "-");
        let dest = client.paths.default_module_path.join(&folder_name);
        fs::create_dir_all(&dest)?;
        let status = Command::new("npm")
            .args([
                "install",
                module_spec,
                "--prefix",
                &dest.to_string_lossy(),
                "--loglevel=error",
            ])
            .status()?;
        if !status.success() {
            eprintln!(
                "{}Failed to install npm module {}",
                PREFIX_MSG_MOD_ERR, module_spec
            );
            return Err(anyhow!("Module installation failed: {}", module_spec));
        }
        dest.join("node_modules").join(&canonic)
    };

    let pkg_json_path = mod_dir.join("package.json");
    let mut mod_name = get_canonic_module_name(module_spec);
    let mut mod_version = "1.0.0".to_string();
    let mut app_cfg: Option<AppConfig> = None;

    if pkg_json_path.exists() {
        let raw = fs::read_to_string(&pkg_json_path)?;
        let val: Value = serde_json::from_str(&raw)?;
        if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
            mod_name = name.to_string();
        }
        if let Some(ver) = val.get("version").and_then(|v| v.as_str()) {
            mod_version = ver.to_string();
        }

        // Copy default package.json `config` keys into `module_conf.json` under `<mod_name>:<key>` if not set
        if let Some(conf_obj) = val.get("config").and_then(|v| v.as_object()) {
            let existing = ModuleConfiguration::get_all_sync(&client.paths);
            let existing_mod = existing.get(&mod_name).and_then(|v| v.as_object());
            for (k, v) in conf_obj {
                let already_set = existing_mod.map(|m| m.contains_key(k)).unwrap_or(false);
                if !already_set {
                    let s = v
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string());
                    let _ = ModuleConfiguration::set_sync(
                        &client.paths,
                        &format!("{}:{}", mod_name, k),
                        &s,
                    );
                }
            }
        }

        // Script detection priority matching PM2 NPM.js:
        // 1. `apps` or `pm2`
        // 2. `bin`
        // 3. `main` (or `index.js`)
        if let Some(apps_val) = val.get("apps").or_else(|| val.get("pm2")) {
            let first = if let Some(arr) = apps_val.as_array() {
                arr.first()
            } else if apps_val.is_object() {
                Some(apps_val)
            } else {
                None
            };
            if let Some(item) = first {
                if let Ok(mut parsed) = AppConfig::from_json_value(item) {
                    let script_path = mod_dir.join(&parsed.script);
                    parsed.script = script_path.to_string_lossy().to_string();
                    parsed.name = Some(parsed.name.unwrap_or_else(|| mod_name.clone()));
                    parsed.cwd = Some(mod_dir.to_string_lossy().to_string());
                    parsed.pmx_module = Some(true);
                    app_cfg = Some(parsed);
                }
            }
        }

        if app_cfg.is_none() {
            let rel_script = if let Some(bin_val) = val.get("bin") {
                if let Some(s) = bin_val.as_str() {
                    s.to_string()
                } else if let Some(obj) = bin_val.as_object() {
                    obj.values()
                        .next()
                        .and_then(|v| v.as_str())
                        .unwrap_or("index.js")
                        .to_string()
                } else {
                    "index.js".to_string()
                }
            } else {
                val.get("main")
                    .and_then(|v| v.as_str())
                    .unwrap_or("index.js")
                    .to_string()
            };
            app_cfg = Some(AppConfig {
                script: mod_dir.join(rel_script).to_string_lossy().to_string(),
                name: Some(mod_name.clone()),
                cwd: Some(mod_dir.to_string_lossy().to_string()),
                pmx_module: Some(true),
                ..Default::default()
            });
        }
    }

    let app = app_cfg.unwrap_or_else(|| AppConfig {
        script: mod_dir.join("index.js").to_string_lossy().to_string(),
        name: Some(mod_name.clone()),
        cwd: Some(mod_dir.to_string_lossy().to_string()),
        pmx_module: Some(true),
        ..Default::default()
    });

    // Record in module-db-v2 in module_conf.json
    let _ = ModuleConfiguration::set_value_sync(
        &client.paths,
        &format!("{}:{}", MODULE_CONF_PREFIX, mod_name),
        json!({
            "version": mod_version,
            "git_repository": false,
        }),
    );

    let procs = client
        .start_app_config(app, opts, &BTreeMap::new(), Some(false))
        .await?;
    println!(
        "{}Module {} successfully installed and launched",
        PREFIX_MSG_MOD, mod_name
    );
    Ok(procs)
}

pub async fn cmd_uninstall_module(
    client: &PmrClient,
    module_name: &str,
) -> Result<Vec<ProcessDescription>> {
    println!("{}Uninstalling module {}...", PREFIX_MSG_MOD, module_name);
    let procs = client.list().await?;
    let mod_procs: Vec<u32> = procs
        .iter()
        .filter(|p| p.pm2_env.pmx_module && p.name == module_name)
        .map(|p| p.pm_id)
        .collect();

    let all_conf = ModuleConfiguration::get_all_sync(&client.paths);
    let in_db = all_conf
        .get(MODULE_CONF_PREFIX)
        .and_then(|v| v.get(module_name))
        .is_some();
    let folder_name = module_name.replace('/', "-");
    let mod_dir = client.paths.default_module_path.join(&folder_name);
    let raw_mod_dir = client.paths.default_module_path.join(module_name);

    if mod_procs.is_empty() && !in_db && !mod_dir.exists() && !raw_mod_dir.exists() {
        eprintln!("{}Unknown module {}", PREFIX_MSG_MOD_ERR, module_name);
        return Err(anyhow!("Unknown module {}", module_name));
    }

    for id in mod_procs {
        let _ = client.request(&RpcRequest::DeleteProcessId { id }).await;
    }

    let _ = ModuleConfiguration::unset_sync(
        &client.paths,
        &format!("{}:{}", MODULE_CONF_PREFIX, module_name),
    );
    if mod_dir.exists() {
        let _ = fs::remove_dir_all(&mod_dir);
    }
    if raw_mod_dir.exists() {
        let _ = fs::remove_dir_all(&raw_mod_dir);
    }
    println!("{}Module {} uninstalled", PREFIX_MSG_MOD, module_name);
    client.list().await
}

pub fn cmd_generate_module_sample(app_name: Option<&str>) -> Result<PathBuf> {
    let name = app_name.unwrap_or("my-pmr-module");
    let dest = PathBuf::from(name);
    fs::create_dir_all(&dest)?;

    let pkg_json = serde_json::json!({
        "name": name,
        "version": "1.0.0",
        "description": "PMR / PM2 module",
        "main": "index.js",
        "config": {},
        "apps": [{
            "name": name,
            "script": "index.js"
        }]
    });
    fs::write(
        dest.join("package.json"),
        serde_json::to_string_pretty(&pkg_json)?,
    )?;

    let index_js = r#"// PMR / PM2 Sample Module
console.log("Module started with PID:", process.pid);
setInterval(() => {}, 10000);
"#;
    fs::write(dest.join("index.js"), index_js)?;

    println!(
        "{}Sample module generated in ./{}",
        PREFIX_MSG_MOD,
        dest.display()
    );
    Ok(dest)
}
