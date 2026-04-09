//! Robust, cross-platform external process management
//!
//! This module provides a unified way to spawn and manage external processes
//! as process groups (Unix) or Job Objects (Windows). This ensures that
//! when a process is killed, all of its children are also terminated.

use process_wrap::tokio::CommandWrap;
use std::process::ExitStatus;
use tokio::process::Command;

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

/// A handle to a spawned external process group (Asynchronous)
pub struct ExternalProcess {
    pub child: Box<dyn process_wrap::tokio::ChildWrapper>,
}

impl ExternalProcess {
    /// Spawn a new command in a process group (Unix) or Job Object (Windows)
    pub fn spawn(program: &str, args: &[&str]) -> std::io::Result<Self> {
        let mut cmd = Command::new(program);
        for arg in args {
            cmd.arg(arg);
        }

        let child: Box<dyn process_wrap::tokio::ChildWrapper> = {
            #[cfg(unix)]
            {
                CommandWrap::from(cmd)
                    .wrap(ProcessGroup::leader())
                    .spawn()?
            }
            #[cfg(windows)]
            {
                CommandWrap::from(cmd).wrap(JobObject).spawn()?
            }
            #[cfg(not(any(unix, windows)))]
            {
                CommandWrap::from(cmd).spawn()?
            }
        };

        Ok(Self { child })
    }

    /// Kill the entire process group
    pub async fn kill(&mut self) -> std::io::Result<()> {
        std::pin::Pin::from(self.child.kill()).await
    }

    /// Wait for the process to complete
    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        std::pin::Pin::from(self.child.wait()).await
    }

    /// Get the process ID of the leader
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }
}

/// RAII guard that kills the process group when dropped
pub struct ChildGuard {
    pub process: Option<ExternalProcess>,
}

impl ChildGuard {
    pub fn new(process: ExternalProcess) -> Self {
        Self {
            process: Some(process),
        }
    }

    /// Disarm the guard so the process continues running after drop
    pub fn disarm(&mut self) -> Option<ExternalProcess> {
        self.process.take()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            // Drop can't be async, so we spawn a task to kill the process group
            tokio::spawn(async move {
                let _ = process.kill().await;
            });
        }
    }
}
