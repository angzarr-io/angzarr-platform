//! The host's syscalls, each held to what it promises, on whatever host
//! runs the tests: unix here; Windows on CI's Windows runner (decision 5).

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use tempfile::TempDir;

use super::{host, Flavor, LockKind, Mode};

fn dir() -> TempDir {
    TempDir::new().unwrap()
}

fn file(at: &Path) -> File {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(at)
        .unwrap()
}

/// The mode `path` has, as the host reads it.
fn mode(path: &Path) -> Option<Mode> {
    host().mode_of(&fs::metadata(path).unwrap())
}

/// `bits` where the host has modes; `None` where it has none.
fn expected(bits: u32) -> Option<Mode> {
    (host().flavor() == Flavor::Unix).then_some(Mode::from_bits(bits))
}

#[test]
fn test_the_host_reports_the_flavor_it_was_built_for() {
    let expected = if cfg!(windows) {
        Flavor::Windows
    } else {
        Flavor::Unix
    };
    assert_eq!(host().flavor(), expected);
}

#[test]
fn test_a_mode_set_is_the_mode_read_where_there_are_modes() {
    let tmp = dir();
    let path = tmp.path().join("f");
    file(&path);
    host().set_mode(&path, Mode::from_bits(0o640)).unwrap();
    assert_eq!(mode(&path), expected(0o640));
    host().set_mode(&path, Mode::from_bits(0o604)).unwrap();
    assert_eq!(mode(&path), expected(0o604));
}

#[test]
fn test_setting_the_mode_of_what_is_missing_is_not_found() {
    let tmp = dir();
    let e = host()
        .set_mode(&tmp.path().join("missing"), Mode::FILE)
        .unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::NotFound);
}

#[test]
fn test_an_open_files_mode_is_set_through_its_handle() {
    let tmp = dir();
    let path = tmp.path().join("f");
    let held = file(&path);
    host().set_file_mode(&held, Mode::from_bits(0o620)).unwrap();
    assert_eq!(mode(&path), expected(0o620));
}

#[test]
fn test_a_file_created_with_a_mode_has_it_and_a_directory_too() {
    let tmp = dir();
    let path = tmp.path().join("private");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    host().open_mode(&mut options, Mode::PRIVATE_FILE);
    options.open(&path).unwrap().write_all(b"x").unwrap();
    assert_eq!(mode(&path), expected(0o600));

    let sub = tmp.path().join("a").join("b");
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    host().dir_mode(&mut builder, Mode::PRIVATE_DIR);
    builder.create(&sub).unwrap();
    assert_eq!(mode(&sub), expected(0o700));
    assert_eq!(mode(&tmp.path().join("a")), expected(0o700));
}

#[test]
fn test_a_directory_link_reads_back_and_is_removed_alone() {
    let os = host();
    let tmp = dir();
    let (target, link) = (tmp.path().join("target"), tmp.path().join("link"));
    fs::create_dir(&target).unwrap();
    fs::write(target.join("kept"), "x").unwrap();
    os.link_dir(&target, &link).unwrap();
    assert!(link.join("kept").exists(), "the link reaches the directory");
    assert_eq!(os.read_link(&link).unwrap(), target);
    os.remove_link(&link).unwrap();
    assert!(!link.exists());
    assert!(
        fs::symlink_metadata(&link).is_err(),
        "the link itself is gone"
    );
    assert!(target.join("kept").exists(), "what it linked to stays");
}

#[test]
fn test_a_plain_file_is_removed_as_a_link_is() {
    let tmp = dir();
    let path = tmp.path().join("plain");
    file(&path);
    host().remove_link(&path).unwrap();
    assert!(!path.exists());
}

#[test]
fn test_read_link_of_what_is_not_a_link_is_invalid_input() {
    let tmp = dir();
    let path = tmp.path().join("plain");
    file(&path);
    let e = host().read_link(&path).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn test_a_file_is_the_file_at_its_path_until_another_replaces_it() {
    let os = host();
    let tmp = dir();
    let (path, other) = (tmp.path().join("lock"), tmp.path().join("other"));
    let held = file(&path);
    assert!(os.same_file(&held, &path).unwrap());
    file(&other);
    assert!(!os.same_file(&held, &other).unwrap(), "another file");
    fs::rename(&other, &path).unwrap();
    assert!(!os.same_file(&held, &path).unwrap(), "renamed over");
    fs::remove_file(&path).unwrap();
    let gone = os.same_file(&held, &path).unwrap_err();
    assert_eq!(gone.kind(), io::ErrorKind::NotFound);
}

#[test]
fn test_a_directory_syncs_and_a_missing_one_is_not_found_where_it_is_opened() {
    let tmp = dir();
    host().sync_dir(tmp.path()).unwrap();
    let missing = host().sync_dir(&tmp.path().join("missing"));
    if host().flavor() == Flavor::Unix {
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}

#[test]
fn test_an_exclusive_lock_excludes_and_shared_locks_share() {
    let os = host();
    let tmp = dir();
    let path = tmp.path().join("lock");
    let (first, second) = (file(&path), file(&path));
    assert!(os.try_lock(&first, LockKind::Exclusive).unwrap());
    assert!(!os.try_lock(&second, LockKind::Exclusive).unwrap());
    assert!(!os.try_lock(&second, LockKind::Shared).unwrap());
    drop(first);
    let third = file(&path);
    assert!(os.try_lock(&second, LockKind::Shared).unwrap());
    assert!(os.try_lock(&third, LockKind::Shared).unwrap());
    assert!(
        !os.try_lock(&file(&path), LockKind::Exclusive).unwrap(),
        "a shared lock excludes an exclusive one"
    );
}

/// The one test that registers for the stop signals, so no other test can
/// see the signals it sends: made, it listens and has not resolved; a
/// SIGTERM, then (on a second one) a SIGINT, sent to this process resolves
/// it.
#[test]
fn test_a_stop_signal_waits_for_a_signal_and_resolves_at_one() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let os = host();
    let mut stop = runtime.block_on(async { os.stop_signal() }).unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    assert_eq!(stop.as_mut().poll(&mut cx), Poll::Pending);
    if os.flavor() != Flavor::Unix {
        return;
    }
    for signal in ["-TERM", "-INT"] {
        let stop = runtime.block_on(async { os.stop_signal() }).unwrap();
        send(signal);
        let resolved =
            runtime.block_on(async { tokio::time::timeout(Duration::from_secs(10), stop).await });
        assert!(resolved.is_ok(), "{signal} did not resolve the stop signal");
    }
}

/// Unix has no console events: a Ctrl-Break is refused as unsupported,
/// and starting a child in its own group changes nothing of it. (On
/// Windows, the watch's clean stop at a Ctrl-Break is the binary's
/// acceptance test.)
#[test]
fn test_a_ctrl_break_is_unsupported_on_unix() {
    if host().flavor() != Flavor::Unix {
        return;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    let mut child = super::own_group(&mut command)
        .args(["--list", "--format", "terse"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let sent = super::ctrl_break(&child);
    assert_eq!(sent.unwrap_err().kind(), io::ErrorKind::Unsupported);
    assert!(child.wait().unwrap().success());
}

/// Send `signal` to this process, as `kill` does.
fn send(signal: &str) {
    let status = std::process::Command::new("kill")
        .arg(signal)
        .arg(std::process::id().to_string())
        .status()
        .unwrap();
    assert!(status.success(), "kill {signal}");
}

#[test]
fn test_text_that_is_not_unicode_is_not_unicode_and_keeps_its_prefix() {
    let odd = super::not_unicode("x-");
    assert!(odd.to_str().is_none());
    assert!(odd.as_encoded_bytes().starts_with(b"x-"));
}

#[test]
fn test_a_fixture_is_absolute_on_the_host_and_spelled_by_its_flavor() {
    let path = super::fixture("/home/u");
    assert!(path.is_absolute(), "{path:?}");
    assert_eq!(
        path.to_str(),
        Some(super::host().flavor().from_top("/home/u").as_str())
    );
}
