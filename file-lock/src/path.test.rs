//! Where a lock lives.

use std::path::{Path, PathBuf};

use file_system::{Fs, Mode, Rooted};

use super::{absolute, clean, flat_name, sidecar_for, valid_harp, Sidecar};

/// A deep directory: its paths' flat names are cut to their tails.
const DEEP: &str =
    "/home/u/checkouts/a-checkout-with-a-long-name/scratch/0123456789abcdef-0123-4567-89ab-cdef01234567";

/// An empty filesystem of the test's own.
fn empty() -> Rooted {
    Rooted::temp().unwrap()
}

/// Each, as Go's derivation gives it: the tail cut to 96 bytes, then the
/// whole path's hash.
#[test]
fn test_a_flat_name_is_gos() {
    assert_eq!(
        flat_name(&format!("{DEEP}/tmp.L0iKUu4t5Y/home/.claude.json")),
        "-name__scratch__0123456789abcdef-0123-4567-89ab-cdef01234567__tmp.L0iKUu4t5Y__home__.claude.json__6cf0c83be30d6300"
    );
    assert_eq!(
        flat_name(&format!("{DEEP}/tmp.qBAVn0XNgO/home/.claude.json")),
        "-name__scratch__0123456789abcdef-0123-4567-89ab-cdef01234567__tmp.qBAVn0XNgO__home__.claude.json__e38e07bd3f26586a"
    );
}

#[test]
fn test_a_flat_name_escapes_what_ntfs_refuses_and_cuts_on_a_character() {
    let name = flat_name("C:/a|b\u{1}");
    assert!(name.starts_with("C%3A__a%7Cb%01__"), "{name}");
    assert_eq!(name.len(), "C%3A__a%7Cb%01__".len() + 16);
    let deep = format!("/{}", "\u{e9}".repeat(60));
    let name = flat_name(&deep);
    let tail = &name[..name.len() - 18];
    assert!(tail.len() <= 96, "{}", tail.len());
    assert!(tail.chars().all(|c| c == '\u{e9}'), "{tail}");
}

/// The flat name of each of these, as the reference derivation gave it.
#[test]
fn test_a_flat_name_cut_inside_a_character_steps_forward_as_gos() {
    assert_eq!(
        flat_name(&format!("/{}x", "\u{e9}".repeat(60))),
        format!("{}x__e3ac857167b96b7b", "\u{e9}".repeat(47))
    );
    assert_eq!(
        flat_name(&"a".repeat(96)),
        format!("{}__ee4caa5518a866f3", "a".repeat(96))
    );
    assert_eq!(
        flat_name(&"a".repeat(97)),
        format!("{}__2a2c60fe96afac91", "a".repeat(96))
    );
}

#[test]
fn test_a_path_is_cleaned_as_gos_filepath_clean_cleans_it() {
    assert_eq!(clean(Path::new("/a/./b/../c/")), Path::new("/a/c"));
    assert_eq!(clean(Path::new("/../x")), Path::new("/x"));
    assert_eq!(clean(Path::new("../a/../../b")), Path::new("../../b"));
    assert_eq!(clean(Path::new("a/..")), Path::new("."));
    assert_eq!(clean(Path::new("")), Path::new("."));
}

#[test]
fn test_a_relative_path_is_made_absolute_against_the_working_directory() {
    let fs = Rooted::temp_with_cwd(Path::new("/work/here")).unwrap();
    assert_eq!(
        absolute(&fs, Path::new("a/./b/../.mcp.json")).unwrap(),
        Path::new("/work/here/a/.mcp.json")
    );
    assert_eq!(
        absolute(&fs, Path::new("/x/../y")).unwrap(),
        Path::new("/y")
    );
}

/// The home path and the reap interlock's path of each of these, with HOME
/// at /home/u, as the reference derivation gave them.
#[test]
fn test_a_file_outside_every_session_is_locked_in_the_home_lock_directory() {
    let home = Path::new("/home/u");
    assert_eq!(
        sidecar_for(&empty(), home, Path::new("/home/u/work/.mcp.json")),
        Sidecar {
            lock: PathBuf::from(
                "/home/u/.ctxloom/locks/__home__u__work__.mcp.json__54e9de5e9d028e9c.lock"
            ),
            interlock: Some(PathBuf::from("/home/u/.ctxloom/locks/reap.interlock")),
        }
    );
    assert_eq!(
        sidecar_for(&empty(), home, Path::new("/home/u/.claude.json")).lock,
        Path::new("/home/u/.ctxloom/locks/__home__u__.claude.json__93b778aeca6eb169.lock")
    );
}

#[test]
fn test_a_file_inside_a_session_is_locked_in_that_sessions_own_lock_directory() {
    let home = Path::new("/home/u");
    let target = Path::new("/home/u/.ctxloom/sessions/jolly-mute-magma/work/.mcp.json");
    assert_eq!(
        sidecar_for(&empty(), home, target),
        Sidecar {
            lock: PathBuf::from(
                "/home/u/.ctxloom/sessions/jolly-mute-magma/locks/work__.mcp.json__2087cb7edc82acdc.lock"
            ),
            interlock: None,
        }
    );
}

#[test]
fn test_the_sessions_root_a_harp_dir_and_the_files_beside_them_are_in_no_session() {
    let home = Path::new("/home/u");
    assert_eq!(
        sidecar_for(&empty(), home, Path::new("/home/u/.ctxloom/sessions/jolly-mute-magma")).lock,
        Path::new(
            "/home/u/.ctxloom/locks/__home__u__.ctxloom__sessions__jolly-mute-magma__a7979502be344972.lock"
        )
    );
    assert_eq!(
        sidecar_for(
            &empty(),
            home,
            Path::new("/home/u/.ctxloom/sessions/ bad/a")
        )
        .lock,
        Path::new(
            "/home/u/.ctxloom/locks/__home__u__.ctxloom__sessions__ bad__a__cbeb694342752469.lock"
        )
    );
    for outside in [
        "/home/u/.ctxloom/sessions",
        "/home/u/.ctxloom/sessions/jolly-mute-magma",
        "/home/u/.ctxloom/sessionsx/jolly/a",
        "/home/u/.ctxloom/sessions/ bad/a",
    ] {
        let sidecar = sidecar_for(&empty(), home, Path::new(outside));
        assert_eq!(
            sidecar.lock,
            Path::new(&format!(
                "/home/u/.ctxloom/locks/{}.lock",
                flat_name(outside)
            )),
            "{outside}"
        );
        assert!(sidecar.interlock.is_some(), "{outside}");
    }
}

#[test]
fn test_a_harp_is_one_path_component_with_visible_edges() {
    for good in ["jolly-mute-magma", "a b", "x"] {
        assert!(valid_harp(good), "{good:?}");
    }
    for bad in [
        "", ".", "..", "a/b", "a\\b", "a:b", "a\u{1}b", "a\u{7f}", "\u{1f}", " a", "a ",
    ] {
        assert!(!valid_harp(bad), "{bad:?}");
    }
}

#[cfg(unix)]
#[test]
fn test_a_session_file_under_a_linked_sessions_root_names_the_lock_as_home_spells_it() {
    let fs = empty();
    let home = Path::new("/home/u");
    let real = Path::new("/elsewhere");
    fs.create_dir_all(&home.join(".ctxloom"), Mode::DIR)
        .unwrap();
    fs.create_dir_all(&real.join("jolly-mute-magma/work"), Mode::DIR)
        .unwrap();
    fs.symlink_dir(real, &home.join(".ctxloom/sessions"))
        .unwrap();
    let sidecar = sidecar_for(&fs, home, &real.join("jolly-mute-magma/work/.mcp.json"));
    assert_eq!(
        sidecar,
        Sidecar {
            lock: home.join(format!(
                ".ctxloom/sessions/jolly-mute-magma/locks/{}.lock",
                flat_name("work/.mcp.json")
            )),
            interlock: None,
        }
    );
    // Outside the resolved root as well as the spelled one.
    assert!(sidecar_for(&fs, home, Path::new("/x/.mcp.json"))
        .interlock
        .is_some());
}
