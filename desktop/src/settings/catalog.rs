use super::{Preferences, setup};
use huggler::{HugglerOptions, Provider, ProviderInstance, ProviderInstanceId};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub(super) struct ProviderPreset {
    pub driver: Provider,
    pub name: &'static str,
    pub executable: &'static str,
    pub args: &'static [&'static str],
    pub guide: &'static str,
}

pub(super) const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        driver: Provider::Cursor,
        name: "Cursor",
        executable: "agent",
        args: &["acp"],
        guide: "https://cursor.com/docs/cli/installation",
    },
    ProviderPreset {
        driver: Provider::Grok,
        name: "Grok Build",
        executable: "grok",
        args: &["--permission-mode", "default", "agent", "stdio"],
        guide: "https://docs.x.ai/build/overview#install",
    },
    ProviderPreset {
        driver: Provider::Fx,
        name: "fx",
        executable: "fx",
        args: &["acp"],
        guide: "https://fx.sh/docs/getting-started/installation",
    },
    ProviderPreset {
        driver: Provider::Kiro,
        name: "Kiro",
        executable: "kiro-cli",
        args: &["acp", "--agent-engine=v3", "--auth-method=cli"],
        guide: "https://kiro.dev/docs/getting-started/installation/",
    },
    ProviderPreset {
        driver: Provider::OpenCode,
        name: "OpenCode",
        executable: "opencode",
        args: &[],
        guide: "https://opencode.ai/docs/",
    },
    ProviderPreset {
        driver: Provider::Antigravity,
        name: "Antigravity",
        executable: "agy",
        args: &["acp"],
        guide: "https://antigravity.google/docs/ide/extensions/acp/#before-you-begin",
    },
    ProviderPreset {
        driver: Provider::Pi,
        name: "Pi",
        executable: "pi",
        args: &[],
        guide: "https://docs.pi.dev/latest/quickstart",
    },
    ProviderPreset {
        driver: Provider::Muse,
        name: "Muse Code",
        executable: "muse",
        args: &[],
        guide: "https://dev.meta.ai/docs/muse-code",
    },
];

impl ProviderPreset {
    pub(super) fn executable(&self, state: &Path) -> PathBuf {
        setup::default_executable(state, self.driver, Path::new(self.executable))
    }
}

pub(super) fn preset(driver: Provider) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|preset| preset.driver == driver)
}

pub(super) fn register(
    options: &mut HugglerOptions,
    prefs: &Preferences,
    state: &Path,
) -> HashSet<ProviderInstanceId> {
    let mut presets = HashSet::new();
    for preset in PRESETS {
        let id = preset.driver.key();
        if options
            .instances
            .iter()
            .any(|instance| instance.id.as_str() == id)
        {
            continue;
        }
        options
            .output
            .provider_labels
            .insert(id.into(), preset.name.into());
        let executable = prefs
            .provider_executables
            .get(id)
            .cloned()
            .unwrap_or_else(|| preset.executable(state));
        let mut instance = ProviderInstance::new(id, preset.driver, executable);
        instance.args = preset.args.iter().map(|arg| (*arg).into()).collect();
        presets.insert(instance.id.clone());
        options.instances.push(instance);
    }
    presets
}
