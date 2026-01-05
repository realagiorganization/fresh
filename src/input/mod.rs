//! Input pipeline
//!
//! This module handles the input-to-action-to-event translation.
//!
//! Pure modules are WASM-compatible. Runtime-only modules depend on crossterm.

// Pure modules (WASM-compatible)
pub mod action;
pub mod command_registry;
pub mod commands;
pub mod fuzzy;
pub mod input_history;
pub mod position_history;

// Re-export pure types at module level
pub use action::{Action, KeyContext};

// Runtime-only modules (depend on crossterm or state)
#[cfg(feature = "runtime")]
pub mod actions;
#[cfg(feature = "runtime")]
pub mod buffer_mode;
#[cfg(feature = "runtime")]
pub mod composite_router;
#[cfg(feature = "runtime")]
pub mod handler;
#[cfg(feature = "runtime")]
pub mod key_translator;
#[cfg(feature = "runtime")]
pub mod keybindings;
#[cfg(feature = "runtime")]
pub mod multi_cursor;
