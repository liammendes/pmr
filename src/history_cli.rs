use crate::client::PmrClient;
use crate::history::{CommandSpec, Query, Request, SettingsPatch};
use crate::protocol::RpcRequest;
use anyhow::{anyhow, Result};
use clap::Subcommand;
use serde_json::Value;
use std::{env, path::PathBuf, time::Duration};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List indexed command history, newest first
    List {
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// Continue after next_cursor from the previous page
        #[arg(long)]
        cursor: Option<i64>,
        /// SQLite FTS5 query over the command, name and namespace
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    /// Inspect a command by its durable UUID
    Show { id: String },
    /// Read retained output from pmr run
    Logs {
        id: String,
        #[arg(long, default_value_t = 100)]
        lines: usize,
    },
    /// Cancel a queued or running command
    Cancel { id: String },
    /// Apply the configured expiry policy in a bounded batch
    Prune {
        #[arg(long, default_value_t = 100)]
        batch: usize,
    },
}

pub async fn request(client: &PmrClient, request: Request) -> Result<Value> {
    let response = client.request(&RpcRequest::History { request }).await?;
    if !response.ok {
        let error = response
            .error
            .unwrap_or_else(|| "history request failed".into());
        if error.contains("unknown variant") {
            return Err(anyhow!("This daemon predates command history. Run pmr update when it is safe to restart your managed processes."));
        }
        return Err(anyhow!(error));
    }
    Ok(response.data)
}

pub async fn history(
    client: &PmrClient,
    action: &Option<Action>,
    name: Option<String>,
    namespace: Option<String>,
    ascending: bool,
) -> Result<()> {
    let query = match action {
        Some(Action::List {
            limit,
            cursor,
            search,
            status,
        }) => Query {
            limit: *limit,
            cursor: *cursor,
            search: search.clone(),
            status: status.clone(),
            name,
            namespace,
            ascending,
        },
        _ => Query {
            name,
            namespace,
            ascending,
            ..Query::default()
        },
    };
    let req = match action {
        Some(Action::Show { id }) => Request::Get { id: id.clone() },
        Some(Action::Logs { id, lines }) => Request::Output {
            stream: None,
            id: id.clone(),
            lines: *lines,
        },
        Some(Action::Cancel { id }) => Request::Cancel { id: id.clone() },
        Some(Action::Prune { batch }) => Request::Prune { batch: *batch },
        _ => Request::List(query),
    };
    let value = request(client, req).await?;
    if matches!(action, Some(Action::Logs { .. })) {
        println!("{}", value["text"].as_str().unwrap_or_default());
        if value["truncated"] == true {
            eprintln!("Output is a retained tail; older bytes or lines are omitted.");
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

pub async fn configure(client: &PmrClient, patch: SettingsPatch) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&request(client, Request::Configure(patch)).await?)?
    );
    Ok(())
}

pub struct RunOptions {
    pub program: String,
    pub args: Vec<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub cwd: Option<String>,
    pub interpreter: Option<String>,
    pub wait: bool,
}

pub async fn run(client: &PmrClient, options: RunOptions) -> Result<()> {
    let id = options
        .id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    uuid::Uuid::parse_str(&id)?;
    let cwd = match options.cwd {
        Some(path) => PathBuf::from(path).canonicalize()?,
        None => env::current_dir()?,
    };
    let name = options.name.unwrap_or_else(|| options.program.clone());
    let (program, args) = match options.interpreter.filter(|s| s != "none") {
        Some(interpreter) => (
            interpreter,
            std::iter::once(options.program)
                .chain(options.args)
                .collect(),
        ),
        None => (options.program, options.args),
    };
    let row = request(
        client,
        Request::Submit(CommandSpec {
            id: id.clone(),
            program,
            args,
            cwd: cwd.to_string_lossy().into_owned(),
            name,
            namespace: options.namespace.unwrap_or_else(|| "default".into()),
            env: env::vars().collect(),
        }),
    )
    .await?;
    if !options.wait {
        println!("{}", serde_json::to_string(&row)?);
        return Ok(());
    }
    loop {
        let row = request(client, Request::Get { id: id.clone() }).await?;
        if !row["finished_at"].is_null() {
            let output = request(
                client,
                Request::Output {
                    stream: None,
                    id: id.clone(),
                    lines: 100_000,
                },
            )
            .await?;
            let text = output["text"].as_str().unwrap_or_default();
            if !text.is_empty() {
                println!("{text}");
            }
            if output["truncated"] == true {
                eprintln!("Output is a retained tail; older bytes are omitted.");
            }
            if row["status"] != "succeeded" {
                let code = row["exit_code"]
                    .as_i64()
                    .filter(|v| (1..=255).contains(v))
                    .unwrap_or(1);
                eprintln!("Command {id}: {}", row["status"]);
                std::process::exit(code as i32);
            }
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
