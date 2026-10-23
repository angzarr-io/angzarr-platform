//! What only the real filesystem does: name each path as given.

use std::path::Path;

use super::Os;
use crate::Fs;

#[test]
fn test_real_path_is_the_path_as_given() {
    let temp = tempfile::tempdir().unwrap();
    // Not resolved: a lock is named by the path it guards, however its
    // links stand, and SQLite opens what it was handed.
    let given = temp.path().join("not-yet").join("..").join("state.db");
    assert_eq!(Os.real_path(&given).unwrap(), given);
    let plain = Path::new(if cfg!(windows) { r"C:\x\y" } else { "/x/y" });
    assert_eq!(Os.real_path(plain).unwrap(), plain);
}
