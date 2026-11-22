use anyhow::{Context, Result, ensure};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use smol::Async;
use std::{
    fs::File,
    io,
    os::fd::BorrowedFd,
    path::Path,
    rc::Rc,
    thread,
    time::{Duration, Instant},
};

pub(super) struct Pty {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Pty {
    pub(super) fn spawn(root: &Path, size: PtySize) -> Result<(Self, Rc<Async<File>>)> {
        ensure!(root.is_dir(), "The checkout is unavailable");
        let pair = native_pty_system().openpty(size)?;
        let fd = pair
            .master
            .as_raw_fd()
            .context("open terminal descriptor")?;
        // The cloned descriptor retains close-on-exec and belongs to the async I/O tasks.
        let fd = unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned()?;
        let io = Rc::new(Async::new(File::from(fd))?);
        let mut shell = CommandBuilder::new_default_prog();
        shell.cwd(root);
        shell.env("TERM", "xterm-256color");
        shell.env("COLORTERM", "truecolor");
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
        ] {
            shell.env_remove(name);
        }
        let child = pair
            .slave
            .spawn_command(shell)
            .context("start login shell")?;
        Ok((
            Self {
                master: pair.master,
                child,
            },
            io,
        ))
    }

    pub(super) fn resize(&self, size: PtySize) -> Result<()> {
        self.master.resize(size)
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
        self.child.try_wait()
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let shell = self.child.process_id().map(|pid| pid as libc::pid_t);
        // The master identifies the foreground job even when the shell put it
        // in a separate process group. Closing a pane hangs up that job.
        if let Some(group) = self
            .master
            .process_group_leader()
            .filter(|group| *group > 0 && Some(*group) != shell)
        {
            unsafe { libc::kill(-group, libc::SIGHUP) };
        }
        if let Some(pid) = shell
            && matches!(self.child.try_wait(), Ok(None))
        {
            // Give the login shell time to relay the hangup to its background jobs.
            // Bound the wait in case startup scripts or a trap ignore SIGHUP.
            unsafe { libc::kill(pid, libc::SIGHUP) };
            let deadline = Instant::now() + Duration::from_millis(100);
            while matches!(self.child.try_wait(), Ok(None)) {
                if Instant::now() >= deadline {
                    // The unreaped child still owns its PID.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    let _ = self.child.wait();
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
