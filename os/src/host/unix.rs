//! The unix syscalls.

use std::ffi::OsString;
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-support"))]
use std::process::{Child, Command};
use std::task::Poll;

use tokio::signal::unix::{signal, SignalKind};

use crate::{Flavor, LockKind, Mode, Os, StopSignal};

#[derive(Debug)]
pub(crate) struct Host;

impl Os for Host {
    fn flavor(&self) -> Flavor {
        Flavor::Unix
    }

    fn set_mode(&self, path: &Path, mode: Mode) -> io::Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(mode.bits()))
    }

    fn set_file_mode(&self, file: &File, mode: Mode) -> io::Result<()> {
        file.set_permissions(fs::Permissions::from_mode(mode.bits()))
    }

    fn mode_of(&self, meta: &Metadata) -> Option<Mode> {
        Some(Mode::from_bits(meta.permissions().mode()))
    }

    fn open_mode(&self, options: &mut OpenOptions, mode: Mode) {
        options.mode(mode.bits());
    }

    fn dir_mode(&self, builder: &mut DirBuilder, mode: Mode) {
        builder.mode(mode.bits());
    }

    fn link_dir(&self, target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    fn read_link(&self, link: &Path) -> io::Result<PathBuf> {
        fs::read_link(link)
    }

    fn remove_link(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn same_file(&self, file: &File, path: &Path) -> io::Result<bool> {
        let (held, there) = (file.metadata()?, fs::metadata(path)?);
        Ok(held.dev() == there.dev() && held.ino() == there.ino())
    }

    // Durability across a crash is not observable from a test: only that
    // the directory is opened (a missing one is NotFound) is.
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        File::open(dir)?.sync_all()
    }

    fn try_lock(&self, file: &File, kind: LockKind) -> io::Result<bool> {
        super::try_lock(file, kind)
    }

    fn stop_signal(&self) -> io::Result<StopSignal> {
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut terminate = signal(SignalKind::terminate())?;
        Ok(Box::pin(std::future::poll_fn(move |cx| {
            let interrupted = interrupt.poll_recv(cx).is_ready();
            let terminated = terminate.poll_recv(cx).is_ready();
            if interrupted || terminated {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn own_group(command: &mut Command) -> &mut Command {
    command
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn ctrl_break(_child: &Child) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "unix has no console events",
    ))
}

pub(crate) fn not_unicode(prefix: &str) -> OsString {
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.push(0xff);
    OsString::from_vec(bytes)
}
