mod ssh;
mod view;

pub(crate) use ssh::{SavedMachine, Target};
pub(crate) use view::RemoteProject;

use anyhow::{Result, bail};
use huggler::machine::{Reply, Request};
use std::path::PathBuf;

pub(crate) async fn add(target: Target, checkout: PathBuf) -> Result<SavedMachine> {
    let connection = ssh::Connection::open(&target, None).await?;
    let Reply::Project(project) = connection
        .client
        .request(Request::OpenProject { checkout })
        .await?
    else {
        bail!("The machine returned an incompatible project response.");
    };
    Ok(SavedMachine {
        target,
        environment: connection.environment.clone(),
        project: project.id,
        checkout: project.root,
    })
}
