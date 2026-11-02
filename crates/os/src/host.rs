//! The host's syscall implementor, chosen by the compile target: the one
//! place in the workspace that does so.

use std::ffi::OsString;
use std::fs::File;
use std::io;
#[cfg(any(test, feature = "test-support"))]
use std::process::{Child, Command};
use std::sync::Arc;

use crate::{LockKind, Os};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

pub(crate) fn make() -> Arc<dyn Os> {
    Arc::new(imp::Host)
}

pub(crate) fn not_unicode(prefix: &str) -> OsString {
    imp::not_unicode(prefix)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn own_group(command: &mut Command) -> &mut Command {
    imp::own_group(command)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn ctrl_break(child: &Child) -> io::Result<()> {
    imp::ctrl_break(child)
}

/// Take `kind` of lock on `file` without waiting: fs4's flock on unix,
/// LockFileEx on Windows, which name no OS here.
fn try_lock(file: &File, kind: LockKind) -> io::Result<bool> {
    let taken = match kind {
        LockKind::Exclusive => fs4::FileExt::try_lock(file),
        LockKind::Shared => fs4::FileExt::try_lock_shared(file),
    };
    match taken {
        Ok(()) => Ok(true),
        Err(fs4::TryLockError::WouldBlock) => Ok(false),
        Err(fs4::TryLockError::Error(e)) => Err(e),
    }
}
