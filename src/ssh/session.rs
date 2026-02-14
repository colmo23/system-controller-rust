use anyhow::{Context, Result};
use openssh::{KnownHosts, Session};
use std::collections::HashMap;

pub struct SessionManager {
    sessions: HashMap<String, Session>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub async fn get_session(&mut self, host: &str) -> Result<&Session> {
        if !self.sessions.contains_key(host) {
            let session = Session::connect_mux(format!("ssh://{}", host), KnownHosts::Accept)
                .await
                .with_context(|| format!("Failed to connect to {}", host))?;
            self.sessions.insert(host.to_string(), session);
        }
        Ok(self.sessions.get(host).unwrap())
    }

    pub async fn run_command(&mut self, host: &str, cmd: &str) -> Result<String> {
        let session = self.get_session(host).await?;
        let output = session
            .shell(cmd)
            .output()
            .await
            .with_context(|| format!("Failed to run command on {}: {}", host, cmd))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stderr.is_empty() {
                anyhow::bail!("Command failed on {}: {}", host, stderr.trim())
            } else if !stdout.is_empty() {
                // Some commands like systemctl is-active return non-zero but have useful stdout
                Ok(stdout.to_string())
            } else {
                anyhow::bail!("Command failed on {} with exit code: {:?}", host, output.status)
            }
        }
    }

    pub async fn close_all(&mut self) {
        for (_, session) in self.sessions.drain() {
            let _ = session.close().await;
        }
    }
}
