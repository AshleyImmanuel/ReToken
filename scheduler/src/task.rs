use std::process::Stdio;
use tokio::process::Command;
use retoken_core::config::SandboxMode;
use crate::virtualization::slice_log;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Success,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub command: String,
    pub status: TaskStatus,
    pub output: String,
    pub sandbox_mode: SandboxMode,
}

impl Task {
    pub fn new(id: String, command: String, sandbox_mode: SandboxMode) -> Self {
        Self {
            id,
            command,
            status: TaskStatus::Pending,
            output: String::new(),
            sandbox_mode,
        }
    }

    /// Executes the task asynchronously.
    /// In a production ReToken environment, SandboxMode::Virtual would use an isolated container.
    pub async fn execute(&mut self) -> Result<(), String> {
        self.status = TaskStatus::Running;
        
        let mut cmd_parts = self.command.split_whitespace();
        let program = cmd_parts.next().ok_or("Empty command")?;
        
        // Native execution for MVP
        let child = match Command::new(program)
            .args(cmd_parts)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                self.status = TaskStatus::Failed;
                self.output = format!("Failed to spawn command: {}", e);
                return Err(e.to_string());
            }
        };

        // Wait for the process to finish
        match child.wait_with_output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                
                let raw_output = format!("{}\n{}", stdout, stderr).trim().to_string();
                
                // VIRTUALIZE LOGS: slice to max 1000 lines (500 head, 500 tail) to save tokens
                self.output = slice_log(&raw_output, 1000, 500, 500);
                
                if output.status.success() {
                    self.status = TaskStatus::Success;
                } else {
                    self.status = TaskStatus::Failed;
                }
                
                Ok(())
            }
            Err(e) => {
                self.status = TaskStatus::Failed;
                self.output = format!("Failed to wait on child: {}", e);
                Err(e.to_string())
            }
        }
    }
}
