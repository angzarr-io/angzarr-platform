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

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ChildStdout, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::process::{spawn_detached, spawn_tree, Tree};

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
