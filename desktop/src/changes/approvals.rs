use crate::git::{self, ChangeScope, ChangesSnapshot, File};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Approval {
    pub scope: ChangeScope,
    pub fingerprint: String,
}

pub(super) type Approvals = BTreeMap<String, Approval>;
type Store = BTreeMap<String, Approvals>;

pub(super) fn path_key(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .escape_ascii()
        .to_string()
}

fn read_store(path: &Path) -> Result<Store> {
    match fs::read(path) {
        Ok(data) => Ok(serde_json::from_slice(&data)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Store::new()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn load(path: &Path, thread: &str) -> Result<Approvals> {
    Ok(read_store(path)?.remove(thread).unwrap_or_default())
}

pub(super) fn save(path: &Path, thread: &str, approvals: &Approvals) -> Result<()> {
    // Each conversation owns its marks. Update only that key in the desktop store.
    let mut stored = read_store(path)?;
    if approvals.is_empty() {
        stored.remove(thread);
    } else {
        stored.insert(thread.to_owned(), approvals.clone());
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().expect("desktop state directory"))?;
    serde_json::to_writer(temp.as_file_mut(), &stored)?;
    temp.flush()?;
    temp.persist(path)?;
    Ok(())
}

#[derive(Clone)]
pub(crate) struct ReviewRow {
    pub file: File,
    pub base: String,
    pub head: String,
    pub scope: ChangeScope,
}

impl ChangesSnapshot {
    fn into_rows(self, scope: ChangeScope) -> impl Iterator<Item = ReviewRow> {
        self.files.into_iter().map(move |file| ReviewRow {
            file,
            base: self.base.clone(),
            head: self.head.clone(),
            scope,
        })
    }
}

pub(super) fn visible_rows(
    root: &Path,
    selected: Option<ChangeScope>,
    approvals: &Approvals,
) -> Result<Vec<ReviewRow>> {
    let mut pending = Vec::new();
    let mut approved = Vec::new();
    let mut approved_paths = BTreeSet::<PathBuf>::new();

    for scope in [ChangeScope::All, ChangeScope::Uncommitted] {
        let is_selected = selected == Some(scope);
        if !is_selected && !approvals.values().any(|mark| mark.scope == scope) {
            continue;
        }
        let snapshot = match git::changed_files(root, scope) {
            Err(_) if selected.is_some() && !is_selected => continue,
            result => result?,
        };
        for row in snapshot.into_rows(scope) {
            let is_approved = approvals
                .get(&path_key(&row.file.path))
                .is_some_and(|mark| {
                    mark.scope == scope
                        && git::diff_fingerprint(root, &row.base, &row.head, &row.file)
                            .is_ok_and(|fingerprint| fingerprint == mark.fingerprint)
                });
            if is_approved {
                if selected.is_none() {
                    approved.push(row);
                } else {
                    approved_paths.insert(row.file.path);
                }
            } else if is_selected {
                pending.push(row);
            }
        }
    }

    if selected.is_none() {
        approved.sort_unstable_by(|a, b| a.file.path.as_os_str().cmp(b.file.path.as_os_str()));
        return Ok(approved);
    }
    pending.retain(|row| !approved_paths.contains(&row.file.path));
    Ok(pending)
}
