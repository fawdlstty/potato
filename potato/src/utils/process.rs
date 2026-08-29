use anyhow::anyhow;
use std::ffi::OsStr;
use tokio::process::Command;

pub struct ProgramRunner {}

impl ProgramRunner {
    pub async fn run_checked<I, S>(program: &str, args: I) -> anyhow::Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(program);
        cmd.args(args);
        let output = cmd.output().await?;
        let out_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let err_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let ret_str = format!("{out_str}\n{err_str}");
        match output.status.success() {
            true => Ok(ret_str),
            false => Err(anyhow!("exit status {:?}\n{ret_str}", output.status.code())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProgramRunner;

    #[tokio::test]
    async fn run_checked_includes_stderr_on_success() {
        #[cfg(windows)]
        let output = ProgramRunner::run_checked("cmd", ["/C", "echo stdout stderr"])
            .await
            .unwrap();
        #[cfg(not(windows))]
        let output = ProgramRunner::run_checked("echo", ["stdout", "stderr"])
            .await
            .unwrap();
        assert_eq!(output, "stdout stderr\n");
    }

    #[tokio::test]
    async fn run_checked_fails_only_for_nonzero_status() {
        #[cfg(windows)]
        let output = ProgramRunner::run_checked("cmd", ["/C", "exit 3"])
            .await
            .unwrap_err();
        #[cfg(not(windows))]
        let output = ProgramRunner::run_checked("sh", ["-c", "exit 3"])
            .await
            .unwrap_err();
        assert!(output.to_string().contains("exit status"));
    }
}
