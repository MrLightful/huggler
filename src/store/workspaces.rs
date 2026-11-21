use super::*;
use crate::{
    workspace::SetupStatus,
    worktree::{self, git},
};

impl Database {
    pub fn allocate_workspace(&self, source: &ProjectId) -> Result<Project> {
        let root = self.project_root(source)?;
        let (repository, commit) = worktree::base(&root)?;
        let path = self.worktree_dir.join(new_id());
        let checkout = Worktree {
            path: path
                .to_str()
                .ok_or_else(|| Error::Invalid("expected UTF-8 workspace".into()))?
                .into(),
            common_dir: repository.clone(),
            base_commit: commit.clone(),
            ready: false,
        };
        checkout.prepare()?;
        let project = self.project(&path)?;
        self.db.execute(
            "INSERT INTO allocated_workspace VALUES(?,?,?)",
            params![project.id.0, repository, commit],
        )?;
        Ok(project)
    }
    pub fn release_workspace(&mut self, project: &ProjectId) -> Result<()> {
        let root = self.project_root(project)?;
        let (repository, commit): (String, String) = self
            .db
            .query_row(
                "SELECT common_dir,base_commit FROM allocated_workspace WHERE project=?",
                [&project.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::Forbidden)?;
        let tx = self.db.transaction()?;
        if tx.prepare("SELECT 1 FROM thread t JOIN project p ON p.id=t.project WHERE t.project=?1 OR t.worktree_path=?2 OR p.root=?2 UNION ALL SELECT 1 FROM workspace_setup WHERE project=?1 AND state='running'")?.exists(params![project.0,root.to_str()])?{
            return Err(Error::Busy);
        }
        if root.try_exists()? {
            if root.canonicalize()? != root
                || !root.starts_with(&self.worktree_dir)
                || !root.join(".git").is_file()
                || worktree::clean_base(&root)? != (repository.clone(), commit)
                || !git(
                    &root,
                    &[
                        "status",
                        "--porcelain",
                        "--ignored",
                        "--untracked-files=all",
                    ],
                )?
                .is_empty()
            {
                return Err(Error::Invalid("checkout changed since allocation".into()));
            }
            git(
                Path::new(&repository),
                &["worktree", "remove", "--", root.to_str().unwrap()],
            )?;
        }
        tx.execute("DELETE FROM project WHERE id=?", [&project.0])?;
        tx.commit()?;
        Ok(())
    }
    pub fn begin_setup(&self, project: &ProjectId) -> Result<PathBuf> {
        let path = self.project_root(project)?;
        if self.db.prepare("SELECT 1 FROM run r JOIN thread t ON t.id=r.thread_id WHERE t.project=? AND r.status IN ('queued','starting','running','waiting')")?.exists([&project.0])?
            || self.db.execute("INSERT INTO workspace_setup(project,state) VALUES(?,'running') ON CONFLICT(project) DO UPDATE SET state='running', detail=NULL WHERE state!='running'",[&project.0])?==0{
            return Err(Error::Busy);
        }
        Ok(path)
    }
    pub fn end_setup(&self, project: &ProjectId, failure: Option<String>) -> Result<SetupStatus> {
        self.db.execute(
            "UPDATE workspace_setup SET state=?, detail=? WHERE project=?",
            params![
                if failure.is_some() { "failed" } else { "ready" },
                failure,
                project.0
            ],
        )?;
        Ok(failure.map_or(SetupStatus::Ready, SetupStatus::Failed))
    }
    pub fn setup_status(&self, project: &ProjectId) -> Result<Option<SetupStatus>> {
        self.project_root(project)?;
        Ok(self
            .db
            .query_row(
                "SELECT state,detail FROM workspace_setup WHERE project=?",
                [&project.0],
                |r| match r.get::<_, String>(0)?.as_str() {
                    "ready" => Ok(SetupStatus::Ready),
                    "failed" => Ok(SetupStatus::Failed(r.get(1)?)),
                    _ => Ok(SetupStatus::Running),
                },
            )
            .optional()?)
    }
}
