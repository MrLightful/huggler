//! Optional Unix process-output terminal with a 64 KiB reconnect window.
use super::*;
use std::{
    collections::VecDeque,
    io::Read,
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

pub struct TerminalHost;
const CAPACITY: usize = 65536;
#[derive(Default)]
struct Buffer {
    bytes: VecDeque<u8>,
    next: u64,
    eof: bool,
    failed: bool,
}
struct Terminal {
    child: Child,
    buffer: Arc<Mutex<Buffer>>,
}
impl ServiceHost for TerminalHost {
    fn open(&mut self, _: &ThreadId, checkout: &Path, request: Open) -> Result<Box<dyn Resource>> {
        let Open::Terminal { executable, args } = request else {
            return Err(Error::Unsupported("expected terminal".into()));
        };
        let (mut reader, writer) = std::io::pipe()?;
        let child = Command::new(executable)
            .args(args)
            .current_dir(checkout)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(writer.try_clone()?)
            .stderr(writer)
            .spawn()?;
        let terminal = Terminal {
            child,
            buffer: Arc::default(),
        };
        let buffer = terminal.buffer.clone();
        std::thread::Builder::new()
            .name("huggler-terminal-output".into())
            .spawn(move || {
                let mut chunk = [0; 4096];
                loop {
                    let read = reader.read(&mut chunk);
                    let mut b = buffer.lock().unwrap();
                    match read {
                        Ok(0) => {
                            b.eof = true;
                            break;
                        }
                        Ok(n) => {
                            let excess = (b.bytes.len() + n).saturating_sub(CAPACITY);
                            b.bytes.drain(..excess);
                            b.bytes.extend(&chunk[..n]);
                            b.next += n as u64;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => {
                            b.failed = true;
                            break;
                        }
                    }
                }
            })?;
        Ok(Box::new(terminal))
    }
}
impl Resource for Terminal {
    fn call(&mut self, action: Action) -> Result<Output> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if matches!(action, Action::ExitStatus) {
            use std::os::unix::process::ExitStatusExt;
            // Peek without reaping. Drop still owns the process group and kills
            // surviving descendants before the kernel can recycle the leader PID.
            let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let status = (unsafe { info.si_pid() } != 0).then(|| {
                let code = unsafe { info.si_status() };
                std::process::ExitStatus::from_raw(if info.si_code == libc::CLD_EXITED {
                    code << 8
                } else {
                    code
                })
            });
            return Ok(Output::ExitStatus(status));
        }
        let Action::Output { after } = action else {
            return Err(Error::Unsupported("expected output cursor".into()));
        };
        let b = self.buffer.lock().unwrap();
        if b.failed {
            return Err(Error::Integration(
                "terminal output pipe closed with an error".into(),
            ));
        }
        let start = b.next - b.bytes.len() as u64;
        let after = after.unwrap_or(start);
        if after < start {
            return Err(Error::CursorExpired);
        }
        if after > b.next {
            return Err(Error::Invalid("terminal cursor is ahead of output".into()));
        }
        Ok(Output::Terminal {
            next: b.next,
            data: b
                .bytes
                .iter()
                .skip((after - start) as usize)
                .copied()
                .collect(),
            eof: b.eof,
        })
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        // The unreaped child owns this process group until wait returns.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}
