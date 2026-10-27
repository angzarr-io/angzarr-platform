//! The real filesystem re-rooted under one directory (afero's `BasePathFs`
//! over `OsFs`): `/a/b` is `<root>/a/b`; on Windows `C:\a` is
//! `<root>\C\a` and `\\server\share\a` is `<root>\UNC\server\share\a`.
//! A `..` never climbs above the root, as `/..` is `/`: it is resolved
//! textually, before anything is looked up. A link made with an absolute
//! target is made re-rooted on disk, and reads back as given. A link
//! planted past the seam that leads out of the root is not defended
//! against, except that its resolution is never handed back
//! (`PermissionDenied`).

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Component, Components, Path, PathBuf};

use tempfile::TempDir;

use crate::{real, CwdError, Entry, Fs, Meta, Mode, Open, Perms};

/// The real filesystem under `root`: every path the caller gives is
/// re-rooted there, and every path it is given back is the caller's.
#[derive(Debug)]
pub struct Rooted {
    root: PathBuf,
    /// The caller's working directory: a caller's path, re-rooted like any
    /// other; or why there is none.
    cwd: Result<PathBuf, CwdError>,
    /// The root's owner, when [`Rooted::temp`] made it: removed on drop.
    _temp: Option<TempDir>,
}

impl Rooted {
    /// The filesystem re-rooted under `root`, an existing directory named
    /// by an absolute path (`InvalidInput` when relative, `NotFound` when
    /// missing). Its links are resolved once, here. A relative path
    /// resolves against `cwd`, the caller's absolute path (`InvalidInput`
    /// otherwise).
    pub fn new(root: &Path, cwd: &Path) -> io::Result<Rooted> {
        if !root.is_absolute() {
            return Err(not_absolute(root));
        }
        let cwd = Caller::parse(cwd)?.path();
        Rooted::over(root, Ok(cwd))
    }

    /// The filesystem re-rooted under `root` (as [`Rooted::new`]) with no
    /// working directory, it could not be read (`why`): a relative path
    /// fails with the [`CwdError`] it makes of `why`; an absolute one is
    /// re-rooted as ever.
    pub fn without_cwd(root: &Path, why: io::Error) -> io::Result<Rooted> {
        if !root.is_absolute() {
            return Err(not_absolute(root));
        }
        Rooted::over(root, Err(CwdError::of(&why)))
    }

    /// The filesystem re-rooted under `root`, an absolute path, working in
    /// `cwd`.
    fn over(root: &Path, cwd: Result<PathBuf, CwdError>) -> io::Result<Rooted> {
        let root = std::fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("not a directory: {}", root.display()),
            ));
        }
        Ok(Rooted {
            root,
            cwd,
            _temp: None,
        })
    }

    /// The filesystem re-rooted under a new temporary directory, removed
    /// with everything in it when this is dropped: a test's filesystem. Its
    /// working directory is the caller's top (`/`, or `C:\` on Windows).
    pub fn temp() -> io::Result<Rooted> {
        Rooted::temp_with_cwd(Path::new(if cfg!(windows) { r"C:\" } else { "/" }))
    }

    /// [`Rooted::temp`], working in `cwd`, the caller's absolute path
    /// (`InvalidInput` otherwise).
    pub fn temp_with_cwd(cwd: &Path) -> io::Result<Rooted> {
        let cwd = Caller::parse(cwd)?.path();
        let temp = TempDir::new()?;
        Ok(Rooted {
            root: std::fs::canonicalize(temp.path())?,
            cwd: Ok(cwd),
            _temp: Some(temp),
        })
    }

    /// The directory everything is under, its links resolved.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The caller's `path`, resolved against the working directory and
    /// cleaned.
    fn caller(&self, path: &Path) -> io::Result<Caller> {
        if path.is_absolute() {
            return Caller::parse(path);
        }
        let cwd = self.cwd.as_ref().map_err(CwdError::to_io)?;
        Caller::parse(&cwd.join(path))
    }

    /// Where the caller's `path` is under the root.
    fn reroot(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(self.caller(path)?.under(&self.root))
    }

    /// The caller's name for `real`, a path under the root; one outside it
    /// has none (`PermissionDenied`).
    fn unroot(&self, real: &Path) -> io::Result<PathBuf> {
        let outside = || {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the path resolves outside the filesystem's root",
            )
        };
        let rest = real.strip_prefix(&self.root).map_err(|_| outside())?;
        let mut components = rest.components();
        let top = Top::from_real(&mut components).ok_or_else(outside)?;
        let parts = components.map(|c| c.as_os_str().to_owned()).collect();
        Ok(Caller { top, parts }.path())
    }
}

impl Fs for Rooted {
    fn cwd(&self) -> Result<&Path, CwdError> {
        self.cwd.as_deref().map_err(Clone::clone)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(self.reroot(path)?)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(self.reroot(path)?)
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8], perms: Perms) -> io::Result<()> {
        real::write_atomic(&self.reroot(path)?, bytes, perms)
    }

    fn open(&self, path: &Path, how: Open) -> io::Result<File> {
        real::open(&self.reroot(path)?, how)
    }

    fn create_dir_all(&self, path: &Path, mode: Mode) -> io::Result<()> {
        real::create_dir_all(&self.reroot(path)?, mode)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        real::remove_file(&self.reroot(path)?)
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(self.reroot(path)?)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(self.reroot(from)?, self.reroot(to)?)
    }

    fn set_permissions(&self, path: &Path, mode: Mode) -> io::Result<()> {
        real::set_permissions(&self.reroot(path)?, mode)
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<Entry>> {
        real::read_dir(&self.reroot(path)?)
    }

    fn metadata(&self, path: &Path) -> io::Result<Meta> {
        real::metadata(&self.reroot(path)?)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        real::symlink_metadata(&self.reroot(path)?)
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        std::fs::exists(self.reroot(path)?)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(self.caller(path)?.path())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.unroot(&std::fs::canonicalize(self.reroot(path)?)?)
    }

    fn symlink_dir(&self, target: &Path, link: &Path) -> io::Result<()> {
        // A target from the top (`/x`, `C:\x`, `\x`) is re-rooted; one
        // relative to the link's directory is written as given.
        let from_top = matches!(
            target.components().next(),
            Some(Component::RootDir | Component::Prefix(_))
        );
        let on_disk = if from_top {
            self.reroot(target)?
        } else {
            target.to_path_buf()
        };
        real::symlink_dir(&on_disk, &self.reroot(link)?)
    }

    fn read_link(&self, link: &Path) -> io::Result<PathBuf> {
        let on_disk = real::read_link(&self.reroot(link)?)?;
        if on_disk.is_absolute() {
            self.unroot(&on_disk)
        } else {
            Ok(on_disk)
        }
    }

    fn make_temp_dir(&self, parent: &Path, prefix: &str) -> io::Result<PathBuf> {
        self.unroot(&real::make_temp_dir(&self.reroot(parent)?, prefix)?)
    }

    fn real_path(&self, path: &Path) -> io::Result<PathBuf> {
        self.reroot(path)
    }

    fn same_file(&self, file: &File, path: &Path) -> io::Result<bool> {
        real::same_file(file, &self.reroot(path)?)
    }
}

fn not_absolute(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("not an absolute path: {}", path.display()),
    )
}

/// A caller's absolute path, `.` and `..` resolved textually: a `..` at the
/// top stays there.
pub(crate) struct Caller {
    top: Top,
    parts: Vec<OsString>,
}

impl Caller {
    /// `path`, which must be absolute (`InvalidInput` otherwise).
    pub(crate) fn parse(path: &Path) -> io::Result<Caller> {
        let mut components = path.components();
        let top = Top::take(&mut components).ok_or_else(|| not_absolute(path))?;
        let mut parts = Vec::new();
        for component in components {
            match component {
                Component::Normal(part) => parts.push(part.to_owned()),
                Component::ParentDir => {
                    parts.pop();
                }
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => return Err(not_absolute(path)),
            }
        }
        Ok(Caller { top, parts })
    }

    /// As the caller spells it.
    pub(crate) fn path(&self) -> PathBuf {
        self.parts.iter().fold(self.top.caller(), |p, c| p.join(c))
    }

    /// Where it is under `root`.
    fn under(&self, root: &Path) -> PathBuf {
        self.parts
            .iter()
            .fold(self.top.under(root), |p, c| p.join(c))
    }
}

/// The top a caller's path starts from: `/` on unix.
#[cfg(unix)]
struct Top;

#[cfg(unix)]
impl Top {
    /// The top `components` start from, taken from them.
    fn take(components: &mut Components) -> Option<Top> {
        matches!(components.next(), Some(Component::RootDir)).then_some(Top)
    }

    /// The top named by the first of a path's `components` under the root,
    /// taken from them.
    fn from_real(_components: &mut Components) -> Option<Top> {
        Some(Top)
    }

    fn caller(&self) -> PathBuf {
        PathBuf::from("/")
    }

    fn under(&self, root: &Path) -> PathBuf {
        root.to_path_buf()
    }
}

/// The top a caller's path starts from on Windows: a drive (`C:\`, held
/// upper-case) or a share (`\\server\share\`), verbatim or not.
#[cfg(windows)]
enum Top {
    Disk(u8),
    Unc(OsString, OsString),
}

// Compiled only on Windows, where no runner tests it yet: its mutants
// cannot be killed here.
#[cfg(windows)]
#[cfg_attr(test, mutants::skip)]
impl Top {
    fn take(components: &mut Components) -> Option<Top> {
        use std::path::Prefix;
        let Some(Component::Prefix(prefix)) = components.next() else {
            return None;
        };
        let top = match prefix.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Top::Disk(d.to_ascii_uppercase()),
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                Top::Unc(server.to_owned(), share.to_owned())
            }
            _ => return None,
        };
        matches!(components.next(), Some(Component::RootDir)).then_some(top)
    }

    fn from_real(components: &mut Components) -> Option<Top> {
        let mut next = || match components.next() {
            Some(Component::Normal(part)) => Some(part.to_owned()),
            _ => None,
        };
        let first = next()?;
        if first == "UNC" {
            return Some(Top::Unc(next()?, next()?));
        }
        match first.as_encoded_bytes() {
            [d] if d.is_ascii_uppercase() => Some(Top::Disk(*d)),
            _ => None,
        }
    }

    fn caller(&self) -> PathBuf {
        match self {
            Top::Disk(d) => PathBuf::from(format!("{}:\\", char::from(*d))),
            Top::Unc(server, share) => {
                let mut top = OsString::from(r"\\");
                top.push(server);
                top.push(r"\");
                top.push(share);
                top.push(r"\");
                PathBuf::from(top)
            }
        }
    }

    fn under(&self, root: &Path) -> PathBuf {
        match self {
            Top::Disk(d) => root.join(char::from(*d).to_string()),
            Top::Unc(server, share) => root.join("UNC").join(server).join(share),
        }
    }
}

#[cfg(test)]
#[path = "rooted.test.rs"]
mod tests;
