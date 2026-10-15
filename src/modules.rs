use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::client::PmrClient;
use crate::config::{AppConfig, StartOptions};
use crate::constants::{PREFIX_MSG_MOD, PREFIX_MSG_MOD_ERR};
use crate::process::ProcessDescription;

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
        let clean_name = module_spec
            .split('@')
            .next()
            .unwrap_or(module_spec)
            .rsplit('/')
            .next()
            .unwrap_or(module_spec);
        let dest = client.paths.default_module_path.join(clean_name);
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
        dest.join("node_modules").join(clean_name)
    };

    let pkg_json_path = mod_dir.join("package.json");
    let (mod_name, main_script) = if pkg_json_path.exists() {
        let raw = fs::read_to_string(&pkg_json_path)?;
        let val: Value = serde_json::from_str(&raw)?;
        let name = val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(module_spec)
            .to_string();
        let main = val
            .get("main")
            .and_then(|v| v.as_str())
            .unwrap_or("index.js")
            .to_string();
        (name, mod_dir.join(main))
    } else {
        (module_spec.to_string(), mod_dir.join("index.js"))
    };

    let app = AppConfig {
        script: main_script.to_string_lossy().to_string(),
        name: Some(mod_name.clone()),
        cwd: Some(mod_dir.to_string_lossy().to_string()),
        pmx_module: Some(true),
        ..Default::default()
    };

    let procs = client
        .start_app_config(app, opts, &BTreeMap::new(), Some(false))
        .await?;
    println!("{}Module {} successfully installed and launched", PREFIX_MSG_MOD, mod_name);
    Ok(procs)
}

pub async fn cmd_uninstall_module(
    client: &PmrClient,
    module_name: &str,
) -> Result<Vec<ProcessDescription>> {
    println!("{}Uninstalling module {}...", PREFIX_MSG_MOD, module_name);
    let _ = client.delete(module_name).await;
    let mod_dir = client.paths.default_module_path.join(module_name);
    if mod_dir.exists() {
        let _ = fs::remove_dir_all(&mod_dir);
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
