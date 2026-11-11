//! The Windows syscalls. Compiled only on Windows, where CI's Windows
//! runner tests them (decision 5). Mutation testing runs on unix, where
//! their mutants cannot be killed, hence each `mutants::skip`.

use std::ffi::OsString;
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::FileTypeExt;
#[cfg(any(test, feature = "test-support"))]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-support"))]
use std::process::{Child, Command};
use std::task::Poll;

use process_wrap::std::{ChildWrapper, CommandWrap, JobObject};
use tokio::signal::windows as signal;

#[cfg(any(test, feature = "test-support"))]
use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
#[cfg(any(test, feature = "test-support"))]
use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

use crate::{Flavor, LockKind, Mode, Os, Signal, Signals, StopSignal};

#[derive(Debug)]
pub(crate) struct Host;

#[cfg_attr(test, mutants::skip)]
impl Os for Host {
    fn flavor(&self) -> Flavor {
        Flavor::Windows
    }

    fn set_mode(&self, path: &Path, _mode: Mode) -> io::Result<()> {
        // No modes: only what is missing is refused.
        fs::metadata(path).map(drop)
    }

    fn set_file_mode(&self, _file: &File, _mode: Mode) -> io::Result<()> {
        Ok(())
    }

    fn mode_of(&self, _meta: &Metadata) -> Option<Mode> {
        None
    }

    fn open_mode(&self, _options: &mut OpenOptions, _mode: Mode) {}

    fn dir_mode(&self, _builder: &mut DirBuilder, _mode: Mode) {}

    fn link_dir(&self, target: &Path, link: &Path) -> io::Result<()> {
        let parent = link.parent().unwrap_or(Path::new(""));
        junction::create(parent.join(target), link)
    }

    fn read_link(&self, link: &Path) -> io::Result<PathBuf> {
        if !fs::symlink_metadata(link)?.file_type().is_symlink() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a link"));
        }
        junction::get_target(link)
    }

    fn remove_link(&self, path: &Path) -> io::Result<()> {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink_dir()) {
            // A junction is a directory: removing it removes the link alone.
            return fs::remove_dir(path);
        }
        fs::remove_file(path)
    }

    fn same_file(&self, file: &File, path: &Path) -> io::Result<bool> {
        let there = same_file::Handle::from_path(path)?;
        Ok(same_file::Handle::from_file(file.try_clone()?)? == there)
    }

    fn sync_dir(&self, _dir: &Path) -> io::Result<()> {
        // NTFS makes a rename durable with the file; a directory cannot be
        // opened to sync.
        Ok(())
    }

    fn try_lock(&self, file: &File, kind: LockKind) -> io::Result<bool> {
        super::try_lock(file, kind)
    }

    // Go's os.Interrupt and SIGTERM on Windows: Ctrl-C and Ctrl-Break, and
    // the console's close, the user's logoff and the system's shutdown.
    fn stop_signal(&self) -> io::Result<StopSignal> {
        let mut signals = self.signals(&Signal::ALL)?;
        Ok(Box::pin(async move {
            signals.recv().await;
        }))
    }

    // Each console event is heard as the signal of its intent (the
    // `signal` module's docs).
    fn signals(&self, heard: &[Signal]) -> io::Result<Signals> {
        let _ = heard;

        Ok(Signals::from_source(|_| Poll::Pending))
    }
}

/// One console event's listener.
enum Event {
    C(signal::CtrlC),
    Break(signal::CtrlBreak),
    Close(signal::CtrlClose),
    Logoff(signal::CtrlLogoff),
    Shutdown(signal::CtrlShutdown),
}

impl Event {
    #[cfg_attr(test, mutants::skip)]
    fn poll_recv(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Option<()>> {
        match self {
            Event::C(e) => e.poll_recv(cx),
            Event::Break(e) => e.poll_recv(cx),
            Event::Close(e) => e.poll_recv(cx),
            Event::Logoff(e) => e.poll_recv(cx),
            Event::Shutdown(e) => e.poll_recv(cx),
        }
    }
}

#[cfg_attr(test, mutants::skip)]
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn own_group(command: &mut Command) -> &mut Command {
    command.creation_flags(CREATE_NEW_PROCESS_GROUP)
}

#[cfg_attr(test, mutants::skip)]
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn ctrl_break(child: &Child) -> io::Result<()> {
    // SAFETY: a plain call taking two integers; a group that is not there
    // is an error it returns.
    if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn not_unicode(prefix: &str) -> OsString {
    let mut wide: Vec<u16> = prefix.encode_utf16().collect();
    wide.push(0xD800);
    OsString::from_wide(&wide)
}

#[cfg_attr(test, mutants::skip)]
pub(crate) fn wrap_tree(wrap: &mut CommandWrap) {
    wrap.wrap(JobObject);
}

// A job has no graceful stop: the kill that follows is the stop.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn request_stop(_child: &dyn ChildWrapper) -> bool {
    false
}
