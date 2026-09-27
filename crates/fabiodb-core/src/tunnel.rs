//! SSH tunnels through the system `ssh`, so the user's keys, agent and
//! `~/.ssh/config` (aliases, ProxyJump) just work. Batch mode: a password
//! prompt would hang invisibly, so key or agent logins only.
//!
//! Connections to the same server through the same SSH host share one tunnel;
//! it closes (ssh is killed) when the last of them is dropped.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SshTunnel {
    /// A host name or an alias from ~/.ssh/config.
    pub host: String,
    /// When not 22 (or what ~/.ssh/config says).
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    /// A private key file; otherwise ssh's defaults and agent.
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
}

pub(crate) struct Tunnel {
    /// Killed on drop.
    _ssh: Child,
    pub local_port: u16,
}

static TUNNELS: LazyLock<Mutex<HashMap<String, Weak<Tunnel>>>> = LazyLock::new(Mutex::default);

const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// A local port forwarded to `host:port` on the far side of `ssh`.
pub(crate) async fn open(ssh: &SshTunnel, host: &str, port: u16) -> Result<Arc<Tunnel>> {
    validate_destination(ssh)?;
    let key = format!("{ssh:?} → {host}:{port}");
    if let Some(tunnel) = TUNNELS.lock().expect("tunnels poisoned").get(&key).and_then(Weak::upgrade) {
        return Ok(tunnel);
    }

    let local_port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    // FABIO_SSH swaps the program, for tests.
    let mut command = Command::new(std::env::var_os("FABIO_SSH").unwrap_or_else(|| "ssh".into()));
    command
        .args(["-N", "-L", &format!("127.0.0.1:{local_port}:{host}:{port}")])
        .args(["-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes"])
        .args(["-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=30"]);
    if let Some(p) = ssh.port {
        command.args(["-p", &p.to_string()]);
    }
    if let Some(key_file) = &ssh.identity_file {
        command.arg("-i").arg(key_file);
    }
    let destination = match &ssh.user {
        Some(user) => format!("{user}@{}", ssh.host),
        None => ssh.host.clone(),
    };
    // Stop a host such as `-oProxyCommand=...` from becoming an ssh option.
    command.arg("--").arg(destination);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| Error::Invalid(format!("Couldn't start ssh: {e}")))?;

    // Ready once the forwarded port accepts; ssh only listens after logging in.
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            let mut stderr = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                let _ = pipe.read_to_string(&mut stderr).await;
            }
            return Err(Error::Invalid(format!("SSH to {} failed: {}", ssh.host, stderr.trim())));
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
            break;
        }
        if started.elapsed() > READY_TIMEOUT {
            return Err(Error::Invalid(format!(
                "SSH to {} didn't open the tunnel within {} s.",
                ssh.host,
                READY_TIMEOUT.as_secs()
            )));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let tunnel = Arc::new(Tunnel { _ssh: child, local_port });
    TUNNELS.lock().expect("tunnels poisoned").insert(key, Arc::downgrade(&tunnel));
    Ok(tunnel)
}

fn validate_destination(ssh: &SshTunnel) -> Result<()> {
    let safe = |value: &str| !value.is_empty() && !value.starts_with('-') && !value.chars().any(char::is_whitespace);
    if !safe(&ssh.host) || ssh.user.as_deref().is_some_and(|u| !safe(u) || u.contains('@')) {
        return Err(Error::Invalid("SSH host or user contains characters that are not valid in a destination.".into()));
    }
    Ok(())
}
