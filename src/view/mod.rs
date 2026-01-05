//! View and UI layer
//!
//! This module contains all presentation and rendering components.
//!
//! Most submodules are pure Rust/ratatui and WASM-compatible.
//! Runtime-only modules (crossterm, tree-sitter, input handlers) are marked with #[cfg(feature = "runtime")].

// Pure Rust view components (WASM-compatible)
pub mod color_support;
pub mod composite_view;
pub mod dimming;
pub mod margin;
pub mod markdown;
pub mod overlay;
pub mod scroll_sync;
pub mod text_content;
pub mod theme;
pub mod virtual_text;

// Mixed view components (have internal gating for runtime-only parts)
#[cfg(any(feature = "runtime", feature = "wasm"))]
pub mod popup;
#[cfg(any(feature = "runtime", feature = "wasm"))]
pub mod ui;

// Runtime-only view components (depend on input, services, tree-sitter, crossterm)
#[cfg(feature = "runtime")]
pub mod calibration_wizard;
#[cfg(feature = "runtime")]
pub mod controls;
#[cfg(feature = "runtime")]
pub mod file_browser_input;
#[cfg(feature = "runtime")]
pub mod file_tree;
#[cfg(feature = "runtime")]
pub mod popup_input;
#[cfg(feature = "runtime")]
pub mod prompt;
#[cfg(feature = "runtime")]
pub mod prompt_input;
#[cfg(feature = "runtime")]
pub mod query_replace_input;
#[cfg(feature = "runtime")]
pub mod semantic_highlight_cache;
#[cfg(feature = "runtime")]
pub mod settings;
#[cfg(feature = "runtime")]
pub mod split;
#[cfg(feature = "runtime")]
pub mod stream;
#[cfg(feature = "runtime")]
pub mod viewport;
