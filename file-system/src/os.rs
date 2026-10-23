//! The real filesystem, paths as given.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::{real, Entry, Fs, Meta, Mode, Open, Perms};

/// The real filesystem: `std::fs` on each path as given, a relative one
/// joined to the working directory it was made with.
#[derive(Clone, Debug)]
pub struct Os {
    cwd: PathBuf,
}

impl Os {
    /// The real filesystem, resolving relative paths against `cwd`, an
    /// absolute path (`InvalidInput` otherwise).
    pub fn new(cwd: &Path) -> io::Result<Os> {
        Ok(Os {
            cwd: absolute(cwd.to_path_buf())?,
        })
    }

    /// `path` as given, or joined to the working directory when relative.
    fn at(&self, path: &Path) -> io::Result<PathBuf> {
        absolute(self.cwd.join(path))
    }
}

/// `path`, refused as `InvalidInput` unless absolute (on Windows, `C:x`
/// is not).
fn absolute(path: PathBuf) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not an absolute path: {}", path.display()),
        ))
    }
}

impl Fs for Os {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(self.at(path)?)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(self.at(path)?)
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8], perms: Perms) -> io::Result<()> {
        real::write_atomic(&self.at(path)?, bytes, perms)
    }

    fn open(&self, path: &Path, how: Open) -> io::Result<File> {
        real::open(&self.at(path)?, how)
    }

    fn create_dir_all(&self, path: &Path, mode: Mode) -> io::Result<()> {
        real::create_dir_all(&self.at(path)?, mode)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        real::remove_file(&self.at(path)?)
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(self.at(path)?)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(self.at(from)?, self.at(to)?)
    }

    fn set_permissions(&self, path: &Path, mode: Mode) -> io::Result<()> {
        real::set_permissions(&self.at(path)?, mode)
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<Entry>> {
        real::read_dir(&self.at(path)?)
    }

    fn metadata(&self, path: &Path) -> io::Result<Meta> {
        real::metadata(&self.at(path)?)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        real::symlink_metadata(&self.at(path)?)
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        std::fs::exists(self.at(path)?)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(self.at(path)?)
    }

    fn symlink_dir(&self, target: &Path, link: &Path) -> io::Result<()> {
        real::symlink_dir(target, &self.at(link)?)
    }

    fn read_link(&self, link: &Path) -> io::Result<PathBuf> {
        real::read_link(&self.at(link)?)
    }

    fn make_temp_dir(&self, parent: &Path, prefix: &str) -> io::Result<PathBuf> {
        real::make_temp_dir(&self.at(parent)?, prefix)
    }

    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        self.at(path)
    }

    fn same_file(&self, file: &File, path: &Path) -> io::Result<bool> {
        real::same_file(file, &self.at(path)?)
    }
}

#[cfg(test)]
#[path = "os.test.rs"]
mod tests;
