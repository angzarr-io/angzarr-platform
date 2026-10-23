//! The real filesystem re-rooted under one directory (afero's `BasePathFs`
//! over `OsFs`): `/a/b` is `<root>/a/b`; on Windows `C:\a` is
//! `<root>\C\a` and `\\server\share\a` is `<root>\UNC\server\share\a`.
//! A `..` never climbs above the root, as `/..` is `/`. A link made with an
//! absolute target is made re-rooted on disk, and reads back as given.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::{Entry, Fs, Meta, Mode, Open, Perms};

/// The real filesystem under `root`: every path the caller gives is
/// re-rooted there, and every path it is given back is the caller's.
#[derive(Debug)]
pub struct Rooted {
    root: PathBuf,
    /// The root's owner, when [`Rooted::temp`] made it: removed on drop.
    _temp: Option<TempDir>,
}

impl Rooted {
    /// The filesystem re-rooted under `root`, an existing directory named
    /// by an absolute path (`InvalidInput` when relative, `NotFound` when
    /// missing). Its links are resolved once, here.
    pub fn new(_root: &Path) -> io::Result<Rooted> {
        todo!("proposal: Rooted::new")
    }

    /// The filesystem re-rooted under a new temporary directory, removed
    /// with everything in it when this is dropped: a test's filesystem.
    pub fn temp() -> io::Result<Rooted> {
        todo!("proposal: Rooted::temp")
    }

    /// The directory everything is under, its links resolved.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Fs for Rooted {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        todo!("proposal: Rooted::read")
    }

    fn read_to_string(&self, _path: &Path) -> io::Result<String> {
        todo!("proposal: Rooted::read_to_string")
    }

    fn write_atomic(&self, _path: &Path, _bytes: &[u8], _perms: Perms) -> io::Result<()> {
        todo!("proposal: Rooted::write_atomic")
    }

    fn open(&self, _path: &Path, _how: Open) -> io::Result<File> {
        todo!("proposal: Rooted::open")
    }

    fn create_dir_all(&self, _path: &Path, _mode: Mode) -> io::Result<()> {
        todo!("proposal: Rooted::create_dir_all")
    }

    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        todo!("proposal: Rooted::remove_file")
    }

    fn remove_dir_all(&self, _path: &Path) -> io::Result<()> {
        todo!("proposal: Rooted::remove_dir_all")
    }

    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        todo!("proposal: Rooted::rename")
    }

    fn set_permissions(&self, _path: &Path, _mode: Mode) -> io::Result<()> {
        todo!("proposal: Rooted::set_permissions")
    }

    fn read_dir(&self, _path: &Path) -> io::Result<Vec<Entry>> {
        todo!("proposal: Rooted::read_dir")
    }

    fn metadata(&self, _path: &Path) -> io::Result<Meta> {
        todo!("proposal: Rooted::metadata")
    }

    fn symlink_metadata(&self, _path: &Path) -> io::Result<Meta> {
        todo!("proposal: Rooted::symlink_metadata")
    }

    fn exists(&self, _path: &Path) -> io::Result<bool> {
        todo!("proposal: Rooted::exists")
    }

    fn canonicalize(&self, _path: &Path) -> io::Result<PathBuf> {
        todo!("proposal: Rooted::canonicalize")
    }

    fn symlink_dir(&self, _target: &Path, _link: &Path) -> io::Result<()> {
        todo!("proposal: Rooted::symlink_dir")
    }

    fn read_link(&self, _link: &Path) -> io::Result<PathBuf> {
        todo!("proposal: Rooted::read_link")
    }

    fn make_temp_dir(&self, _parent: &Path, _prefix: &str) -> io::Result<PathBuf> {
        todo!("proposal: Rooted::make_temp_dir")
    }

    fn real_path(&self, _path: &Path) -> io::Result<PathBuf> {
        todo!("proposal: Rooted::real_path")
    }
}

#[cfg(test)]
#[path = "rooted.test.rs"]
mod tests;
