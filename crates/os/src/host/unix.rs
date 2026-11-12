//! The unix syscalls.

use std::ffi::{CStr, OsString};
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::task::Poll;

use process_wrap::std::{ChildWrapper, CommandWrap, ProcessGroup};
use tokio::signal::unix::{signal, SignalKind};

use crate::{Flavor, LockKind, Mode, Os, Signal, Signals, StopSignal};

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
        let mut signals = self.signals(&[Signal::Interrupt, Signal::Terminate])?;
        Ok(Box::pin(async move {
            signals.recv().await;
        }))
    }

    fn signals(&self, heard: &[Signal]) -> io::Result<Signals> {
        let mut streams = Vec::new();
        for &kind in Signal::ALL.iter().filter(|kind| heard.contains(kind)) {
            streams.push((kind, signal(kind_of(kind))?));
        }
        // In order, stopping at the first heard: each one passed has
        // registered its waker, and a caller polls again after a signal.
        Ok(Signals::from_source(move |cx| {
            for (kind, stream) in &mut streams {
                if stream.poll_recv(cx).is_ready() {
                    return Poll::Ready(*kind);
                }
            }
            Poll::Pending
        }))
    }

    fn user_home(&self) -> io::Result<PathBuf> {
        account_home(FIRST_BUFFER, LAST_BUFFER)
    }
}

/// getpwuid_r's first buffer for the entry's strings: what glibc's
/// `_SC_GETPW_R_SIZE_MAX` says, and more than an ordinary entry needs.
const FIRST_BUFFER: usize = 1024;

/// The most getpwuid_r's buffer is grown to: an entry needing more is
/// not an account's (one MiB).
const LAST_BUFFER: usize = 1_048_576;

/// The effective uid's home directory from the account database, with a
/// buffer of `start` bytes (at least one) for the entry's strings,
/// doubled while the entry does not fit (`ERANGE`) and the buffer is
/// shorter than `last`; past that, the `ERANGE`. No entry, or one with no
/// home or an empty one, is `NotFound`.
pub(crate) fn account_home(start: usize, last: usize) -> io::Result<PathBuf> {
    // SAFETY: a plain call; it cannot fail.
    let uid = unsafe { libc::geteuid() };
    let mut buffer = vec![0u8; start];
    loop {
        // SAFETY: an all-zero passwd is a valid value of the plain struct
        // (null pointers, zero ids); getpwuid_r fills it.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: every pointer is to memory this frame owns, `buffer`'s
        // length is the length given, and the entry's strings point into
        // `buffer`, read below while it lives.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        match status {
            libc::ERANGE if buffer.len() < last => buffer.resize(buffer.len() * 2, 0),
            // SAFETY: `found`, when not null, is `entry`, whose pw_dir is
            // null or a NUL-terminated string in `buffer`.
            0 => return unsafe { home_of(found.as_ref().map(|entry| entry.pw_dir.cast_const())) },
            error => return Err(io::Error::from_raw_os_error(error)),
        }
    }
}

/// The home directory an account entry names: `dir`, the entry's
/// pw_dir, or `None` for no entry. No entry, a null or an empty pw_dir is
/// `NotFound`.
///
/// # Safety
///
/// A `dir` that is not null is a NUL-terminated string, valid while this
/// runs.
pub(crate) unsafe fn home_of(dir: Option<*const libc::c_char>) -> io::Result<PathBuf> {
    let dir = match dir {
        // SAFETY: the caller's promise.
        Some(dir) if !dir.is_null() => unsafe { CStr::from_ptr(dir) },
        _ => return Err(no_home()),
    };
    if dir.is_empty() {
        return Err(no_home());
    }
    Ok(PathBuf::from(OsString::from_vec(dir.to_bytes().to_vec())))
}

fn no_home() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "the account has no home directory")
}

/// Start `command` leading a new session (and so a new process group),
/// with no controlling terminal: `setsid` in the child, before the exec.
/// A forked child never leads a group, so the call is not refused; one
/// that did not make the child its session's leader fails the spawn.
pub(crate) fn spawn_detached(mut command: Command) -> io::Result<Child> {
    // SAFETY: the closure runs in the forked child before the exec, and
    // calls only setsid and getpid, which are async-signal-safe,
    // allocating nothing.
    unsafe {
        command.pre_exec(|| {
            // The new session's id is the caller's pid; -1 on failure.
            if libc::setsid() != libc::getpid() {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}

fn kind_of(signal: Signal) -> SignalKind {
    match signal {
        Signal::Hangup => SignalKind::hangup(),
        Signal::Interrupt => SignalKind::interrupt(),
        Signal::Terminate => SignalKind::terminate(),
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

/// SIGTERM's number, which POSIX leaves to the system but every unix
/// target of this workspace (Linux, macOS, the BSDs) gives as 15.
const SIGTERM: i32 = 15;

pub(crate) fn wrap_tree(wrap: &mut CommandWrap) {
    wrap.wrap(ProcessGroup::leader());
}

pub(crate) fn request_stop(child: &dyn ChildWrapper) -> bool {
    child.signal(SIGTERM).is_ok()
}
