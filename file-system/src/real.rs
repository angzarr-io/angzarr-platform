//! Each operation on an OS path, `std::fs` underneath: what [`crate::Os`]
//! and [`crate::Rooted`] both do once they know where a caller's path is.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::{Entry, Kind, Meta, Mode, Open, Perms};

pub(crate) fn write_atomic(path: &Path, bytes: &[u8], perms: Perms) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a path with no directory"))?;
    let mode = match perms {
        Perms::Exact(mode) => mode,
        Perms::KeepOr(mode) => match fs::metadata(path) {
            Ok(meta) => mode_of(&meta).unwrap_or(mode),
            Err(e) if e.kind() == io::ErrorKind::NotFound => mode,
            Err(e) => return Err(e),
        },
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // Dropped unpersisted (an error below), the temporary file is removed.
    let mut staged = tempfile::Builder::new()
        .prefix(&format!(".{name}."))
        .suffix(".tmp")
        .tempfile_in(dir)?;
    staged.write_all(bytes)?;
    set_file_mode(staged.as_file(), mode)?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|e| e.error)?;
    sync_dir(dir)
}

pub(crate) fn open(path: &Path, how: Open) -> io::Result<File> {
    let mut options = OpenOptions::new();
    let mode = match how {
        Open::Read => {
            options.read(true);
            None
        }
        Open::Create(mode) => {
            options.read(true).write(true).create(true).truncate(false);
            Some(mode)
        }
        Open::CreateNew(mode) => {
            options.read(true).write(true).create_new(true);
            Some(mode)
        }
        Open::Append(mode) => {
            options.append(true).create(true);
            Some(mode)
        }
    };
    #[cfg(unix)]
    if let Some(mode) = mode {
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode.bits());
    }
    #[cfg(not(unix))]
    let _ = mode;
    options.open(path)
}

pub(crate) fn create_dir_all(path: &Path, mode: Mode) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, mode.bits());
    #[cfg(not(unix))]
    let _ = mode;
    builder.create(path)
}

#[cfg(unix)]
pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

// Compiled only on Windows, where no runner tests it yet: its mutants
// cannot be killed here.
#[cfg(windows)]
#[cfg_attr(test, mutants::skip)]
pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink_dir()) {
        // A junction is a directory: removing it removes the link alone.
        return fs::remove_dir(path);
    }
    fs::remove_file(path)
}

pub(crate) fn set_permissions(path: &Path, mode: Mode) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode.bits()))
    }
    #[cfg(not(unix))]
    {
        // Modes are ignored off unix; only what is missing is refused.
        let _ = mode;
        fs::metadata(path).map(drop)
    }
}

pub(crate) fn read_dir(path: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            Ok(Entry {
                name: entry.file_name(),
                kind: kind_of(entry.file_type()?),
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub(crate) fn metadata(path: &Path) -> io::Result<Meta> {
    meta(&fs::metadata(path)?)
}

pub(crate) fn symlink_metadata(path: &Path) -> io::Result<Meta> {
    meta(&fs::symlink_metadata(path)?)
}

/// Make `link` a link to the directory `target`, written as given.
#[cfg(unix)]
pub(crate) fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Make `link` a junction to the directory `target`: a junction names its
/// target absolutely, so a relative one is resolved against `link`'s
/// directory.
// Compiled only on Windows, where no runner tests it yet: its mutants
// cannot be killed here.
#[cfg(windows)]
#[cfg_attr(test, mutants::skip)]
pub(crate) fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    let parent = link.parent().unwrap_or(Path::new(""));
    junction::create(parent.join(target), link)
}

#[cfg(unix)]
pub(crate) fn read_link(link: &Path) -> io::Result<PathBuf> {
    fs::read_link(link)
}

// Compiled only on Windows, where no runner tests it yet: its mutants
// cannot be killed here.
#[cfg(windows)]
#[cfg_attr(test, mutants::skip)]
pub(crate) fn read_link(link: &Path) -> io::Result<PathBuf> {
    if !fs::symlink_metadata(link)?.file_type().is_symlink() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a link"));
    }
    junction::get_target(link)
}

pub(crate) fn make_temp_dir(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    Ok(tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)?
        .keep())
}

#[cfg(unix)]
pub(crate) fn same_file(file: &File, path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let (held, there) = (file.metadata()?, fs::metadata(path)?);
    Ok(held.dev() == there.dev() && held.ino() == there.ino())
}

// Compiled only on Windows, where no runner tests it yet: its mutants
// cannot be killed here.
#[cfg(windows)]
#[cfg_attr(test, mutants::skip)]
pub(crate) fn same_file(file: &File, path: &Path) -> io::Result<bool> {
    let there = same_file::Handle::from_path(path)?;
    Ok(same_file::Handle::from_file(file.try_clone()?)? == there)
}

fn meta(meta: &fs::Metadata) -> io::Result<Meta> {
    let kind = kind_of(meta.file_type());
    let mode = mode_of(meta);
    Ok(Meta {
        kind,
        len: meta.len(),
        executable: kind == Kind::File && mode.is_none_or(|m| m.bits() & 0o111 != 0),
        mode,
        modified: meta.modified()?,
    })
}

fn kind_of(file_type: fs::FileType) -> Kind {
    if file_type.is_symlink() {
        Kind::Symlink
    } else if file_type.is_dir() {
        Kind::Dir
    } else if file_type.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

fn mode_of(meta: &fs::Metadata) -> Option<Mode> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(Mode::from_bits(meta.permissions().mode()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

fn set_file_mode(file: &File, mode: Mode) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(mode.bits()))
    }
    #[cfg(not(unix))]
    {
        let _ = (file, mode);
        Ok(())
    }
}

/// Make a rename in `dir` durable: sync the directory (unix; a no-op on
/// Windows, where a directory cannot be opened to sync).
// Untestable: durability across a crash is not observable from a test.
#[cfg_attr(test, mutants::skip)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}
