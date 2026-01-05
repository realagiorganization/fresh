// Editor library - exposes all core modules for testing

// Initialize i18n with translations from locales/ directory
rust_i18n::i18n!("locales", fallback = "en");

pub mod i18n;

#[cfg(feature = "plugins")]
pub mod v8_init;

// Core types and config are always available (needed for schema generation)
pub mod config;
pub mod partial_config;
pub mod types;

// Runtime-only modules (require the "runtime" feature)
#[cfg(feature = "runtime")]
pub mod config_io;
#[cfg(feature = "runtime")]
pub mod session;

// Modules with internal gating (pure types ungated, runtime code gated internally)
pub mod app;
pub mod input;
pub mod model;
pub mod primitives;
pub mod services;
pub mod state;
pub mod view;

// WASM browser build modules
#[cfg(feature = "wasm")]
pub mod wasm;
