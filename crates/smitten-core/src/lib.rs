//! Shared types for the Smitten game framework.
//!
//! This crate holds the plain data types that games see ([`Color`], [`Key`],
//! [`Config`], [`Filter`]) and the [`backend`] boundary that platform crates implement.
//! It has no dependencies and no platform code. Games should depend on the
//! `smitten` crate, which re-exports everything they need from here.

pub mod backend;

mod blend;
mod color;
mod config;
mod filter;
mod input;
mod key;
mod stencil;
mod system;

pub use blend::BlendMode;
pub use color::Color;
pub use config::{Config, Scaling};
pub use filter::{Filter, Wrap};
pub use input::{
    GamepadAxis, GamepadButton, GamepadInput, JoystickHat, JoystickId, JoystickInfo, JoystickInput,
    MouseButton, TouchId,
};
pub use key::Key;
pub use stencil::{CompareMode, StencilAction, StencilMode};
pub use system::{
    Cursor, DisplayOrientation, FullscreenType, MessageBoxKind, PowerInfo, PowerState, WindowState,
};
