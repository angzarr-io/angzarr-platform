//! A tree's kill reaches every descendant, however the leader ended; none
//! of it by sleeping: a descendant's death is seen as the EOF of a pipe it
//! held, and a hang is a failure by a bounded wait.

use std::io::ErrorKind;
use std::process::Command;

use super::{spawn_detached, spawn_tree};

#[test]
fn test_a_program_that_cannot_start_is_the_spawn_error() {
    let missing = Command::new("/nonexistent/angzarr-os-process-test");
    let err = spawn_tree(missing).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn test_a_detached_program_that_cannot_start_is_the_spawn_error() {
    let missing = Command::new("/nonexistent/angzarr-os-process-test");
    let err = spawn_detached(missing).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

/// A detached child runs the command as it was given (its arguments) and
/// its exit status is the caller's to read.
#[test]
fn test_a_detached_child_runs_and_its_status_is_read() {
    let mut command = if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "exit 3"]);
        cmd
    } else {
        let mut sh = Command::new("sh");
        sh.args(["-c", "exit 3"]);
        sh
    };
    command.stdin(std::process::Stdio::null());
    let mut child = spawn_detached(command).unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(3));
}

/// What makes [`test_detached_middle`] the middle process of
/// [`test_a_detached_child_holds_none_of_the_callers_standard_handles`]
/// rather than a no-op.
const MIDDLE: &str = "ANGZARR_OS_DETACHED_MIDDLE";

/// How long the caller's pipes may stay open once the middle process has
/// exited before a detached child is said to hold them: far longer than a
/// process takes to exit, far shorter than the child's minute of sleep.
const HELD: std::time::Duration = std::time::Duration::from_secs(10);

/// A child that sleeps for a minute, writing nothing.
fn sleeper() -> Command {
    if cfg!(windows) {
        let mut ping = Command::new("ping");
        ping.args(["-n", "61", "127.0.0.1"]);
        ping
    } else {
        let mut sleep = Command::new("sleep");
        sleep.arg("60");
        sleep
    }
}

/// Run by the test below alone, as the middle process (the caller of
/// [`spawn_detached`]), whose standard handles are that test's pipes: it
/// starts a detached sleeper with null stdio, prints its pid, and exits.
#[test]
#[ignore = "the middle process of a test, which runs it"]
// The sleeper is left running on purpose, to outlive this process; the
// test that runs this kills it.
#[allow(clippy::zombie_processes)]
fn test_detached_middle() {
    if std::env::var_os(MIDDLE).is_none() {
        return;
    }
    let mut command = sleeper();
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = spawn_detached(command).unwrap();
    println!("detached-pid={}", child.id());
}

/// What a reader of one of the middle process's pipes saw.
enum Seen {
    Line(String),
    Eof,
}

/// A detached child holds none of its caller's standard handles: once the
/// caller has exited, a reader of the caller's stdout and stderr sees their
/// EOF at once, though the child lives on (std on Windows creates every
/// child inheriting every inheritable handle, the caller's own pipes
/// among them).
#[test]
fn test_a_detached_child_holds_none_of_the_callers_standard_handles() {
    use std::io::{BufRead, BufReader, Read};
    use std::process::Stdio;
    use std::sync::mpsc;

    let name = module_path!().split_once("::").unwrap().1;
    let mut middle = Command::new(std::env::current_exe().unwrap());
    middle
        .args(["--ignored", "--exact", "--nocapture", "--test-threads=1"])
        .arg(format!("{name}::test_detached_middle"))
        .env(MIDDLE, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut middle = middle.spawn().unwrap();
    let (tx, rx) = mpsc::channel();
    let read = |pipe: Box<dyn Read + Send>, tx: mpsc::Sender<Seen>| {
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                let _ = tx.send(Seen::Line(line));
            }
            let _ = tx.send(Seen::Eof);
        })
    };
    read(Box::new(middle.stdout.take().unwrap()), tx.clone());
    read(Box::new(middle.stderr.take().unwrap()), tx);
    assert!(middle.wait().unwrap().success(), "the middle process ran");

    let mut pid = None;
    let mut eofs = 0;
    while eofs < 2 {
        match rx.recv_timeout(HELD) {
            Ok(Seen::Line(line)) => {
                // libtest may print the test's name on the same line.
                if let Some((_, id)) = line.split_once("detached-pid=") {
                    pid = Some(id.trim().to_owned());
                }
            }
            Ok(Seen::Eof) => eofs += 1,
            Err(_) => break,
        }
    }
    let pid = pid.expect("the middle process printed the detached pid");
    let mut kill = if cfg!(windows) {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/F", "/PID", &pid]);
        taskkill
    } else {
        let mut kill = Command::new("kill");
        kill.args(["-9", &pid]);
        kill
    };
    let _ = kill.stdout(Stdio::null()).stderr(Stdio::null()).status();
    assert_eq!(
        eofs, 2,
        "the caller's stdout and stderr were still open {HELD:?} after it exited: \
         the detached child holds them"
    );
}

/// Windows has no ask: nothing is sent and the call says so.
#[cfg(windows)]
#[test]
fn test_request_stop_sends_nothing_on_windows() {
    let mut command = Command::new("cmd");
    command.args(["/C", "exit 0"]);
    let mut child = command.spawn().unwrap();
    assert!(!super::request_stop(&child));
    assert!(child.wait().unwrap().success());
}

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ChildStdout, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::process::{request_stop, spawn_detached, spawn_tree, Tree};

    const SIGKILL: i32 = 9;

    /// How long a test waits for what must be prompt before it calls it a
    /// hang: far longer than a kill takes, far shorter than the 30 seconds
    /// a surviving descendant would live.
    const HANG: Duration = Duration::from_secs(15);

    fn sh(script: &str) -> Tree {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        spawn_tree(command).unwrap()
    }

    /// The pipe's next line, a pid.
    fn pid_line(out: &mut BufReader<ChildStdout>) -> u32 {
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        line.trim().parse().expect("a pid line")
    }

    /// Run `f` and fail, rather than hang, if it does not return in time.
    fn promptly<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(HANG).expect("did not finish promptly")
    }

    /// Everything left on the pipe, to its EOF: which comes only when no
    /// process holds the write end any more.
    fn to_eof(mut out: BufReader<ChildStdout>) -> Vec<u8> {
        let mut rest = Vec::new();
        out.read_to_end(&mut rest).unwrap();
        rest
    }

    #[test]
    fn test_kill_tree_kills_a_descendant_and_reaps_the_child_promptly() {
        // The leader waits on a background `sleep` that inherits stdout.
        let mut tree = sh("sleep 30 & echo $!; wait");
        let mut out = BufReader::new(tree.take_stdout().unwrap());
        let grandchild = pid_line(&mut out);
        assert_ne!(grandchild, tree.id());
        assert_eq!(tree.try_wait().unwrap(), None);

        promptly(move || {
            tree.kill_tree().unwrap();
            // The leader was killed by the group's SIGKILL, and is reaped.
            let status = tree.try_wait().unwrap().expect("reaped by the kill");
            assert_eq!(status.signal(), Some(SIGKILL));
            // The write end closes only when the `sleep` is gone too.
            assert!(to_eof(out).is_empty());
        });
    }

    #[test]
    fn test_kill_tree_reaches_a_descendant_after_the_child_has_exited() {
        // The leader exits at once; the `sleep` it started holds the pipe.
        let mut tree = sh("sleep 30 & echo $!");
        let mut out = BufReader::new(tree.take_stdout().unwrap());
        let grandchild = pid_line(&mut out);
        assert_ne!(grandchild, 0);

        promptly(move || {
            // wait() does not wait for the descendant holding the pipe.
            assert!(tree.wait().unwrap().success());
            tree.kill_tree().unwrap();
            assert!(to_eof(out).is_empty());
        });
    }

    #[test]
    fn test_kill_tree_is_idempotent_and_keeps_the_exit_status() {
        let mut tree = sh("exit 3");
        assert_eq!(tree.wait().unwrap().code(), Some(3));
        tree.kill_tree().unwrap();
        tree.kill_tree().unwrap();
        assert_eq!(tree.wait().unwrap().code(), Some(3));
        assert_eq!(tree.try_wait().unwrap().unwrap().code(), Some(3));
    }

    #[test]
    fn test_kill_tree_of_a_child_that_has_exited_unreaped_is_ok() {
        // `cat` of a closed stdin exits at once; its status is read only
        // after the kill, which reaps.
        let mut tree = sh("cat");
        drop(tree.take_stdin());
        let mut out = BufReader::new(tree.take_stdout().unwrap());
        // EOF on stdout: the child has closed it by exiting.
        assert!(out.fill_buf().unwrap().is_empty());
        tree.kill_tree().unwrap();
        assert!(tree.try_wait().unwrap().is_some());
    }

    #[test]
    fn test_terminate_tree_lets_a_child_that_stops_on_sigterm_stop() {
        // Ready is printed after the trap is set; a descendant is left.
        let mut tree = sh("trap 'exit 7' TERM; sleep 30 & echo $!; wait");
        let mut out = BufReader::new(tree.take_stdout().unwrap());
        pid_line(&mut out);

        promptly(move || {
            tree.terminate_tree(HANG).unwrap();
            // Its own exit, not the kill's, and the descendant is gone.
            let status = tree.try_wait().unwrap().unwrap();
            assert_eq!(status.code(), Some(7));
            assert!(to_eof(out).is_empty());
        });
    }

    #[test]
    fn test_terminate_tree_kills_a_child_that_ignores_sigterm_when_the_grace_is_up() {
        let mut tree = sh("trap '' TERM; echo ready; sleep 30 & echo $!; wait");
        let mut out = BufReader::new(tree.take_stdout().unwrap());
        let mut ready = String::new();
        out.read_line(&mut ready).unwrap();
        assert_eq!(ready, "ready\n");
        pid_line(&mut out);

        promptly(move || {
            // No grace: the ignored SIGTERM is followed by the kill.
            tree.terminate_tree(Duration::ZERO).unwrap();
            let status = tree.try_wait().unwrap().unwrap();
            assert_eq!(status.signal(), Some(SIGKILL));
            assert!(to_eof(out).is_empty());
        });
    }

    #[test]
    fn test_request_stop_asks_one_child_which_stops_on_its_own() {
        // Ready is printed after the trap is set, so the ask finds it set.
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("trap 'exit 7' TERM; echo ready; i=0; while [ $i -lt 30 ]; do sleep 1; i=$((i+1)); done")
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut ready = String::new();
        out.read_line(&mut ready).unwrap();
        assert_eq!(ready, "ready\n");

        assert!(request_stop(&child));
        promptly(move || {
            // Its own exit (the trap's 7), not a kill: nothing else was sent.
            let status = child.wait().unwrap();
            assert_eq!(status.code(), Some(7));
        });
    }

    #[test]
    fn test_request_stop_asks_the_child_alone_not_its_group() {
        // The child is in OUR group (a plain spawn); a SIGTERM to the
        // group would reach this test process too, and end it.
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("trap 'exit 7' TERM; echo ready; i=0; while [ $i -lt 30 ]; do sleep 1; i=$((i+1)); done")
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut ready = String::new();
        out.read_line(&mut ready).unwrap();
        assert_eq!(session_and_group(child.id()).1, session_and_group(0).1);

        assert!(request_stop(&child));
        promptly(move || assert_eq!(child.wait().unwrap().code(), Some(7)));
    }

    #[test]
    fn test_request_stop_never_signals_a_number_that_is_no_pid() {
        // Cast to pid_t it would be -1: every process the caller may signal.
        assert!(!crate::host::request_stop_pid(u32::MAX));
    }

    #[test]
    fn test_the_pipes_the_command_made_are_taken_once() {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("exit 0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut tree = spawn_tree(command).unwrap();
        assert!(tree.take_stdin().is_some());
        assert!(tree.take_stdin().is_none());
        assert!(tree.take_stdout().is_some());
        assert!(tree.take_stdout().is_none());
        assert!(tree.take_stderr().is_some());
        assert!(tree.take_stderr().is_none());
        tree.kill_tree().unwrap();
    }

    #[test]
    fn test_pipes_the_command_did_not_make_are_not_there() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("exit 0");
        command.stdin(Stdio::null()).stdout(Stdio::null());
        command.stderr(Stdio::null());
        let mut tree = spawn_tree(command).unwrap();
        assert!(tree.take_stdin().is_none());
        assert!(tree.take_stdout().is_none());
        assert!(tree.take_stderr().is_none());
        tree.kill_tree().unwrap();
    }

    #[test]
    fn test_id_is_the_childs_own_pid() {
        let mut tree = sh("echo $$; wait");
        let mut out = BufReader::new(tree.take_stdout().unwrap());
        assert_eq!(pid_line(&mut out), tree.id());
        tree.kill_tree().unwrap();
    }

    /// The session and process group of `pid` (0: this process).
    fn session_and_group(pid: u32) -> (i32, i32) {
        let pid = i32::try_from(pid).unwrap();
        // SAFETY: plain calls taking a pid; a pid not there is -1.
        let (sid, pgid) = unsafe { (libc::getsid(pid), libc::getpgid(pid)) };
        assert!(sid > 0 && pgid > 0, "pid {pid} has a session and group");
        (sid, pgid)
    }

    #[test]
    fn test_a_detached_child_leads_a_session_and_group_of_its_own() {
        // The child waits on its stdin, so it is there to be asked about.
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null());
        let mut child = spawn_detached(command).unwrap();
        let (session, group) = session_and_group(child.id());
        let ours = session_and_group(0);
        // The setsid ran before the exec, so it is done once spawn returns.
        assert_eq!(session, i32::try_from(child.id()).unwrap());
        assert_eq!(group, session, "a session's leader leads its group");
        assert_ne!(session, ours.0, "not in the spawner's session");
        assert_ne!(group, ours.1, "not in the spawner's group");
        drop(child.stdin.take());
        promptly(move || assert!(child.wait().unwrap().success()));
    }

    #[test]
    fn test_a_detached_child_has_no_controlling_terminal() {
        // A new session has none: opening /dev/tty fails, whether or not
        // the spawner has one.
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("if (exec 3</dev/tty) 2>/dev/null; then exit 1; fi; exit 0")
            .stdin(Stdio::null());
        let mut child = spawn_detached(command).unwrap();
        promptly(move || assert!(child.wait().unwrap().success()));
    }

    #[test]
    fn test_a_detached_child_keeps_the_stdio_the_command_set() {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("echo detached")
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        let mut child = spawn_detached(command).unwrap();
        let mut out = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(out, "detached\n");
        assert!(child.wait().unwrap().success());
    }
}
