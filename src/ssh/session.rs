use anyhow::{Context, Result};
use openssh::{KnownHosts, Session};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::timeout;

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
            log::info!("Opening SSH connection to {}", host);
            let session = timeout(
                Duration::from_secs(2),
                Session::connect_mux(format!("ssh://{}", host), KnownHosts::Accept),
            )
            .await
            .with_context(|| {
                log::error!("SSH connection to {} timed out after 2s", host);
                format!("Connection to {} timed out after 2s", host)
            })?
            .with_context(|| {
                log::error!("SSH connection to {} failed", host);
                format!("Failed to connect to {}", host)
            })?;
            log::info!("SSH connection to {} established", host);
            self.sessions.insert(host.to_string(), session);
        }
        Ok(self.sessions.get(host).unwrap())
    }

    pub async fn run_command(&mut self, host: &str, cmd: &str) -> Result<String> {
        log::debug!("Running command on {}: {}", host, cmd);
        let session = self.get_session(host).await?;
        let output = session
            .shell(cmd)
            .output()
            .await
            .with_context(|| {
                log::error!("Command execution failed on {}: {}", host, cmd);
                format!("Failed to run command on {}: {}", host, cmd)
            })?;

        if output.status.success() {
            log::debug!("Command succeeded on {}: {}", host, cmd);
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stderr.is_empty() {
                log::warn!("Command failed on {}: {} — {}", host, cmd, stderr.trim());
                anyhow::bail!("Command failed on {}: {}", host, stderr.trim())
            } else if !stdout.is_empty() {
                // Some commands like systemctl is-active return non-zero but have useful stdout
                log::debug!("Command exited non-zero on {} (has stdout): {}", host, cmd);
                Ok(stdout.to_string())
            } else {
                log::warn!("Command failed on {} with exit code {:?}: {}", host, output.status, cmd);
                anyhow::bail!("Command failed on {} with exit code: {:?}", host, output.status)
            }
        }
    }

    pub async fn close_all(&mut self) {
        let count = self.sessions.len();
        if count > 0 {
            log::debug!("Closing {} SSH sessions", count);
        }
        for (host, session) in self.sessions.drain() {
            log::debug!("Closing SSH session to {}", host);
            let _ = session.close().await;
        }
    }
}
