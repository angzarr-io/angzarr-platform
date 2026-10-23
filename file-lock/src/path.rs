//! Where a lock lives: the flat name, the home path and the reap
//! interlock's path, derived so every binary of the family names one lock
//! file for one guarded file.

use std::path::{Component, Path, PathBuf};

use file_system::Fs;
use sha2::{Digest, Sha256};

use crate::LockError;

/// The home lock directory under a home: the one directory `ctxloom clean`
/// reaps.
pub const HOME_LOCKS: &str = ".ctxloom/locks";

/// The sessions root under a home.
const SESSIONS: &str = ".ctxloom/sessions";

/// A session's own lock directory, inside its harp directory.
const HARP_LOCKS: &str = "locks";

/// The reap interlock's name in the home lock directory.
pub const REAP_INTERLOCK: &str = "reap.interlock";

/// A lock file's suffix.
const SUFFIX: &str = ".lock";

/// The readable tail a flat name keeps, in bytes.
const TAIL_MAX: usize = 96;

/// The hex digits of the path's SHA-256 a flat name keeps.
const HASH_LEN: usize = 16;

/// `path` as one bounded file name: the last [`TAIL_MAX`] bytes of it
/// with `/` as `__` and NTFS's reserved characters escaped, then `__` and
/// the first [`HASH_LEN`] hex digits of the whole path's SHA-256.
pub fn flat_name(path: &str) -> String {
    // Only a platform separator that is not `/` becomes one.
    let slashed = path.replace(std::path::MAIN_SEPARATOR, "/");
    let digest = Sha256::digest(slashed.as_bytes());
    let hash: String = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..HASH_LEN]
        .to_string();
    let mut flat = String::with_capacity(slashed.len());
    for c in slashed.replace('/', "__").chars() {
        if u32::from(c) < 0x20 || "<>:\"|?*".contains(c) {
            flat.push_str(&format!("%{:02X}", u32::from(c)));
        } else {
            flat.push(c);
        }
    }
    // The cut is 0 for a name within the bound; past it, a cut inside a
    // character steps forward to the next one.
    let mut cut = flat.len().saturating_sub(TAIL_MAX);
    while !flat.is_char_boundary(cut) {
        cut += 1;
    }
    format!("{}__{hash}", &flat[cut..])
}

/// `path` cleaned lexically: no `.`, and each `..` taking the component
/// before it (none above the root).
pub fn clean(path: &Path) -> PathBuf {
    let mut parts: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(Component::Normal(_)) => {
                    parts.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => parts.push(component),
            },
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return PathBuf::from(".");
    }
    parts.iter().collect()
}

/// `path` joined to `fs`'s working directory when it is relative, and
/// cleaned. Links are never resolved, so a path names its lock however its
/// links stand.
pub fn absolute(fs: &dyn Fs, path: &Path) -> Result<PathBuf, LockError> {
    fs.absolute(path).map_err(|e| LockError::io(path, e))
}

/// The locks guarding a file: its sidecar, and the reap interlock to hold
/// shared around taking it, when the sidecar is one `ctxloom clean` may
/// reap.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sidecar {
    pub lock: PathBuf,
    pub interlock: Option<PathBuf>,
}

/// The lock path under `home` for `target` (absolute and clean, as
/// [`absolute`] makes it): strictly inside a session's directory,
/// `~/.ctxloom/sessions/<harp>/locks/<FlatName(rel)>.lock`, with no
/// interlock (a session's locks go with it); anywhere else,
/// `~/.ctxloom/locks/<FlatName(target)>.lock`, under the reap interlock.
pub fn sidecar_for(fs: &dyn Fs, home: &Path, target: &Path) -> Sidecar {
    let sessions = clean(&home.join(SESSIONS));
    let inside = split_harp(&sessions, target).or_else(|| {
        // The sessions root as its links resolve too, so a target is
        // inside it whichever spelling of the root it was given under.
        let resolved = fs.canonicalize(&sessions).ok()?;
        split_harp(&resolved, target)
    });
    if let Some((harp, rel)) = inside {
        let name = format!("{}{SUFFIX}", flat_name(&rel));
        return Sidecar {
            lock: sessions.join(harp).join(HARP_LOCKS).join(name),
            interlock: None,
        };
    }
    let dir = clean(&home.join(HOME_LOCKS));
    let name = format!("{}{SUFFIX}", flat_name(&target.to_string_lossy()));
    Sidecar {
        lock: dir.join(name),
        interlock: Some(dir.join(REAP_INTERLOCK)),
    }
}

/// `target` split at the harp directory it lies strictly inside, directly
/// under `sessions`: the harp, and the rest of the path (slash-separated).
fn split_harp(sessions: &Path, target: &Path) -> Option<(String, String)> {
    let root = sessions.to_string_lossy().replace('\\', "/");
    let target = target.to_string_lossy().replace('\\', "/");
    let under = target.strip_prefix(&format!("{}/", root.trim_end_matches('/')))?;
    let (harp, rel) = under.split_once('/')?;
    (!rel.is_empty() && valid_harp(harp)).then(|| (harp.to_string(), rel.to_string()))
}

/// Whether `harp` is a harp: one path component, with no edge a human
/// cannot see.
fn valid_harp(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(|c| c < '\u{20}' || c == '\u{7f}')
        && name.trim() == name
}

#[cfg(test)]
#[path = "path.test.rs"]
mod tests;
