use huggler::{HugglerOptions, ProviderInstance, ToolPresentation, WorktreeRoot};
use serde::Deserialize;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    state_dir: Option<PathBuf>,
    name: Option<String>,
    slug: Option<String>,
    #[serde(default)]
    storage: Storage,
    #[serde(default)]
    instances: Vec<ProviderInstance>,
    #[serde(default)]
    output: Output,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Storage {
    database_path: Option<PathBuf>,
    worktree_root: Option<Root>,
    worktree_name: Option<String>,
    worktree_words: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    content = "path",
    rename_all = "camelCase",
    deny_unknown_fields
)]
enum Root {
    Shared(PathBuf),
    ProjectRelative(PathBuf),
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Output {
    name: Option<String>,
    title: Option<String>,
    version: Option<String>,
    instructions: Option<String>,
    #[serde(default)]
    provider_labels: BTreeMap<String, String>,
    #[serde(default)]
    tools: BTreeMap<String, Presentation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Presentation {
    name: Option<String>,
    title: Option<String>,
    description: Option<String>,
}

impl Options {
    pub fn into_core(self) -> huggler::Result<HugglerOptions> {
        if self
            .state_dir
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(huggler::Error::Invalid("stateDir must be a path".into()));
        }
        let mut core = self.state_dir.map(HugglerOptions::new).unwrap_or_default();
        if let Some(slug) = self.slug {
            core = core.with_name_and_slug(self.name.as_deref().unwrap_or(&slug), &slug)?;
        } else if let Some(name) = self.name {
            core = core.with_name(&name)?;
        } else if core.state_dir.as_os_str().is_empty() {
            return Err(huggler::Error::Invalid(
                "provide name, slug or stateDir".into(),
            ));
        }
        core.instances = self.instances;
        if let Some(path) = self.storage.database_path {
            core.storage.database_path = path;
        }
        if let Some(root) = self.storage.worktree_root {
            core.storage.worktree_root = match root {
                Root::Shared(path) => WorktreeRoot::Shared(path),
                Root::ProjectRelative(path) => WorktreeRoot::ProjectRelative(path),
            };
        }
        if let Some(words) = self.storage.worktree_words {
            core.storage.worktree_words = words;
        }
        if let Some(name) = self.storage.worktree_name {
            core.storage.worktree_name = name;
        }
        let output = &mut core.output;
        if let Some(name) = self.output.name {
            output.name = name;
        }
        if let Some(title) = self.output.title {
            output.title = title;
        }
        if let Some(version) = self.output.version {
            output.version = version;
        }
        output.instructions = self.output.instructions;
        output.provider_labels.extend(self.output.provider_labels);
        for (key, tool) in self.output.tools {
            output.tools.insert(
                key,
                ToolPresentation {
                    name: tool.name,
                    title: tool.title,
                    description: tool.description,
                },
            );
        }
        Ok(core)
    }
}
