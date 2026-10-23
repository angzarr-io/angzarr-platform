//! The filesystem seam: every filesystem operation taskloom (and, later,
//! ctxloom) makes goes through an [`Fs`], held as `Arc<dyn Fs>`.
//!
//! Two implementors, both `std::fs` underneath:
//!
//! - [`Os`]: the real filesystem, paths as given. Production.
//! - [`Rooted`]: every path re-rooted under one directory (afero's
//!   `BasePathFs` over `OsFs`). Tests, through [`Rooted::temp`].
//!
//! Errors are `std::io::Error` with the kind the OS gave (`NotFound`,
//! `AlreadyExists`, ...), plus the seam's own refusals:
//!
//! - every path is ABSOLUTE: a relative one is refused as `InvalidInput`
//!   by both implementors, so no operation reads the process's working
//!   directory. A caller resolving a relative path joins it to the working
//!   directory it was handed (the composition root reads it once).
//! - a path given or returned is the CALLER'S path: [`Rooted`] never shows
//!   its root, except through [`Fs::real_path`], which exists for what
//!   must open a path itself (SQLite, advisory locks).
//!
//! PROPOSAL: the signatures are the design under review; the bodies are
//! not written.

mod os;
mod rooted;

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use os::Os;
pub use rooted::Rooted;

/// Permission bits (`0o777` at most). On unix they are the mode. On
/// Windows nothing yet honours them (an open question: Go reads
/// "owner-only" there as the DACL).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mode(u32);

impl Mode {
    /// A file only its owner may read or write.
    pub const PRIVATE_FILE: Mode = Mode(0o600);
    /// A directory only its owner may enter.
    pub const PRIVATE_DIR: Mode = Mode(0o700);
    /// A file anyone may read, its owner write.
    pub const FILE: Mode = Mode(0o644);
    /// A directory anyone may enter, its owner write.
    pub const DIR: Mode = Mode(0o755);

    /// The mode with these permission bits; anything past `0o777` is
    /// dropped.
    pub const fn from_bits(bits: u32) -> Mode {
        Mode(bits & 0o777)
    }

    /// The permission bits.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// The permissions an atomic write leaves the file with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perms {
    /// Exactly this mode, past the umask (Go's `WriteFile`).
    Exact(Mode),
    /// An existing file's own mode; this one for a file the write creates
    /// (Go's `WriteFileKeepMode`, agent-configs' `write`).
    KeepOr(Mode),
}

/// How [`Fs::open`] opens a file. A mode is the created file's, before
/// the umask; an existing file's mode is never changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Open {
    /// An existing file, read-only.
    Read,
    /// Created if missing, never truncated; for writing (a lock file).
    Create(Mode),
    /// Created only if missing (`O_EXCL`); `AlreadyExists` otherwise.
    CreateNew(Mode),
    /// Created if missing; every write lands at the end (`O_APPEND`).
    Append(Mode),
}

/// What a path is, links not followed (in [`Fs::symlink_metadata`] and a
/// directory's entries).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    /// A FIFO, socket or device.
    Other,
}

/// What [`Fs::metadata`] and [`Fs::symlink_metadata`] report.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub struct Meta {
    pub kind: Kind,
    /// In bytes.
    pub len: u64,
    /// A file with any execute bit (unix); any file (elsewhere).
    pub executable: bool,
    /// The permission bits on unix; `None` elsewhere.
    pub mode: Option<Mode>,
    pub modified: SystemTime,
}

/// One entry of a directory.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The entry's name in its directory (not a path).
    pub name: std::ffi::OsString,
    /// The entry itself, its link not followed.
    pub kind: Kind,
}

/// The filesystem. Every path is absolute (`InvalidInput` otherwise) and
/// the caller's own; see the crate's documentation.
pub trait Fs: Send + Sync + std::fmt::Debug {
    /// The file's bytes.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// The file's text; `InvalidData` when it is not UTF-8.
    fn read_to_string(&self, path: &Path) -> io::Result<String>;

    /// Replace the file at `path` with `bytes` whole: a unique temporary
    /// file beside it, written, synced, given `perms`, renamed over it, and
    /// its directory synced (unix). A reader sees all of the old file or
    /// all of the new; a failure leaves no temporary file. The directory
    /// must exist (`NotFound`).
    fn write_atomic(&self, path: &Path, bytes: &[u8], perms: Perms) -> io::Result<()>;

    /// The file, opened as `how` says: a handle for what needs one (an
    /// advisory lock, an appended log).
    fn open(&self, path: &Path, how: Open) -> io::Result<File>;

    /// The directory and its missing parents, each made with `mode` (before
    /// the umask). One already there is left as it is.
    fn create_dir_all(&self, path: &Path, mode: Mode) -> io::Result<()>;

    /// Remove the file (or link: never what it links to).
    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Remove the directory and everything in it, following no link.
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Rename `from` to `to`, replacing a file at `to`.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// Set the mode exactly.
    fn set_permissions(&self, path: &Path, mode: Mode) -> io::Result<()>;

    /// The directory's entries, sorted by name.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<Entry>>;

    /// What is at `path`, links followed.
    fn metadata(&self, path: &Path) -> io::Result<Meta>;

    /// What is at `path`, a link itself reported as one.
    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta>;

    /// Whether anything is at `path`, links followed (a dangling link is
    /// `false`); an error is a failure to find out, never "absent".
    fn exists(&self, path: &Path) -> io::Result<bool>;

    /// `path` with every link and `.`/`..` resolved, as the caller names
    /// it.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;

    /// Make `link` a link to the directory `target`, written as given
    /// (relative to `link`'s parent when relative).
    fn symlink_dir(&self, target: &Path, link: &Path) -> io::Result<()>;

    /// The target `link` was made with, as the caller gave it.
    fn read_link(&self, link: &Path) -> io::Result<PathBuf>;

    /// A new, empty directory in `parent`, its name starting `prefix`.
    /// Removing it is the caller's business.
    fn make_temp_dir(&self, parent: &Path, prefix: &str) -> io::Result<PathBuf>;

    /// The OS path `path` is at, for what must open a path itself (SQLite's
    /// database, an advisory lock file): `path` itself for [`Os`], its
    /// re-rooted place for [`Rooted`]. Nothing else may use it.
    fn real_path(&self, path: &Path) -> io::Result<PathBuf>;
}

#[cfg(test)]
#[path = "lib.test.rs"]
mod tests;
