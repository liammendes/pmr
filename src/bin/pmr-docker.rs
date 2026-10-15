use anyhow::Result;
use std::env;
use std::process::Command;

fn main() -> Result<()> {
    let exe = env::current_exe()?.parent().unwrap().join("pmr-runtime");
    let mut cmd = Command::new(exe);
    cmd.args(env::args().skip(1));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        Err(err.into())
    }

    #[cfg(not(unix))]
    {
        let status = cmd.status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}
