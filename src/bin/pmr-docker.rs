use anyhow::Result;
use std::env;
use std::process::Command;

fn main() -> Result<()> {
    let exe = env::current_exe()?
        .parent()
        .unwrap()
        .join("pmr-runtime");
    let status = Command::new(exe).args(env::args().skip(1)).status()?;
    std::process::exit(status.code().unwrap_or(0));
}
