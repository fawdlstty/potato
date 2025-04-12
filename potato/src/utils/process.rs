use anyhow::anyhow;
use tokio::process::Command;

pub struct ProgramRunner {}

impl ProgramRunner {
    pub async fn run_until_exit(cmd_str: &str) -> anyhow::Result<String> {
        // let (program, argument) = cmd.split_once(' ').unwrap_or((cmd, ""));
        // let mut cmd = Command::new(program);
        // let args: Vec<&str> = argument.split(' ').collect();
        // if args.len() > 0 && args[0] != "" {
        //     cmd.args(&args);
        // }
        let shell = std::env::var("SHELL").unwrap_or("/usr/bin/sh".to_string());
        let mut cmd = Command::new(&shell);
        cmd.args(vec!["-c", cmd_str]);
        let output = cmd.output().await?;
        let out_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let err_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let ret_str = format!("{out_str}\n{err_str}");
        match err_str.is_empty() {
            true => Ok(ret_str),
            false => Err(anyhow!(ret_str)),
        }
    }
}
