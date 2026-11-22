use anyhow::{Context, Result, bail, ensure};
use huggler::{
    EnvironmentId, ProjectId,
    machine::{Client, Reply, Request},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, process::Stdio, time::Duration};
use tokio::process::{Child, Command};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Target {
    pub host: String,
    pub port: Option<u16>,
}

impl Target {
    pub fn validate(&self) -> Result<()> {
        let (user, host) = match self.host.split_once('@') {
            Some((user, host)) => (Some(user), host),
            None => (None, self.host.as_str()),
        };
        let name = |value: &str| {
            value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        };
        ensure!(
            user.is_none_or(name) && (name(host) || host.parse::<std::net::Ipv6Addr>().is_ok()),
            "Enter an SSH alias or a destination such as user@buildbox."
        );
        ensure!(self.port != Some(0), "Enter a port between 1 and 65535.");
        Ok(())
    }

    fn command(&self) -> Command {
        let mut ssh = Command::new("/usr/bin/ssh");
        #[cfg(all(feature = "qualification", target_os = "macos"))]
        if crate::qualification::scripted()
            && let Some(config) = std::env::var_os("HUGGLER_UI_SSH_CONFIG")
        {
            ssh.arg("-F").arg(config);
        }
        ssh.args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ForwardAgent=no",
            "-o",
            "ForwardX11=no",
            "-o",
            "PermitLocalCommand=no",
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            "-o",
            "ForkAfterAuthentication=no",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "ServerAliveInterval=3",
            "-o",
            "ServerAliveCountMax=2",
        ]);
        if let Some(port) = self.port {
            ssh.args(["-p", &port.to_string()]);
        }
        ssh.kill_on_drop(true)
            .stdin(Stdio::null())
            .env_remove("SSH_ASKPASS");
        ssh
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedMachine {
    pub target: Target,
    pub environment: EnvironmentId,
    pub project: ProjectId,
    pub checkout: std::path::PathBuf,
}

pub(super) struct Connection {
    pub client: Client,
    pub environment: EnvironmentId,
    _ssh: Child,
    _directory: tempfile::TempDir,
}

impl Connection {
    pub async fn open(target: &Target, expected: Option<&EnvironmentId>) -> Result<Self> {
        target.validate()?;
        let output = tokio::time::timeout(
            Duration::from_secs(12),
            target
                .command()
                .args([
                    "-o",
                    "ClearAllForwardings=yes",
                    "--",
                    &target.host,
                    "hugglerd socket-path",
                ])
                .output(),
        )
        .await
        .context("SSH did not return the machine socket path in time.")??;
        if !output.status.success() {
            bail!(
                "{}",
                diagnostic(
                    &output.stderr,
                    "SSH could not find hugglerd on the remote PATH."
                )
            );
        }
        // Read the machine-readable last line so a login banner cannot be mistaken for a path.
        let output = std::str::from_utf8(&output.stdout)?;
        let remote: String = serde_json::from_str(output.lines().last().unwrap_or(""))
            .context("hugglerd did not return a socket path.")?;
        ensure!(
            Path::new(&remote).is_absolute() && !remote.contains([':', '\n', '\r', '\0']),
            "The machine socket path cannot be forwarded over SSH."
        );
        let directory = tempfile::Builder::new().prefix("hg-").tempdir_in("/tmp")?;
        let socket = directory.path().join("control.sock");
        let forward = format!("{}:{remote}", socket.display());
        let log_path = directory.path().join("ssh.log");
        let log = fs::File::create(&log_path)?;
        let mut ssh = target
            .command()
            .args([
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StreamLocalBindMask=0177",
                "-L",
                &forward,
                "--",
                &target.host,
            ])
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()?;
        let ready = async {
            let client = Client::new(&socket)?;
            loop {
                if ssh.try_wait()?.is_some() {
                    bail!(
                        "{}",
                        diagnostic(&fs::read(&log_path)?, "SSH closed the connection.")
                    );
                }
                if socket.exists() {
                    match client.request(Request::Identify).await {
                        Ok(Reply::Machine(machine)) => return Ok((client, machine.id)),
                        Ok(_) => bail!("The machine returned an incompatible response."),
                        Err(huggler::machine::ClientError::Connection(_)) => (),
                        Err(error) => return Err(error.into()),
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        };
        let (client, environment) = tokio::time::timeout(Duration::from_secs(10), ready)
            .await
            .with_context(|| {
                diagnostic(
                    &fs::read(&log_path).unwrap_or_default(),
                    "Huggler did not respond. Check that hugglerd serve is running on the machine.",
                )
            })??;
        ensure!(
            expected.is_none_or(|expected| expected == &environment),
            "This SSH target reaches a different Huggler environment."
        );
        Ok(Self {
            client: client.for_environment(environment.clone()),
            environment,
            _ssh: ssh,
            _directory: directory,
        })
    }
}

fn diagnostic(stderr: &[u8], fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        fallback.into()
    } else {
        detail.chars().take(500).collect()
    }
}
