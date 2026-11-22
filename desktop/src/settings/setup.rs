use anyhow::{Context as _, Result, ensure};
use async_process::Command;
use futures_lite::future::race;
use gpui::BackgroundExecutor;
use huggler::{Provider, ProviderInstance};
use semver::Version;
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) const CODEX_GUIDE: &str = "https://github.com/openai/codex#installation";
pub(super) const CODEX_AUTH_GUIDE: &str = "https://developers.openai.com/codex/auth";
pub(super) const CLAUDE_GUIDE: &str =
    "https://github.com/agentclientprotocol/claude-agent-acp#standalone-executables";
pub(super) const CLAUDE_AUTH_GUIDE: &str = "https://code.claude.com/docs/en/agent-sdk/quickstart";

pub fn executable_exists(path: &Path) -> bool {
    if path.is_absolute() {
        return is_executable(path);
    }
    on_path(path).is_some()
}

pub(super) fn on_path(name: &Path) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    fs::metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && {
            #[cfg(unix)]
            {
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
    })
}

fn desktop_path(name: &Path) -> Option<PathBuf> {
    dirs::home_dir()
        .into_iter()
        .map(|home| home.join(".local/bin"))
        .chain([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ])
        .map(|dir| dir.join(name))
        .find(|path| is_executable(path))
}

pub fn default_executable(state: &Path, provider: Provider, name: &Path) -> PathBuf {
    let managed = Release::for_provider(provider).map(|release| release.executable(state));
    if let Some(path) = &managed
        && is_executable(path)
    {
        return path.clone();
    }
    if executable_exists(name) {
        return name.to_owned();
    }
    desktop_path(name)
        .or(managed)
        .unwrap_or_else(|| name.to_owned())
}

#[derive(Clone, Copy)]
pub(super) struct Release {
    version: &'static str,
    url: &'static str,
    sha256: &'static str,
    size: u64,
    directory: &'static str,
    program: &'static str,
    companion: &'static str,
}

impl Release {
    pub(super) fn for_provider(provider: Provider) -> Option<Self> {
        let (version, url, sha256, size) = match (
            provider,
            std::env::consts::OS,
            std::env::consts::ARCH,
        ) {
            (Provider::Codex, "macos", "aarch64") => (
                "0.170.0",
                "https://github.com/openai/codex/releases/download/rust-v0.170.0/codex-package-aarch64-apple-darwin.tar.gz",
                "975524bf13aedd1b122d8d0e00c09ee67091b8f688c0b374943dbb3cf22c7893",
                173593904,
            ),
            (Provider::Codex, "macos", "x86_64") => (
                "0.170.0",
                "https://github.com/openai/codex/releases/download/rust-v0.170.0/codex-package-x86_64-apple-darwin.tar.gz",
                "2e971524460a570b913dbdc9b49e5e9c14556e44e7f62f00b95b4a8f1e2cdf25",
                188621592,
            ),
            (Provider::ClaudeAgent, "macos", "aarch64") => (
                "0.109.0",
                "https://github.com/agentclientprotocol/claude-agent-acp/releases/download/v0.109.0/claude-agent-acp-darwin-arm64.zip",
                "603fdb593f00335a996986f9ed9f8a84928dad4b20e08b25862782fd770b2a28",
                198976966,
            ),
            (Provider::ClaudeAgent, "macos", "x86_64") => (
                "0.109.0",
                "https://github.com/agentclientprotocol/claude-agent-acp/releases/download/v0.109.0/claude-agent-acp-darwin-x64.zip",
                "c8d7f0c4f11c951656c1c87add9faffd8036cd8cbcd9658057e3a6774535e4f2",
                209935200,
            ),
            _ => return None,
        };
        let (directory, program, companion) = match provider {
            Provider::Codex => ("codex", "bin/codex", "bin/codex-code-mode-host"),
            Provider::ClaudeAgent => ("claude-agent", "claude-agent-acp", "claude"),
            _ => unreachable!(),
        };
        Some(Self {
            version,
            url,
            sha256,
            size,
            directory,
            program,
            companion,
        })
    }

    fn executable(self, state: &Path) -> PathBuf {
        state
            .join("providers")
            .join(self.directory)
            .join(self.program)
    }

    pub(super) fn for_instance(state: &Path, instance: &ProviderInstance) -> Option<Self> {
        Self::for_provider(instance.driver)
            .filter(|release| release.executable(state) == instance.executable)
    }

    pub(super) fn update_version(self, current: &Version) -> Option<&'static str> {
        let available = Version::parse(self.version).expect("managed release version");
        available
            .cmp_precedence(current)
            .is_gt()
            .then_some(self.version)
    }

    pub(super) fn install(self, state: &Path, update: bool) -> Result<()> {
        let providers = state.join("providers");
        fs::create_dir_all(&providers).context("Could not open the provider folder.")?;
        let stage = tempfile::tempdir_in(&providers).context("Could not prepare the download.")?;
        let archive = stage.path().join("release");
        let download = std::process::Command::new("/usr/bin/curl")
            .args([
                "-q",
                "--fail",
                "--silent",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "15",
                "--max-time",
                "300",
                "--max-filesize",
            ])
            .arg(self.size.to_string())
            .arg("--output")
            .arg(&archive)
            .arg(self.url)
            .status()
            .context("Could not start the download.")?;
        ensure!(
            download.success(),
            "Download failed. Check your connection and try again."
        );
        self.install_archive(&archive, stage.path(), &providers, update)
    }

    fn install_archive(
        self,
        archive: &Path,
        stage: &Path,
        providers: &Path,
        update: bool,
    ) -> Result<()> {
        use sha2::{Digest, Sha256};

        let mut file = File::open(archive).context("Could not read the download.")?;
        let mut hash = Sha256::new();
        let length = io::copy(&mut file, &mut hash).context("Could not verify the download.")?;
        ensure!(
            length == self.size && format!("{:x}", hash.finalize()) == self.sha256,
            "The download did not match the published release. Try again."
        );

        let unpacked = stage.join("runtime");
        fs::create_dir(&unpacked).context("Could not prepare the provider folder.")?;
        // The digest pins the entire vendor archive. Keep its companion binaries and resources together.
        let status = std::process::Command::new("/usr/bin/tar")
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(&unpacked)
            .output()
            .context("Could not unpack the download.")?;
        ensure!(
            status.status.success(),
            "Could not unpack the download. Check available disk space."
        );
        ensure!(
            is_executable(&unpacked.join(self.program))
                && is_executable(&unpacked.join(self.companion)),
            "The download is missing required files."
        );
        let destination = providers.join(self.directory);
        if update {
            return swap_release(&unpacked, &destination)
                .context("Could not replace the provider files.");
        }
        if let Err(error) = fs::rename(&unpacked, &destination) {
            // Another Huggler window may have published the same complete release.
            if !is_executable(&destination.join(self.program))
                || !is_executable(&destination.join(self.companion))
            {
                return Err(error).context("Could not save the provider files.");
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn swap_release(staged: &Path, installed: &Path) -> io::Result<()> {
    use std::ffi::CString;
    let staged = CString::new(staged.as_os_str().as_encoded_bytes())?;
    let installed = CString::new(installed.as_os_str().as_encoded_bytes())?;
    // Both directories were staged on the same filesystem. Swap the complete packages atomically.
    if unsafe { libc::renamex_np(staged.as_ptr(), installed.as_ptr(), libc::RENAME_SWAP) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn swap_release(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

pub(super) async fn version(
    instance: &ProviderInstance,
    executor: &BackgroundExecutor,
) -> Option<Version> {
    let mut command = Command::new(&instance.executable);
    command.arg("--version").envs(instance.environment());
    let output = race(
        async { command.kill_on_drop(true).output().await.ok() },
        async {
            executor.timer(Duration::from_secs(2)).await;
            None
        },
    )
    .await?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter(|word| word.len() <= 64)
        .find_map(|word| Version::parse(word.trim_start_matches('v')).ok())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Account {
    SignedIn,
    SignedOut,
    Unknown,
}

fn codex_command(instance: &ProviderInstance) -> Command {
    let mut command = Command::new(&instance.executable);
    command.args(&instance.args).envs(instance.environment());
    if let Some(home) = &instance.home {
        command.env("CODEX_HOME", home);
    }
    command
}

fn claude_command(instance: &ProviderInstance) -> Option<Command> {
    let cli = if let Some(path) = instance
        .environment()
        .get("CLAUDE_CODE_EXECUTABLE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CLAUDE_CODE_EXECUTABLE").map(PathBuf::from))
    {
        path
    } else {
        let adapter = if instance.executable.is_absolute() {
            Some(instance.executable.clone())
        } else {
            on_path(&instance.executable)
        };
        adapter
            .and_then(|path| fs::canonicalize(path).ok())
            .map(|path| path.with_file_name("claude"))
            .filter(|path| is_executable(path))
            .or_else(|| on_path(Path::new("claude")))
            .or_else(|| desktop_path(Path::new("claude")))?
    };
    let mut command = Command::new(cli);
    command.envs(instance.environment());
    if let Some(home) = &instance.home {
        command.env("CLAUDE_CONFIG_DIR", home);
    }
    Some(command)
}

pub(super) async fn account(instance: &ProviderInstance, executor: &BackgroundExecutor) -> Account {
    let mut command = match instance.driver {
        Provider::Codex => {
            let mut command = codex_command(instance);
            command.args(["login", "status"]);
            command
        }
        Provider::ClaudeAgent => {
            let Some(mut command) = claude_command(instance) else {
                return Account::Unknown;
            };
            command.args(["auth", "status", "--json"]);
            command
        }
        _ => return Account::Unknown,
    };
    let output = race(
        async { command.kill_on_drop(true).output().await.ok() },
        async {
            executor.timer(Duration::from_secs(8)).await;
            None
        },
    )
    .await;
    let Some(output) = output else {
        return Account::Unknown;
    };
    if instance.driver == Provider::ClaudeAgent {
        return match serde_json::from_slice::<serde_json::Value>(&output.stdout)
            .ok()
            .and_then(|status| status["loggedIn"].as_bool())
        {
            Some(true) => Account::SignedIn,
            Some(false) => Account::SignedOut,
            None => Account::Unknown,
        };
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        if stdout
            .lines()
            .chain(stderr.lines())
            .map(str::trim)
            .any(|line| {
                line.starts_with("Logged in using ") || line.starts_with("API key configured")
            })
        {
            Account::SignedIn
        } else {
            Account::Unknown
        }
    } else if stdout
        .lines()
        .chain(stderr.lines())
        .any(|line| line.trim() == "Not logged in")
    {
        Account::SignedOut
    } else {
        Account::Unknown
    }
}

pub(super) fn codex_login_command(instance: &ProviderInstance) -> String {
    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }

    let mut command = Vec::new();
    if let Some(home) = &instance.home {
        command.push(format!("CODEX_HOME={}", quote(&home.to_string_lossy())));
    }
    command.push(quote(&instance.executable.to_string_lossy()));
    command.extend(instance.args.iter().map(|arg| quote(arg)));
    command.push("login".into());
    command.join(" ")
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn provider_update_swaps_complete_directories() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let installed = temp.path().join("installed");
        let staged = temp.path().join("staged");
        fs::create_dir(&installed)?;
        fs::create_dir(&staged)?;
        fs::write(installed.join("provider"), "old executable")?;
        fs::write(staged.join("provider"), "new executable")?;
        fs::write(staged.join("companion"), "new companion")?;

        swap_release(&staged, &installed)?;

        assert_eq!(
            fs::read_to_string(installed.join("provider"))?,
            "new executable"
        );
        assert_eq!(
            fs::read_to_string(installed.join("companion"))?,
            "new companion"
        );
        assert_eq!(
            fs::read_to_string(staged.join("provider"))?,
            "old executable"
        );
        Ok(())
    }
}
