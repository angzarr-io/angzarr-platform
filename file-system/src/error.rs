//! Why a filesystem has no working directory.

use std::fmt;

use annals_diagnostic::{Arg, Diagnostic};

/// The working directory the filesystem was made over could not be read
/// (it was deleted, or is not searchable): [`crate::Fs::cwd`] says so,
/// and a relative path, which has nothing to resolve against, fails with
/// it (an `io::Error` carrying this).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CwdError {
    /// Why it could not be read, as the OS said.
    pub reason: String,
}

impl fmt::Display for CwdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the working directory cannot be read: {}", self.reason)
    }
}

impl std::error::Error for CwdError {}

impl Diagnostic for CwdError {
    fn code(&self) -> &'static str {
        "file-system-no-cwd"
    }

    fn args(&self) -> Vec<(&'static str, Arg)> {
        vec![("reason", self.reason.as_str().into())]
    }
}

/// This crate's en-US messages, its name first: what an edge's catalog
/// embeds to render its diagnostics.
pub const EN_US: (&str, &str) = (
    "file-system",
    include_str!("../locales/en-US/file-system.ftl"),
);

/// One value of every error, for the catalog test.
pub fn diagnostic_samples() -> Vec<Box<dyn Diagnostic>> {
    vec![Box::new(CwdError {
        reason: "No such file or directory (os error 2)".to_string(),
    })]
}

#[cfg(test)]
#[path = "error.test.rs"]
mod tests;
