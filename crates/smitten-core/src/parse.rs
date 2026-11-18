//! The error from reading an enum back from its name.

use std::fmt;

/// The error from parsing a [`Key`](crate::Key), a
/// [`PixelFormat`](crate::PixelFormat) or another value from a name that
/// is not one of its names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseNameError {
    what: &'static str,
    name: String,
}

impl ParseNameError {
    /// An error for `name`, which names no `what`, such as `"key"`.
    pub fn new(what: &'static str, name: &str) -> Self {
        ParseNameError {
            what,
            name: name.to_string(),
        }
    }

    /// The name that was not recognized.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for ParseNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not the name of a {}", self.name, self.what)
    }
}

impl std::error::Error for ParseNameError {}
