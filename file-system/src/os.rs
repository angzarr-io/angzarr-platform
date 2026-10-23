//! The real filesystem, paths as given.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Entry, Fs, Meta, Mode, Open, Perms};

/// The real filesystem: `std::fs` on each path as given (absolute only).
#[derive(Clone, Copy, Default, Debug)]
pub struct Os;

impl Fs for Os {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        todo!("proposal: Os::read")
    }

    fn read_to_string(&self, _path: &Path) -> io::Result<String> {
        todo!("proposal: Os::read_to_string")
    }

    fn write_atomic(&self, _path: &Path, _bytes: &[u8], _perms: Perms) -> io::Result<()> {
        todo!("proposal: Os::write_atomic")
    }

    fn open(&self, _path: &Path, _how: Open) -> io::Result<File> {
        todo!("proposal: Os::open")
    }

    fn create_dir_all(&self, _path: &Path, _mode: Mode) -> io::Result<()> {
        todo!("proposal: Os::create_dir_all")
    }

    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        todo!("proposal: Os::remove_file")
    }

    fn remove_dir_all(&self, _path: &Path) -> io::Result<()> {
        todo!("proposal: Os::remove_dir_all")
    }

    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        todo!("proposal: Os::rename")
    }

    fn set_permissions(&self, _path: &Path, _mode: Mode) -> io::Result<()> {
        todo!("proposal: Os::set_permissions")
    }

    fn read_dir(&self, _path: &Path) -> io::Result<Vec<Entry>> {
        todo!("proposal: Os::read_dir")
    }

    fn metadata(&self, _path: &Path) -> io::Result<Meta> {
        todo!("proposal: Os::metadata")
    }

    fn symlink_metadata(&self, _path: &Path) -> io::Result<Meta> {
        todo!("proposal: Os::symlink_metadata")
    }

    fn exists(&self, _path: &Path) -> io::Result<bool> {
        todo!("proposal: Os::exists")
    }

    fn canonicalize(&self, _path: &Path) -> io::Result<PathBuf> {
        todo!("proposal: Os::canonicalize")
    }

    fn symlink_dir(&self, _target: &Path, _link: &Path) -> io::Result<()> {
        todo!("proposal: Os::symlink_dir")
    }

    fn read_link(&self, _link: &Path) -> io::Result<PathBuf> {
        todo!("proposal: Os::read_link")
    }

    fn make_temp_dir(&self, _parent: &Path, _prefix: &str) -> io::Result<PathBuf> {
        todo!("proposal: Os::make_temp_dir")
    }

    fn real_path(&self, _path: &Path) -> io::Result<PathBuf> {
        todo!("proposal: Os::real_path")
    }
}

#[cfg(test)]
#[path = "os.test.rs"]
mod tests;
