//! The contract every [`Fs`] keeps: each test runs once against [`Os`]
//! (under a temporary directory, by its real path) and once against
//! [`Rooted`] (at a caller's path that names no real directory, `/work`),
//! with the same assertions. A test that reaches past the seam (to plant a
//! link the seam would not make, or to check what std sees) says so.

use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{Entry, Fs, Kind, Mode, Open, Os, Perms, Rooted};

/// A filesystem under test and the absolute directory its test works in.
struct Case {
    fs: Arc<dyn Fs>,
    base: PathBuf,
    _temp: Option<tempfile::TempDir>,
}

impl Case {
    fn at(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.base.clone(), |p, c| p.join(c))
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = self.at(rel);
        self.fs
            .write_atomic(&path, bytes, Perms::KeepOr(Mode::FILE))
            .unwrap();
        path
    }

    fn dir(&self, rel: &str) -> PathBuf {
        let path = self.at(rel);
        self.fs.create_dir_all(&path, Mode::DIR).unwrap();
        path
    }
}

fn os_case() -> Case {
    let temp = tempfile::tempdir().unwrap();
    // The temporary directory as its links resolve (macOS's /var is
    // /private/var), so canonicalize has nothing of its own to add.
    let base = std::fs::canonicalize(temp.path()).unwrap();
    Case {
        fs: Arc::new(Os),
        base,
        _temp: Some(temp),
    }
}

/// A caller's absolute path naming no real directory: only re-rooting
/// makes it writable.
fn caller_base() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\work")
    } else {
        PathBuf::from("/work")
    }
}

fn rooted_case() -> Case {
    let fs = Rooted::temp().unwrap();
    let base = caller_base();
    fs.create_dir_all(&base, Mode::DIR).unwrap();
    Case {
        fs: Arc::new(fs),
        base,
        _temp: None,
    }
}

/// Each named test function, run once against each implementor.
macro_rules! contract {
    ($($name:ident),* $(,)?) => {$(
        mod $name {
            #[test]
            fn os() {
                super::$name(&super::os_case());
            }

            #[test]
            fn rooted() {
                super::$name(&super::rooted_case());
            }
        }
    )*};
}

// --- Decision: object-safe, shared as Arc<dyn Fs> -------------------------

#[test]
fn test_an_fs_is_held_as_a_shared_trait_object() {
    fn shared<T: Send + Sync + 'static>(_: &T) {}
    let os: Arc<dyn Fs> = Arc::new(Os);
    shared(&os);
}

// --- Decision: absolute paths only; no op reads the working directory ----

fn test_a_relative_path_is_refused_as_invalid_input(case: &Case) {
    let fs = &case.fs;
    let rel = Path::new("relative/file");
    let abs = case.at("x");
    let invalid = |r: std::io::Result<()>, op: &str| {
        let e = r.expect_err(op);
        assert_eq!(e.kind(), ErrorKind::InvalidInput, "{op}: {e}");
    };
    invalid(fs.read(rel).map(drop), "read");
    invalid(fs.read_to_string(rel).map(drop), "read_to_string");
    invalid(
        fs.write_atomic(rel, b"x", Perms::Exact(Mode::FILE)),
        "write_atomic",
    );
    invalid(fs.open(rel, Open::Read).map(drop), "open");
    invalid(fs.create_dir_all(rel, Mode::DIR), "create_dir_all");
    invalid(fs.remove_file(rel), "remove_file");
    invalid(fs.remove_dir_all(rel), "remove_dir_all");
    invalid(fs.rename(rel, &abs), "rename from");
    invalid(fs.rename(&abs, rel), "rename to");
    invalid(fs.set_permissions(rel, Mode::FILE), "set_permissions");
    invalid(fs.read_dir(rel).map(drop), "read_dir");
    invalid(fs.metadata(rel).map(drop), "metadata");
    invalid(fs.symlink_metadata(rel).map(drop), "symlink_metadata");
    invalid(fs.exists(rel).map(drop), "exists");
    invalid(fs.canonicalize(rel).map(drop), "canonicalize");
    invalid(fs.symlink_dir(&abs, rel), "symlink_dir link");
    invalid(fs.read_link(rel).map(drop), "read_link");
    invalid(fs.make_temp_dir(rel, "t").map(drop), "make_temp_dir");
    invalid(fs.real_path(rel).map(drop), "real_path");
}

// --- Decision: errors are io::Error with the OS's own kinds ---------------

fn test_what_is_missing_is_not_found(case: &Case) {
    let fs = &case.fs;
    let missing = case.at("missing");
    let not_found = |r: std::io::Result<()>, op: &str| {
        let e = r.expect_err(op);
        assert_eq!(e.kind(), ErrorKind::NotFound, "{op}: {e}");
    };
    not_found(fs.read(&missing).map(drop), "read");
    not_found(fs.read_to_string(&missing).map(drop), "read_to_string");
    not_found(fs.open(&missing, Open::Read).map(drop), "open read");
    not_found(fs.remove_file(&missing), "remove_file");
    not_found(fs.remove_dir_all(&missing), "remove_dir_all");
    not_found(fs.read_dir(&missing).map(drop), "read_dir");
    not_found(fs.metadata(&missing).map(drop), "metadata");
    not_found(fs.symlink_metadata(&missing).map(drop), "symlink_metadata");
    not_found(fs.canonicalize(&missing).map(drop), "canonicalize");
    not_found(fs.read_link(&missing).map(drop), "read_link");
    assert!(!fs.exists(&missing).unwrap(), "exists");
}

fn test_text_that_is_not_utf8_is_invalid_data(case: &Case) {
    let path = case.write("bytes", &[0xff, 0xfe, 0x00]);
    let e = case.fs.read_to_string(&path).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidData);
    assert_eq!(case.fs.read(&path).unwrap(), [0xff, 0xfe, 0x00]);
}

// --- Decision: an atomic write replaces whole and leaves nothing beside ---

fn test_an_atomic_write_replaces_the_file_whole_and_leaves_no_temporary(case: &Case) {
    let path = case.write("config.json", b"first, and longer");
    case.fs
        .write_atomic(&path, b"second", Perms::KeepOr(Mode::FILE))
        .unwrap();
    assert_eq!(case.fs.read_to_string(&path).unwrap(), "second");
    let names: Vec<_> = case
        .fs
        .read_dir(&case.base)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["config.json"]);
}

fn test_an_atomic_write_does_not_make_its_directory(case: &Case) {
    let path = case.at("absent/config.json");
    let e = case
        .fs
        .write_atomic(&path, b"x", Perms::Exact(Mode::FILE))
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    assert!(!case.fs.exists(&case.at("absent")).unwrap());
}

#[cfg(unix)]
fn test_an_exact_mode_is_applied_past_the_umask(case: &Case) {
    // 0o664 is narrowed by any umask with a group- or other-write bit
    // (the usual 022 gives 0o644), so only a chmod after the create
    // leaves it.
    let path = case.at("shared");
    case.fs
        .write_atomic(&path, b"x", Perms::Exact(Mode::from_bits(0o664)))
        .unwrap();
    assert_eq!(
        case.fs.metadata(&path).unwrap().mode,
        Some(Mode::from_bits(0o664))
    );
}

#[cfg(unix)]
fn test_keep_or_keeps_an_existing_files_mode_and_gives_a_new_one_its_own(case: &Case) {
    let new = case.at("new");
    case.fs
        .write_atomic(&new, b"x", Perms::KeepOr(Mode::PRIVATE_FILE))
        .unwrap();
    assert_eq!(
        case.fs.metadata(&new).unwrap().mode,
        Some(Mode::PRIVATE_FILE)
    );

    let authored = case.at("authored");
    case.fs
        .write_atomic(&authored, b"x", Perms::Exact(Mode::from_bits(0o640)))
        .unwrap();
    case.fs
        .write_atomic(&authored, b"y", Perms::KeepOr(Mode::PRIVATE_FILE))
        .unwrap();
    assert_eq!(
        case.fs.metadata(&authored).unwrap().mode,
        Some(Mode::from_bits(0o640))
    );
}

// --- Decision: open hands back a std File (locks, appended logs) ----------

fn test_create_new_refuses_a_file_already_there(case: &Case) {
    let path = case.write("taken", b"mine");
    let e = case
        .fs
        .open(&path, Open::CreateNew(Mode::FILE))
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::AlreadyExists);
    assert_eq!(case.fs.read(&path).unwrap(), b"mine");

    let fresh = case.at("fresh");
    let mut file = case.fs.open(&fresh, Open::CreateNew(Mode::FILE)).unwrap();
    file.write_all(b"made").unwrap();
    drop(file);
    assert_eq!(case.fs.read(&fresh).unwrap(), b"made");
}

fn test_create_opens_a_file_there_without_truncating_it(case: &Case) {
    let path = case.write("lock", b"guarded path\n");
    drop(case.fs.open(&path, Open::Create(Mode::FILE)).unwrap());
    assert_eq!(case.fs.read(&path).unwrap(), b"guarded path\n");

    let missing = case.at("new.lock");
    drop(case.fs.open(&missing, Open::Create(Mode::FILE)).unwrap());
    assert_eq!(case.fs.read(&missing).unwrap(), b"");
}

fn test_append_writes_after_what_is_there_and_creates_what_is_not(case: &Case) {
    let path = case.at("log");
    for line in [&b"one\n"[..], b"two\n"] {
        let mut file = case.fs.open(&path, Open::Append(Mode::FILE)).unwrap();
        file.write_all(line).unwrap();
    }
    assert_eq!(case.fs.read(&path).unwrap(), b"one\ntwo\n");
}

fn test_read_opens_an_existing_file_for_reading_only(case: &Case) {
    let path = case.write("ro", b"text");
    let mut file = case.fs.open(&path, Open::Read).unwrap();
    let mut got = String::new();
    file.read_to_string(&mut got).unwrap();
    assert_eq!(got, "text");
    assert!(file.write_all(b"more").is_err(), "a read handle writes");
}

#[cfg(unix)]
fn test_a_created_file_gets_its_mode(case: &Case) {
    let path = case.at("private");
    drop(
        case.fs
            .open(&path, Open::CreateNew(Mode::PRIVATE_FILE))
            .unwrap(),
    );
    assert_eq!(
        case.fs.metadata(&path).unwrap().mode,
        Some(Mode::PRIVATE_FILE)
    );
}

// --- Decision: directories --------------------------------------------------

fn test_create_dir_all_makes_the_parents_and_may_be_repeated(case: &Case) {
    let deep = case.at("a/b/c");
    case.fs.create_dir_all(&deep, Mode::DIR).unwrap();
    case.fs.create_dir_all(&deep, Mode::DIR).unwrap();
    assert_eq!(case.fs.metadata(&deep).unwrap().kind, Kind::Dir);
    assert_eq!(case.fs.metadata(&case.at("a")).unwrap().kind, Kind::Dir);
}

#[cfg(unix)]
fn test_a_private_directory_is_owner_only(case: &Case) {
    let dir = case.at("private/inner");
    case.fs.create_dir_all(&dir, Mode::PRIVATE_DIR).unwrap();
    for d in [case.at("private"), dir] {
        assert_eq!(case.fs.metadata(&d).unwrap().mode, Some(Mode::PRIVATE_DIR));
    }
}

fn test_a_directory_lists_sorted_by_name_each_entry_as_itself(case: &Case) {
    case.write("b-file", b"x");
    let target = case.dir("c-dir");
    case.fs.symlink_dir(&target, &case.at("a-link")).unwrap();
    let entries = case.fs.read_dir(&case.base).unwrap();
    let want = [
        ("a-link", Kind::Symlink),
        ("b-file", Kind::File),
        ("c-dir", Kind::Dir),
    ]
    .map(|(name, kind)| Entry {
        name: name.into(),
        kind,
    });
    assert_eq!(entries, want);
}

fn test_remove_dir_all_removes_a_link_and_not_what_it_links_to(case: &Case) {
    let kept = case.dir("kept");
    let inside = case.write("kept/file", b"keep me");
    case.dir("doomed");
    case.fs.symlink_dir(&kept, &case.at("doomed/link")).unwrap();
    case.fs.remove_dir_all(&case.at("doomed")).unwrap();
    assert!(!case.fs.exists(&case.at("doomed")).unwrap());
    assert_eq!(case.fs.read(&inside).unwrap(), b"keep me");
}

fn test_remove_file_removes_a_link_and_not_what_it_links_to(case: &Case) {
    let target = case.dir("target");
    let link = case.at("link");
    case.fs.symlink_dir(&target, &link).unwrap();
    case.fs.remove_file(&link).unwrap();
    assert!(!case.fs.exists(&link).unwrap());
    assert!(case.fs.exists(&target).unwrap());
}

// --- Decision: metadata ------------------------------------------------------

fn test_metadata_follows_a_link_and_symlink_metadata_does_not(case: &Case) {
    let target = case.dir("target");
    let link = case.at("link");
    case.fs.symlink_dir(&target, &link).unwrap();
    assert_eq!(case.fs.metadata(&link).unwrap().kind, Kind::Dir);
    assert_eq!(case.fs.symlink_metadata(&link).unwrap().kind, Kind::Symlink);
}

fn test_metadata_reports_the_length_and_when_it_was_modified(case: &Case) {
    let before = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
    let path = case.write("sized", b"12345");
    let meta = case.fs.metadata(&path).unwrap();
    assert_eq!(meta.kind, Kind::File);
    assert_eq!(meta.len, 5);
    assert!(meta.modified >= before, "{:?} < {before:?}", meta.modified);
}

#[cfg(unix)]
fn test_a_file_with_any_execute_bit_is_executable(case: &Case) {
    let path = case.write("tool", b"#!/bin/sh\n");
    assert!(!case.fs.metadata(&path).unwrap().executable);
    case.fs
        .set_permissions(&path, Mode::from_bits(0o610))
        .unwrap();
    assert!(case.fs.metadata(&path).unwrap().executable);
    let dir = case.dir("dir");
    assert!(!case.fs.metadata(&dir).unwrap().executable, "a directory");
}

fn test_exists_follows_links_and_a_dangling_one_is_absent(case: &Case) {
    let file = case.write("file", b"x");
    let gone = case.dir("gone");
    let link = case.at("link");
    case.fs.symlink_dir(&gone, &link).unwrap();
    assert!(case.fs.exists(&file).unwrap());
    assert!(case.fs.exists(&link).unwrap());
    case.fs.remove_dir_all(&gone).unwrap();
    assert!(!case.fs.exists(&link).unwrap(), "a dangling link");
    assert_eq!(case.fs.symlink_metadata(&link).unwrap().kind, Kind::Symlink);
}

// --- Decision: rename and permissions ---------------------------------------

fn test_rename_replaces_a_file_at_the_target(case: &Case) {
    let from = case.write("from", b"new");
    let to = case.write("to", b"old");
    case.fs.rename(&from, &to).unwrap();
    assert_eq!(case.fs.read(&to).unwrap(), b"new");
    assert!(!case.fs.exists(&from).unwrap());
}

#[cfg(unix)]
fn test_set_permissions_sets_the_mode_exactly(case: &Case) {
    let path = case.write("file", b"x");
    case.fs
        .set_permissions(&path, Mode::from_bits(0o666))
        .unwrap();
    assert_eq!(
        case.fs.metadata(&path).unwrap().mode,
        Some(Mode::from_bits(0o666))
    );
}

// --- Decision: links read back as made; canonical paths are the caller's --

fn test_a_relative_link_reads_back_as_written_and_is_followed(case: &Case) {
    case.dir("real");
    let link = case.at("alias");
    case.fs.symlink_dir(Path::new("real"), &link).unwrap();
    assert_eq!(case.fs.read_link(&link).unwrap(), Path::new("real"));
    case.write("alias/through", b"via the link");
    assert_eq!(
        case.fs.read(&case.at("real/through")).unwrap(),
        b"via the link"
    );
}

fn test_an_absolute_link_reads_back_as_written_and_is_followed(case: &Case) {
    let real = case.dir("real");
    let link = case.at("alias");
    case.fs.symlink_dir(&real, &link).unwrap();
    assert_eq!(case.fs.read_link(&link).unwrap(), real);
    case.write("alias/through", b"via the link");
    assert_eq!(
        case.fs.read(&case.at("real/through")).unwrap(),
        b"via the link"
    );
}

fn test_read_link_of_what_is_not_a_link_is_invalid_input(case: &Case) {
    let file = case.write("plain", b"x");
    let e = case.fs.read_link(&file).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
}

fn test_canonicalize_resolves_links_and_dots_into_the_callers_path(case: &Case) {
    let real = case.dir("real");
    case.write("real/file", b"x");
    case.fs.symlink_dir(&real, &case.at("alias")).unwrap();
    let spelled = case.at("alias").join(".").join("file");
    assert_eq!(
        case.fs.canonicalize(&spelled).unwrap(),
        case.at("real/file")
    );
}

// --- Decision: temporary directories are caller-visible and fresh ----------

fn test_a_temporary_directory_is_new_empty_and_in_its_parent(case: &Case) {
    let one = case.fs.make_temp_dir(&case.base, ".stage-").unwrap();
    let two = case.fs.make_temp_dir(&case.base, ".stage-").unwrap();
    assert_ne!(one, two);
    for dir in [&one, &two] {
        assert_eq!(dir.parent(), Some(case.base.as_path()));
        let name = dir.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with(".stage-"), "{name}");
        assert!(case.fs.read_dir(dir).unwrap().is_empty());
    }
}

// --- Decision: real_path is where std (SQLite, flock) finds the same file --

fn test_real_path_is_where_std_finds_what_the_seam_wrote(case: &Case) {
    // Reaches past the seam on purpose: what real_path is for.
    let path = case.write("state.db", b"sqlite would open this");
    let real = case.fs.real_path(&path).unwrap();
    assert!(real.is_absolute());
    assert_eq!(std::fs::read(&real).unwrap(), b"sqlite would open this");
    // A path not there yet is still named (SQLite creates its database).
    let later = case.fs.real_path(&case.at("later.db")).unwrap();
    std::fs::write(&later, b"made by std").unwrap();
    assert_eq!(case.fs.read(&case.at("later.db")).unwrap(), b"made by std");
}

contract!(
    test_a_relative_path_is_refused_as_invalid_input,
    test_what_is_missing_is_not_found,
    test_text_that_is_not_utf8_is_invalid_data,
    test_an_atomic_write_replaces_the_file_whole_and_leaves_no_temporary,
    test_an_atomic_write_does_not_make_its_directory,
    test_create_new_refuses_a_file_already_there,
    test_create_opens_a_file_there_without_truncating_it,
    test_append_writes_after_what_is_there_and_creates_what_is_not,
    test_read_opens_an_existing_file_for_reading_only,
    test_create_dir_all_makes_the_parents_and_may_be_repeated,
    test_a_directory_lists_sorted_by_name_each_entry_as_itself,
    test_remove_dir_all_removes_a_link_and_not_what_it_links_to,
    test_remove_file_removes_a_link_and_not_what_it_links_to,
    test_metadata_follows_a_link_and_symlink_metadata_does_not,
    test_metadata_reports_the_length_and_when_it_was_modified,
    test_exists_follows_links_and_a_dangling_one_is_absent,
    test_rename_replaces_a_file_at_the_target,
    test_a_relative_link_reads_back_as_written_and_is_followed,
    test_an_absolute_link_reads_back_as_written_and_is_followed,
    test_read_link_of_what_is_not_a_link_is_invalid_input,
    test_canonicalize_resolves_links_and_dots_into_the_callers_path,
    test_a_temporary_directory_is_new_empty_and_in_its_parent,
    test_real_path_is_where_std_finds_what_the_seam_wrote,
);

#[cfg(unix)]
contract!(
    test_an_exact_mode_is_applied_past_the_umask,
    test_keep_or_keeps_an_existing_files_mode_and_gives_a_new_one_its_own,
    test_a_created_file_gets_its_mode,
    test_a_private_directory_is_owner_only,
    test_a_file_with_any_execute_bit_is_executable,
    test_set_permissions_sets_the_mode_exactly,
);
