//! What only re-rooting decides: where a caller's path lands under the
//! root, that nothing climbs out of it, and that the root never shows.
//! These reach past the seam (std on the real paths) to see where things
//! landed.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::Rooted;
use crate::{Fs, Mode, Perms};

/// `parts` joined under `root`.
fn under(root: &Path, parts: &[&str]) -> PathBuf {
    parts.iter().fold(root.to_path_buf(), |p, c| p.join(c))
}

fn caller(unix: &str, windows: &str) -> PathBuf {
    PathBuf::from(if cfg!(windows) { windows } else { unix })
}

// --- Construction --------------------------------------------------------

#[test]
fn test_a_root_must_be_an_absolute_existing_directory() {
    let e = Rooted::new(Path::new("relative/root")).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
    let temp = tempfile::tempdir().unwrap();
    let e = Rooted::new(&temp.path().join("missing")).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
}

#[test]
fn test_a_root_is_held_with_its_links_resolved() {
    let temp = tempfile::tempdir().unwrap();
    let fs = Rooted::new(temp.path()).unwrap();
    assert_eq!(fs.root(), std::fs::canonicalize(temp.path()).unwrap());
}

#[test]
fn test_a_temporary_root_goes_when_the_filesystem_is_dropped() {
    let fs = Rooted::temp().unwrap();
    let root = fs.root().to_path_buf();
    assert!(root.is_dir());
    let file = caller("/home/u/file", r"C:\Users\u\file");
    fs.create_dir_all(file.parent().unwrap(), Mode::DIR)
        .unwrap();
    fs.write_atomic(&file, b"x", Perms::Exact(Mode::FILE))
        .unwrap();
    drop(fs);
    assert!(!root.exists());
}

#[test]
fn test_two_roots_see_nothing_of_each_other() {
    let one = Rooted::temp().unwrap();
    let two = Rooted::temp().unwrap();
    let path = caller("/shared/name", r"C:\shared\name");
    one.create_dir_all(path.parent().unwrap(), Mode::DIR)
        .unwrap();
    one.write_atomic(&path, b"one's", Perms::Exact(Mode::FILE))
        .unwrap();
    assert!(!two.exists(&path).unwrap());
}

// --- Decision: an absolute path lands under the root ------------------------

#[cfg(unix)]
#[test]
fn test_an_absolute_path_lands_at_the_same_path_under_the_root() {
    let fs = Rooted::temp().unwrap();
    let real = fs
        .real_path(Path::new("/home/u/.ctxloom/state.db"))
        .unwrap();
    assert_eq!(
        real,
        under(fs.root(), &["home", "u", ".ctxloom", "state.db"])
    );
}

#[cfg(windows)]
#[test]
fn test_a_drive_path_lands_under_its_drive_letter() {
    let fs = Rooted::temp().unwrap();
    let real = fs.real_path(Path::new(r"C:\Users\u\x")).unwrap();
    assert_eq!(real, under(fs.root(), &["C", "Users", "u", "x"]));
}

#[cfg(windows)]
#[test]
fn test_a_drive_letter_of_either_case_is_one_directory() {
    let fs = Rooted::temp().unwrap();
    assert_eq!(
        fs.real_path(Path::new(r"c:\x")).unwrap(),
        fs.real_path(Path::new(r"C:\x")).unwrap()
    );
}

#[cfg(windows)]
#[test]
fn test_a_unc_path_lands_under_unc_server_share() {
    let fs = Rooted::temp().unwrap();
    let real = fs.real_path(Path::new(r"\\server\share\dir\x")).unwrap();
    assert_eq!(
        real,
        under(fs.root(), &["UNC", "server", "share", "dir", "x"])
    );
}

#[cfg(windows)]
#[test]
fn test_a_verbatim_path_lands_where_its_plain_spelling_does() {
    let fs = Rooted::temp().unwrap();
    assert_eq!(
        fs.real_path(Path::new(r"\\?\C:\x")).unwrap(),
        fs.real_path(Path::new(r"C:\x")).unwrap()
    );
    assert_eq!(
        fs.real_path(Path::new(r"\\?\UNC\server\share\x")).unwrap(),
        fs.real_path(Path::new(r"\\server\share\x")).unwrap()
    );
}

// --- Decision: `..` cannot climb above the root, as `/..` is `/` ------------

#[test]
fn test_dot_dot_stops_at_the_root() {
    let fs = Rooted::temp().unwrap();
    let climbing = caller("/../../etc/passwd", r"C:\..\..\etc\passwd");
    let landed = fs.real_path(&climbing).unwrap();
    let want = if cfg!(windows) {
        under(fs.root(), &["C", "etc", "passwd"])
    } else {
        under(fs.root(), &["etc", "passwd"])
    };
    assert_eq!(landed, want);
    let deeper = caller("/a/../../b", r"C:\a\..\..\b");
    assert!(fs.real_path(&deeper).unwrap().starts_with(fs.root()));
}

#[test]
fn test_a_write_that_climbs_lands_inside_the_root() {
    let fs = Rooted::temp().unwrap();
    let top = caller("/", r"C:\");
    fs.create_dir_all(&top, Mode::DIR).unwrap();
    let climbing = caller("/../escaped", r"C:\..\escaped");
    fs.write_atomic(&climbing, b"kept in", Perms::Exact(Mode::FILE))
        .unwrap();
    assert_eq!(
        std::fs::read(fs.real_path(&caller("/escaped", r"C:\escaped")).unwrap()).unwrap(),
        b"kept in"
    );
    assert!(!fs.root().parent().unwrap().join("escaped").exists());
}

// --- Decision: a link with an absolute target is re-rooted on disk ----------

#[test]
fn test_an_absolute_link_target_is_rerooted_on_disk_and_reads_back_as_given() {
    let fs = Rooted::temp().unwrap();
    let target = caller("/data/real", r"C:\data\real");
    let link = caller("/data/alias", r"C:\data\alias");
    fs.create_dir_all(&target, Mode::DIR).unwrap();
    fs.symlink_dir(&target, &link).unwrap();
    let on_disk = std::fs::read_link(fs.real_path(&link).unwrap()).unwrap();
    assert_eq!(on_disk, fs.real_path(&target).unwrap());
    assert_eq!(fs.read_link(&link).unwrap(), target);
}

// --- Decision: the root never shows in what the caller is given -------------

#[test]
fn test_a_canonical_path_is_the_callers_never_the_roots() {
    let fs = Rooted::temp().unwrap();
    let dir = caller("/work/real", r"C:\work\real");
    fs.create_dir_all(&dir, Mode::DIR).unwrap();
    let got = fs.canonicalize(&dir.join("..").join("real")).unwrap();
    assert_eq!(got, dir);
}

#[test]
fn test_a_temporary_directory_is_named_as_the_caller_names_it() {
    let fs = Rooted::temp().unwrap();
    let parent = caller("/tmp", r"C:\Temp");
    fs.create_dir_all(&parent, Mode::DIR).unwrap();
    let made = fs.make_temp_dir(&parent, "t-").unwrap();
    assert_eq!(made.parent(), Some(parent.as_path()));
    assert!(fs.real_path(&made).unwrap().is_dir());
}

#[cfg(unix)]
#[test]
fn test_a_root_reached_through_a_link_still_never_shows() {
    // macOS's temporary directories are under /var, a link to
    // /private/var: the root as given and as resolved both stay hidden.
    let temp = tempfile::tempdir().unwrap();
    let real_root = temp.path().join("root");
    std::fs::create_dir(&real_root).unwrap();
    let linked_root = temp.path().join("linked");
    std::os::unix::fs::symlink(&real_root, &linked_root).unwrap();
    let fs = Rooted::new(&linked_root).unwrap();
    fs.create_dir_all(Path::new("/a/b"), Mode::DIR).unwrap();
    assert_eq!(
        fs.canonicalize(Path::new("/a/./b")).unwrap(),
        Path::new("/a/b")
    );
}

#[cfg(unix)]
#[test]
fn test_canonicalizing_out_of_the_root_is_refused() {
    // A link the seam would not make (planted by std) pointing outside the
    // root: its resolution has no caller's path, so it is an error rather
    // than the real path leaking.
    let outside = tempfile::tempdir().unwrap();
    let fs = Rooted::temp().unwrap();
    fs.create_dir_all(Path::new("/w"), Mode::DIR).unwrap();
    let link = fs.real_path(Path::new("/w/out")).unwrap();
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    assert!(fs.canonicalize(Path::new("/w/out")).is_err());
}
