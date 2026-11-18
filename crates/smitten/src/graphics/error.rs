use std::fmt;

/// Why a canvas, mesh or other graphics object could not be made or
/// changed, or a draw target could not be set.
///
/// These failures come from arguments the GPU cannot honor (a format it
/// cannot draw to, multisampling with mipmaps, a vertex attribute that is
/// not there) or from the GPU itself running out of room, so the message
/// is the useful part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsError {
    message: String,
}

impl GraphicsError {
    pub(crate) fn new(message: impl Into<String>) -> GraphicsError {
        GraphicsError {
            message: message.into(),
        }
    }

    /// What went wrong.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GraphicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for GraphicsError {}

/// The backend reports failures as messages.
impl From<String> for GraphicsError {
    fn from(message: String) -> GraphicsError {
        GraphicsError::new(message)
    }
}
