//! Shared types for the Smitten game framework.
//!
//! This crate holds the plain data types that games see ([`Color`], [`Key`],
//! [`Config`], [`Filter`]) and the [`backend`] boundary that platform crates implement.
//! It has no dependencies and no platform code. Games should depend on the
//! `smitten` crate, which re-exports everything they need from here.

#![forbid(unsafe_code)]

pub mod backend;

mod blend;
mod color;
mod config;
mod cull;
mod filter;
mod input;
mod key;
mod parse;
mod pixel_format;
mod stencil;
mod system;

pub use blend::{Blend, BlendAlphaMode, BlendMode};
pub use color::Color;
pub use config::{Config, Scaling};
pub use cull::{CullMode, VertexWinding};
pub use filter::{Filter, Wrap};
pub use input::{
    GamepadAxis, GamepadButton, GamepadInput, JoystickHat, JoystickId, JoystickInfo, JoystickInput,
    MouseButton, TouchId,
};
pub use key::Key;
pub use parse::ParseNameError;
pub use pixel_format::PixelFormat;
pub use stencil::{CompareMode, DepthMode, StencilAction, StencilMode};
pub use system::{
    Cursor, DisplayOrientation, FullscreenType, MessageBoxKind, PowerInfo, PowerState, WindowState,
};
