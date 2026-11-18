//! The crate-level error type, for code that loads several kinds of thing
//! and wants to pass any failure up with `?`.

use std::error::Error as StdError;
use std::fmt;

/// Any error, boxed, with the chain of causes it came from.
///
/// Every error type in Smitten, `std::io::Error` and any other type that
/// implements [`std::error::Error`] converts into it, so [`Game::load`]
/// and other loading code can use `?` on all of them:
///
/// ```no_run
/// use smitten::graphics::{self, Font, Image};
///
/// struct Assets {
///     player: Image,
///     font: Font,
///     level: String,
/// }
///
/// fn load_assets() -> smitten::Result<Assets> {
///     Ok(Assets {
///         player: Image::load("player.png")?,
///         font: Font::load("font.ttf", 24.0)?,
///         level: smitten::filesystem::read_string("level1.txt")?,
///     })
/// }
/// ```
///
/// Like `anyhow::Error`, it does not implement [`std::error::Error`]
/// itself, which is what lets it convert from every type that does. Use
/// [`Error::downcast_ref`] to look for a particular error, and
/// [`Error::chain`] to walk its causes.
///
/// [`Game::load`]: crate::Game::load
pub struct Error(Box<dyn StdError + 'static>);

impl Error {
    /// An error with only a message, for failures a game finds itself.
    pub fn msg(message: impl fmt::Display) -> Error {
        Error(Box::new(Message(message.to_string())))
    }

    /// The error, if it is an `E`.
    pub fn downcast_ref<E: StdError + 'static>(&self) -> Option<&E> {
        self.0.downcast_ref()
    }

    /// The error, then what caused it, then what caused that, and so on.
    pub fn chain(&self) -> impl Iterator<Item = &(dyn StdError + 'static)> {
        let first: &(dyn StdError + 'static) = &*self.0;
        std::iter::successors(Some(first), |&error| error.source())
    }

    /// The error as a trait object.
    pub fn as_dyn(&self) -> &(dyn StdError + 'static) {
        &*self.0
    }
}

impl<E: StdError + 'static> From<E> for Error {
    fn from(error: E) -> Error {
        Error(Box::new(error))
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// Shows the error, and with `{:#}` each of its causes after it.
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)?;
        if formatter.alternate() {
            for cause in self.chain().skip(1) {
                write!(formatter, ": {cause}")?;
            }
        }
        Ok(())
    }
}

/// The error [`Error::msg`] makes.
#[derive(Debug)]
struct Message(String);

impl fmt::Display for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl StdError for Message {}

/// A [`std::result::Result`] whose error defaults to [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Outer(std::io::Error);

    impl fmt::Display for Outer {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("could not load the level")
        }
    }

    impl StdError for Outer {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            Some(&self.0)
        }
    }

    fn fails() -> Result<()> {
        Err(Outer(std::io::Error::other("disk on fire")))?;
        Ok(())
    }

    #[test]
    fn question_mark_converts_any_error_and_keeps_its_causes() {
        let error = fails().unwrap_err();
        assert_eq!(error.to_string(), "could not load the level");
        assert_eq!(
            format!("{error:#}"),
            "could not load the level: disk on fire"
        );
        assert!(error.downcast_ref::<Outer>().is_some());
        assert_eq!(error.chain().count(), 2);
    }

    #[test]
    fn a_message_is_an_error_too() {
        assert_eq!(Error::msg("no levels found").to_string(), "no levels found");
    }
}
