//! The logic of every flavor, on whatever host runs the tests.

use super::{Flavor, DEFAULT_PATHEXT};
use crate::Mode;

use Flavor::{Unix, Windows};

fn vars<'a>(set: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        set.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.to_string())
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

// ------------------------------------------------------------ home

#[test]
fn test_the_home_is_home_on_unix_and_userprofile_on_windows() {
    let env = [("HOME", "/home/u"), ("USERPROFILE", r"C:\Users\u")];
    assert_eq!(Unix.home_dir(&vars(&env)), Some("/home/u".to_string()));
    assert_eq!(
        Windows.home_dir(&vars(&env)),
        Some(r"C:\Users\u".to_string())
    );
}

#[test]
fn test_a_home_set_empty_or_unset_is_none() {
    let empty = [("HOME", ""), ("USERPROFILE", "")];
    for flavor in [Unix, Windows] {
        assert_eq!(flavor.home_dir(&vars(&empty)), None, "{flavor:?}");
        assert_eq!(flavor.home_dir(&vars(&[])), None, "{flavor:?}");
    }
    let unix_only = [("HOME", "/home/u")];
    assert_eq!(
        Windows.home_dir(&vars(&unix_only)),
        None,
        "HOME is not read"
    );
}

// ------------------------------------------------------------ environment

#[test]
fn test_windows_compares_variable_names_in_any_case() {
    assert_eq!(Unix.env_key("Path"), "Path");
    assert_eq!(Windows.env_key("Path"), "PATH");
    assert!(Windows.env_has_prefix("taskloom_config_TAG_SCHEMA", "TASKLOOM_CONFIG_"));
    assert!(!Unix.env_has_prefix("taskloom_config_TAG_SCHEMA", "TASKLOOM_CONFIG_"));
    assert!(Unix.env_has_prefix("TASKLOOM_CONFIG_TAG_SCHEMA", "TASKLOOM_CONFIG_"));
    assert!(!Windows.env_has_prefix("TASKLOOM", "TASKLOOM_CONFIG_"));
}

// ------------------------------------------------------------ PATH

#[test]
fn test_path_splits_at_each_flavors_separator() {
    assert_eq!(
        Unix.split_search_path("/usr/bin:/bin"),
        strings(&["/usr/bin", "/bin"])
    );
    assert_eq!(
        Windows.split_search_path(r"C:\Windows;C:\bin"),
        strings(&[r"C:\Windows", r"C:\bin"])
    );
    assert_eq!(
        Unix.split_search_path(r"C:\Windows;C:\bin"),
        strings(&["C", r"\Windows;C", r"\bin"]),
        "unix splits at every colon, and at nothing else"
    );
}

#[test]
fn test_a_quoted_windows_entry_keeps_its_semicolons() {
    assert_eq!(
        Windows.split_search_path(r#"C:\a;"C:\b;c";D:\d"#),
        strings(&[r"C:\a", r"C:\b;c", r"D:\d"])
    );
}

#[test]
fn test_an_empty_path_entry_is_dropped_never_the_working_directory() {
    assert_eq!(
        Unix.split_search_path("/bin::/usr/bin:"),
        strings(&["/bin", "/usr/bin"])
    );
    assert_eq!(
        Windows.split_search_path(r";C:\bin;;"),
        strings(&[r"C:\bin"])
    );
    assert!(Unix.split_search_path("").is_empty());
}

// ------------------------------------------------------------ PATHEXT (decision 7)

#[test]
fn test_unix_looks_a_command_up_by_its_own_name() {
    assert_eq!(
        Unix.candidates("taskloom", Some(".EXE")),
        strings(&["taskloom"])
    );
}

#[test]
fn test_windows_tries_each_pathext_extension_in_order() {
    assert_eq!(
        Windows.candidates("taskloom", Some(".EXE;.CMD")),
        strings(&["taskloom.EXE", "taskloom.CMD"])
    );
    let default: Vec<String> = DEFAULT_PATHEXT
        .split(';')
        .map(|ext| format!("taskloom{ext}"))
        .collect();
    assert_eq!(Windows.candidates("taskloom", None), default);
    assert_eq!(Windows.candidates("taskloom", Some("")), default);
}

#[test]
fn test_a_windows_command_with_a_pathext_extension_is_looked_up_as_named() {
    assert_eq!(
        Windows.candidates("taskloom.exe", Some(".COM;.EXE")),
        strings(&["taskloom.exe"]),
        "any case"
    );
    assert_eq!(
        Windows.candidates("taskloom.txt", Some(".EXE")),
        strings(&["taskloom.txt.EXE"]),
        "an extension PATHEXT does not name is part of the name"
    );
}

#[test]
fn test_executable_is_an_execute_bit_on_unix_and_any_file_on_windows() {
    let bits = |b| Some(Mode::from_bits(b));
    assert!(Unix.is_executable(bits(0o755)));
    assert!(Unix.is_executable(bits(0o010)));
    assert!(Unix.is_executable(bits(0o001)));
    assert!(!Unix.is_executable(bits(0o644)));
    assert!(!Unix.is_executable(None));
    assert!(Windows.is_executable(None));
    assert!(Windows.is_executable(bits(0o644)));
}

// ------------------------------------------------------------ paths

#[test]
fn test_to_slash_rewrites_only_windows_separators() {
    assert_eq!(Unix.to_slash(r"a\b/c"), r"a\b/c");
    assert_eq!(Windows.to_slash(r"C:\a\b/c"), "C:/a/b/c");
}

#[test]
fn test_a_windows_path_is_displayed_without_its_verbatim_prefix() {
    assert_eq!(Windows.display_path(r"\\?\C:\Users\u"), r"C:\Users\u");
    assert_eq!(
        Windows.display_path(r"\\?\UNC\srv\share\x"),
        r"\\srv\share\x"
    );
    assert_eq!(Windows.display_path(r"C:\Users\u"), r"C:\Users\u");
    assert_eq!(Unix.display_path(r"/a/\\?\b"), r"/a/\\?\b");
}

#[test]
fn test_windows_paths_are_the_same_path_in_any_case_or_separator() {
    let spellings = [
        r"C:\Users\U\repo",
        r"c:/users/u/repo/",
        r"\\?\C:\USERS\u\Repo",
    ];
    let keys: Vec<String> = spellings.iter().map(|p| Windows.path_key(p)).collect();
    assert!(keys.iter().all(|k| *k == keys[0]), "{keys:?}");
    assert_ne!(Windows.path_key(r"C:\a"), Windows.path_key(r"D:\a"));
    assert_eq!(
        Windows.path_key(r"C:\"),
        Windows.path_key("c:/"),
        "the top keeps its separator"
    );
    assert_ne!(Unix.path_key("/Repo"), Unix.path_key("/repo"));
    assert_eq!(Unix.path_key("/a/b"), "/a/b");
}

#[test]
fn test_an_absolute_path_splits_into_its_top_and_the_rest() {
    let split = |flavor: Flavor, path: &str| flavor.split_top(path);
    let pair = |top: &str, rest: &str| Some((top.to_string(), rest.to_string()));
    assert_eq!(split(Unix, "/a/b"), pair("/", "a/b"));
    assert_eq!(split(Unix, "/"), pair("/", ""));
    assert_eq!(split(Windows, r"C:\a\b"), pair(r"C:\", r"a\b"));
    assert_eq!(split(Windows, "C:/a"), pair("C:/", "a"));
    assert_eq!(split(Windows, r"\\srv\share\a"), pair(r"\\srv\share\", "a"));
}

#[test]
fn test_a_path_that_is_not_absolute_has_no_top() {
    for path in ["a/b", "", "./a"] {
        assert_eq!(Unix.split_top(path), None, "{path:?}");
    }
    for path in [r"a\b", r"C:a", r"\a", "/a", ""] {
        assert_eq!(Windows.split_top(path), None, "{path:?}");
    }
}

#[test]
fn test_a_windows_top_may_be_verbatim() {
    assert_eq!(
        Windows.split_top(r"\\?\C:\a"),
        Some((r"\\?\C:\".to_string(), "a".to_string()))
    );
    assert_eq!(
        Windows.split_top(r"\\?\UNC\srv\share\a"),
        Some((r"\\?\UNC\srv\share\".to_string(), "a".to_string()))
    );
}

#[test]
fn test_the_top_each_flavor_names_from() {
    assert_eq!(Unix.top(), "/");
    assert_eq!(Windows.top(), r"C:\");
    for flavor in [Unix, Windows] {
        assert_eq!(
            flavor.split_top(flavor.top()),
            Some((flavor.top().to_string(), String::new())),
            "{flavor:?}"
        );
    }
}

// ------------------------------------------------------------ lock records

#[test]
fn test_a_unix_path_is_recorded_as_its_bytes_utf8_or_not() {
    assert_eq!(Unix.path_record(b"/home/u/x"), Ok(&b"/home/u/x"[..]));
    // Go writes a unix path's bytes as they are, UTF-8 or not.
    assert_eq!(Unix.path_record(b"/a/\xff"), Ok(&b"/a/\xff"[..]));
}

#[test]
fn test_a_windows_path_is_recorded_as_utf8_and_an_unpaired_surrogate_is_refused() {
    let utf8 = "C:\\Users\\Z\u{00fc}rich".as_bytes();
    assert_eq!(Windows.path_record(utf8), Ok(utf8));
    // An unpaired surrogate (U+D800), as std encodes it on Windows: valid
    // there, but it has no UTF-8 spelling, which Go's record needs.
    let surrogate = b"C:\\a\xed\xa0\x80";
    assert_eq!(Windows.path_record(surrogate), Err(super::NotUtf8));
}

#[test]
fn test_a_relative_windows_path_keys_without_its_trailing_separator() {
    assert_eq!(Windows.path_key(r"a\B\"), r"a\b");
    assert_eq!(Windows.path_key("a/B/"), r"a\b");
}

#[test]
fn test_a_verbatim_path_that_is_no_drive_or_share_is_displayed_as_it_is() {
    let volume = r"\\?\Volume{1234}\a";
    assert_eq!(Windows.display_path(volume), volume);
    assert_eq!(Windows.display_path(r"\\srv\share\a"), r"\\srv\share\a");
}

#[test]
fn test_a_device_path_has_no_top() {
    assert_eq!(Windows.split_top(r"\\.\COM1\x"), None);
    assert_eq!(Windows.split_top(r"\\?\Volume{1234}\a"), None);
}

#[test]
fn test_an_empty_pathext_entry_is_no_extension() {
    assert_eq!(
        Windows.candidates("taskloom", Some(".EXE;;.CMD;")),
        strings(&["taskloom.EXE", "taskloom.CMD"])
    );
    let default: Vec<String> = DEFAULT_PATHEXT
        .split(';')
        .map(|ext| format!("taskloom{ext}"))
        .collect();
    assert_eq!(
        Windows.candidates("taskloom", Some(";")),
        default,
        "a PATHEXT naming no extension is as good as empty"
    );
}

#[test]
fn test_a_path_that_is_not_utf8_splits_whole_entries_and_drops_empty_ones() {
    let host = crate::host().flavor();
    let separator = if host == Unix { ":" } else { ";" };
    let odd = crate::not_unicode("/odd");
    let mut path = odd.clone();
    for part in [separator, separator, "/b"] {
        path.push(part);
    }
    assert_eq!(
        host.split_search_path_os(&path),
        [std::path::PathBuf::from(odd), "/b".into()]
    );
}

#[test]
fn test_a_utf8_path_splits_by_the_flavors_rules() {
    let path = std::ffi::OsStr::new(r#""C:\a;b";;C:\c"#);
    assert_eq!(
        Windows.split_search_path_os(path),
        [std::path::PathBuf::from(r"C:\a;b"), r"C:\c".into()]
    );
    assert_eq!(
        Unix.split_search_path_os(std::ffi::OsStr::new("/a::/b")),
        [std::path::PathBuf::from("/a"), "/b".into()]
    );
}

#[test]
fn test_the_home_variable_each_flavor_reads() {
    assert_eq!(Unix.home_var(), "HOME");
    assert_eq!(Windows.home_var(), "USERPROFILE");
}
