//! Low-level primitives and utilities
//!
//! This module contains syntax highlighting, ANSI handling,
//! and text manipulation utilities.
//!
//! Most submodules are pure Rust and WASM-compatible.
//! Runtime-only modules (tree-sitter based) are marked with #[cfg(feature = "runtime")].

// Pure Rust primitives (WASM-compatible)
pub mod ansi;
pub mod ansi_background;
pub mod display_width;
pub mod grammar_registry;
pub mod grapheme;
pub mod highlight_engine;
pub mod line_iterator;
pub mod line_wrapping;
pub mod snippet;
pub mod syntect_highlighter;
pub mod text_property;
pub mod visual_layout;
pub mod word_navigation;

// Runtime-only primitives (depend on tree-sitter which requires native C code)
#[cfg(feature = "runtime")]
pub mod highlighter;
#[cfg(feature = "runtime")]
pub mod indent;
#[cfg(feature = "runtime")]
pub mod semantic_highlight;
