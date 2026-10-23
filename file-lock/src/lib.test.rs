//! Taking a lock: the re-check, the bounded wait, shared and exclusive
//! holders, the guard record, and a foreign file's sidecar under the reap
//! interlock. Each contention is forced: the holder is this test's own
//! handle, and a wait is released from inside the waiter's own pause.
//! Every test's files are on a filesystem of its own (`Rooted::temp`).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use file_system::{Fs, Mode, Open, Perms, Rooted};

use super::{
    lock, lock_file, lock_within, path, take_current, take_once, until, Kind, LockError, Waits,
    LOCK_ATTEMPTS, LOCK_WAIT,
};

/// Open and lock the lock file at `path` exclusively, as another process
/// holding it would.
fn hold(fs: &dyn Fs, path: &Path) -> File {
    fs.create_dir_all(path.parent().unwrap(), Mode::DIR)
        .unwrap();
    let file = fs.open(path, Open::Create(Mode::FILE)).unwrap();
    fs4::FileExt::lock(&file).unwrap();
    file
}

/// Whether some holder keeps `path` from being locked exclusively now.
fn held(fs: &dyn Fs, path: &Path) -> bool {
    let file = fs.open(path, Open::Read).unwrap();
    fs4::FileExt::try_lock(&file).is_err()
}

/// Whether some exclusive holder keeps `path` from being locked shared now.
fn held_exclusively(fs: &dyn Fs, path: &Path) -> bool {
    let file = fs.open(path, Open::Read).unwrap();
    fs4::FileExt::try_lock_shared(&file).is_err()
}

/// A test's own filesystem, and the directory its locks go in.
fn rooted() -> (Rooted, PathBuf) {
    let fs = Rooted::temp().unwrap();
    let dir = PathBuf::from("/locks");
    fs.create_dir_all(&dir, Mode::DIR).unwrap();
    (fs, dir)
}

/// Write `bytes` to `path`, replacing what is there.
fn write(fs: &dyn Fs, path: &Path, bytes: &[u8]) {
    fs.write_atomic(path, bytes, Perms::KeepOr(Mode::FILE))
        .unwrap();
}

/// Whether `path` comes free for an exclusive taker within the wait every
/// taker allows, as it must once its holders are dropped. Not at once: a
/// lock is held while any descriptor on its open file description lives,
/// and a child another thread of this test binary forks (spawning
/// `mkfifo`, say) shares every descriptor here until it execs.
fn released(fs: &dyn Fs, path: &Path) -> bool {
    lock_within(fs, path, LOCK_WAIT).is_ok()
}

fn never() -> impl FnMut() -> Result<(), LockError> {
    || panic!("paused, with nobody holding the lock")
}

#[test]
fn test_an_exclusive_lock_excludes_every_other_holder_until_dropped() {
    let (fs, dir) = rooted();
    let lock_path = dir.join("sub/deeper/x.lock");
    let locked = lock_within(&fs, &lock_path, Duration::ZERO).expect("free, its directory made");
    assert!(held(&fs, &lock_path));
    assert!(held_exclusively(&fs, &lock_path));
    drop(locked);
    assert!(released(&fs, &lock_path), "released on drop");
}

#[test]
fn test_shared_locks_stand_together_and_keep_an_exclusive_one_out() {
    let (fs, dir) = rooted();
    let lock_path = dir.join("x.lock");
    let one = lock(&fs, &lock_path, Kind::Shared, None, &mut never()).unwrap();
    let two = lock(&fs, &lock_path, Kind::Shared, None, &mut never()).unwrap();
    assert!(
        held(&fs, &lock_path),
        "shared holders keep an exclusive taker out"
    );
    assert!(
        !held_exclusively(&fs, &lock_path),
        "and let a shared one in"
    );
    drop((one, two));
    assert!(released(&fs, &lock_path));
}

#[test]
fn test_nothing_is_locked_at_a_relative_path() {
    let (fs, _) = rooted();
    let refused = lock_within(&fs, Path::new("relative-lock-dir/x.lock"), Duration::ZERO);
    assert!(matches!(
        refused,
        Err(LockError::RelativePath { path }) if path == "relative-lock-dir/x.lock"
    ));
    assert!(!fs.exists(Path::new("relative-lock-dir")).unwrap());
}

#[test]
fn test_a_lock_path_that_is_not_a_regular_file_is_refused() {
    let (fs, dir) = rooted();
    let refused = lock_within(&fs, &dir, Duration::ZERO);
    assert!(matches!(
        refused,
        Err(LockError::NotRegular { path }) if path == dir.display().to_string()
    ));
}

#[cfg(unix)]
#[test]
fn test_a_fifo_at_the_lock_path_is_refused_without_opening_it() {
    // Opening a FIFO blocks until a writer comes: it is refused first.
    let (fs, dir) = rooted();
    let fifo = dir.join("x.lock");
    let made = std::process::Command::new("mkfifo")
        .arg(fs.real_path(&fifo).unwrap())
        .status()
        .unwrap();
    assert!(made.success());
    assert!(matches!(
        lock_within(&fs, &fifo, Duration::ZERO),
        Err(LockError::NotRegular { .. })
    ));
}

#[test]
fn test_a_handle_that_is_not_a_regular_file_is_refused() {
    let (fs, dir) = rooted();
    let handle = fs.open(&dir, Open::Read).unwrap();
    assert!(matches!(
        super::require_regular(&handle, &dir),
        Err(LockError::NotRegular { .. })
    ));
    let file = dir.join("f");
    write(&fs, &file, b"");
    super::require_regular(&fs.open(&file, Open::Read).unwrap(), &file).unwrap();
}

#[test]
fn test_a_lock_file_that_cannot_be_made_is_an_io_error_naming_it() {
    let (fs, dir) = rooted();
    let blocker = dir.join("file");
    write(&fs, &blocker, b"");
    let refused = lock_within(&fs, &blocker.join("x.lock"), Duration::ZERO);
    assert!(matches!(
        refused,
        Err(LockError::Io { path, .. }) if path == blocker.display().to_string()
    ));
}

#[test]
fn test_a_lock_taken_on_a_file_since_unlinked_is_taken_again() {
    // `ctxloom clean` reaps lock sidecars: a lock granted on a file that
    // was unlinked meanwhile excludes nobody, since the next taker creates
    // and locks a new file at the path.
    let (fs, dir) = rooted();
    let lock_path = dir.join("x.lock");
    let mut attempts = 0;
    let locked = take_current(&fs, &lock_path, || {
        attempts += 1;
        let file = hold(&fs, &lock_path);
        if attempts == 1 {
            // Reaped, and a new taker's file made in its place.
            fs.remove_file(&lock_path).unwrap();
            write(&fs, &lock_path, b"");
        }
        Ok(file)
    })
    .unwrap();
    assert_eq!(attempts, 2, "the first grant was on a reaped file");
    assert!(
        held(&fs, &lock_path),
        "what is held is the file at the path"
    );
    drop(locked);
    assert!(released(&fs, &lock_path));
}

#[test]
fn test_a_lock_replaced_every_time_gives_up_naming_the_file() {
    let (fs, dir) = rooted();
    let lock_path = dir.join("x.lock");
    let mut attempts = 0;
    let refused = take_current(&fs, &lock_path, || {
        attempts += 1;
        let file = hold(&fs, &lock_path);
        fs.remove_file(&lock_path).unwrap();
        Ok(file)
    });
    assert_eq!(attempts, LOCK_ATTEMPTS);
    assert!(matches!(
        refused,
        Err(LockError::Replaced { path, attempts })
            if path == lock_path.display().to_string() && attempts == LOCK_ATTEMPTS
    ));
}

#[test]
fn test_a_failed_attempt_fails_the_lock() {
    let (fs, dir) = rooted();
    let lock_path = dir.join("x.lock");
    let failure = LockError::RelativePath {
        path: "x".to_string(),
    };
    let refused = take_current(&fs, &lock_path, || Err(failure.clone()));
    assert_eq!(refused.unwrap_err(), failure);
}

#[test]
fn test_a_lock_held_past_the_wait_fails_saying_which() {
    let (fs, dir) = rooted();
    let lock_path = dir.join("x.lock");
    let holder = hold(&fs, &lock_path);
    let refused = lock_within(&fs, &lock_path, Duration::from_millis(30));
    assert!(matches!(
        refused,
        Err(LockError::Timeout { path, waited_ms: 30 }) if path == lock_path.display().to_string()
    ));
    drop(holder);
    assert!(released(&fs, &lock_path), "free once released");
}

#[test]
fn test_a_lock_is_held_while_any_descriptor_sharing_it_lives() {
    // A child forked by another thread holds a copy of every descriptor
    // until it execs (close-on-exec acts only at the exec), and flock is
    // per open file description: so a guard dropped in that window leaves
    // its lock held, and a zero wait taken then is refused. Forced here
    // with the copy the fork would make.
    let (fs, dir) = rooted();
    let lock_path = dir.join("x.lock");
    let locked = lock_within(&fs, &lock_path, Duration::ZERO).unwrap();
    let forked = locked._file.try_clone().unwrap();
    drop(locked);
    assert!(
        matches!(
            lock_within(&fs, &lock_path, Duration::ZERO),
            Err(LockError::Timeout { waited_ms: 0, .. })
        ),
        "a zero wait tries once, and the copy still holds it"
    );
    drop(forked);
    assert!(released(&fs, &lock_path), "free once the last copy closes");
}

#[test]
fn test_a_held_lock_is_waited_for_until_it_is_released() {
    for kind in [Kind::Exclusive, Kind::Shared] {
        let (fs, dir) = rooted();
        let lock_path = dir.join("x.lock");
        let mut holder = Some(hold(&fs, &lock_path));
        let mut pauses_while_held = 0;
        // Pauses once the holder let go are a child another thread forked
        // (`mkfifo`) still sharing its descriptor, until that child execs.
        let file = take_once(&fs, &lock_path, kind, None, &mut || {
            match holder.take() {
                Some(_released) => pauses_while_held += 1,
                None => std::thread::yield_now(),
            }
            Ok(())
        })
        .expect("taken once the holder let go");
        assert_eq!(pauses_while_held, 1, "{kind:?} paused while held");
        drop(file);
    }
}

#[test]
fn test_a_pause_that_gives_up_fails_the_take() {
    let (fs, dir) = rooted();
    let lock_path = dir.join("x.lock");
    let _holder = hold(&fs, &lock_path);
    let gave_up = LockError::RelativePath {
        path: "x".to_string(),
    };
    let refused = take_once(&fs, &lock_path, Kind::Exclusive, None, &mut || {
        Err(gave_up.clone())
    });
    assert_eq!(refused.unwrap_err(), gave_up);
}

#[test]
fn test_a_pause_waits_while_the_wait_lasts_and_gives_up_after() {
    let lock_path = Path::new("/locks/x.lock");
    let mut long = until(lock_path, Duration::from_secs(60));
    assert!(long().is_ok(), "a minute has not passed");
    let mut none = until(lock_path, Duration::ZERO);
    assert!(matches!(
        none(),
        Err(LockError::Timeout { waited_ms: 0, .. })
    ));
}

#[test]
fn test_a_lock_file_it_creates_records_what_it_guards_and_one_there_is_kept() {
    let (fs, dir) = rooted();
    let lock_path = dir.join("x.lock");
    let guarded = Path::new("/home/u/.claude.json");
    let locked = lock(
        &fs,
        &lock_path,
        Kind::Exclusive,
        Some(guarded),
        &mut never(),
    )
    .unwrap();
    assert_eq!(fs.read(&lock_path).unwrap(), b"/home/u/.claude.json\n");
    drop(locked);
    // Another taker's file is left exactly as it is.
    write(&fs, &lock_path, b"/elsewhere\n");
    let mut wait = until(&lock_path, LOCK_WAIT);
    drop(lock(&fs, &lock_path, Kind::Exclusive, Some(guarded), &mut wait).unwrap());
    assert_eq!(fs.read(&lock_path).unwrap(), b"/elsewhere\n");
    // A taker guarding nothing records nothing.
    let bare = dir.join("bare.lock");
    drop(lock_within(&fs, &bare, Duration::ZERO).unwrap());
    assert_eq!(fs.read(&bare).unwrap(), b"");
}

// --------------------------------------------- a foreign file's sidecar

/// A home and a file outside it to guard, and where their locks go.
struct Home {
    fs: Rooted,
}

impl Home {
    /// Working in `/work`, where the file to guard is.
    fn new() -> Home {
        Home {
            fs: Rooted::temp_with_cwd(Path::new("/work")).unwrap(),
        }
    }
    fn home(&self) -> PathBuf {
        PathBuf::from("/home/u")
    }
    fn target(&self) -> PathBuf {
        PathBuf::from("/work/.mcp.json")
    }
    fn sidecar(&self) -> path::Sidecar {
        path::sidecar_for(&self.fs, &self.home(), &self.target())
    }
}

fn short(lock: u64, reap: u64) -> Waits {
    Waits {
        lock: Duration::from_millis(lock),
        reap_interlock: Duration::from_millis(reap),
    }
}

#[test]
fn test_a_file_lock_holds_its_sidecar_and_the_reap_interlock_shared_until_dropped() {
    let at = Home::new();
    let sidecar = at.sidecar();
    let interlock = sidecar.interlock.clone().unwrap();
    let locked = lock_file(&at.fs, &at.home(), &at.target(), Waits::DEFAULT).unwrap();
    assert!(held(&at.fs, &sidecar.lock), "the sidecar, exclusively");
    assert!(held(&at.fs, &interlock), "a reap is kept out");
    assert!(
        !held_exclusively(&at.fs, &interlock),
        "the interlock is only shared"
    );
    assert_eq!(
        at.fs.read_to_string(&sidecar.lock).unwrap(),
        format!("{}\n", at.target().display()),
        "the sidecar records the file it guards"
    );
    drop(locked);
    assert!(released(&at.fs, &sidecar.lock));
    assert!(released(&at.fs, &interlock));
}

#[test]
fn test_a_relative_target_names_the_lock_its_absolute_path_does() {
    let at = Home::new();
    let locked = lock_file(
        &at.fs,
        &at.home(),
        Path::new("x/../.mcp.json"),
        Waits::DEFAULT,
    )
    .unwrap();
    assert!(held(&at.fs, &at.sidecar().lock));
    assert_eq!(
        at.fs.read_to_string(&at.sidecar().lock).unwrap(),
        "/work/.mcp.json\n"
    );
    drop(locked);
}

#[test]
fn test_a_file_lock_whose_sidecar_is_held_waits_out_the_wait_then_refuses() {
    let at = Home::new();
    let sidecar = at.sidecar();
    let holder = hold(&at.fs, &sidecar.lock);
    let refused = lock_file(&at.fs, &at.home(), &at.target(), short(30, 1000));
    assert!(matches!(
        refused,
        Err(LockError::Timeout { ref path, waited_ms: 30 })
            if *path == sidecar.lock.display().to_string()
    ));
    assert!(
        released(&at.fs, &sidecar.interlock.unwrap()),
        "the interlock is let go with the refusal"
    );
    drop(holder);
    lock_file(&at.fs, &at.home(), &at.target(), Waits::DEFAULT).expect("free once released");
}

#[test]
fn test_a_reap_in_progress_refuses_the_lock_before_its_sidecar_is_touched() {
    let at = Home::new();
    let sidecar = at.sidecar();
    let reaping = hold(&at.fs, &sidecar.interlock.clone().unwrap());
    let refused = lock_file(&at.fs, &at.home(), &at.target(), short(1000, 30));
    assert!(matches!(
        refused,
        Err(LockError::ReapInProgress { ref path, waited_ms: 30 })
            if *path == sidecar.interlock.as_ref().unwrap().display().to_string()
    ));
    assert!(
        !at.fs.exists(&sidecar.lock).unwrap(),
        "no sidecar is made during a reap"
    );
    drop(reaping);
    lock_file(&at.fs, &at.home(), &at.target(), Waits::DEFAULT).expect("taken once the reap ends");
}

#[test]
fn test_a_session_files_lock_is_the_sessions_own_with_no_interlock() {
    let at = Home::new();
    let target = at
        .home()
        .join(".ctxloom/sessions/jolly-mute-magma/work/.mcp.json");
    let locked = lock_file(&at.fs, &at.home(), &target, Waits::DEFAULT).unwrap();
    let sidecar = path::sidecar_for(&at.fs, &at.home(), &target);
    assert!(sidecar.interlock.is_none());
    assert!(held(&at.fs, &sidecar.lock));
    assert!(
        !at.fs.exists(&at.home().join(".ctxloom/locks")).unwrap(),
        "nothing in the home lock directory"
    );
    drop(locked);
}
