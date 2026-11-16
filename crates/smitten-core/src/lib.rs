//! Shared types for the Smitten game framework.
//!
//! This crate holds the plain data types that games see ([`Color`], [`Key`],
//! [`Config`], [`Filter`]) and the [`backend`] boundary that platform crates implement.
//! It has no dependencies and no platform code. Games should depend on the
//! `smitten` crate, which re-exports everything they need from here.

pub mod backend;

mod color;
mod config;
mod filter;
mod input;
mod key;

pub use color::Color;
pub use config::Config;
pub use filter::Filter;
pub use input::{GamepadAxis, GamepadButton, JoystickId, MouseButton};
pub use key::Key;
