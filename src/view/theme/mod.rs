//! Theme module - color schemes and UI styling
//!
//! This module is organized into two parts:
//!
//! - **`types`**: Pure theme types (WASM-compatible)
//!   - `Theme` struct with all color fields
//!   - `ThemeFile` and related serde types for JSON parsing
//!   - Built-in themes: `dark()`, `light()`, `high_contrast()`, `nostalgia()`
//!   - `from_name_embedded()` for hardcoded theme lookup
//!
//! - **`loader`** (runtime-only): Theme loading from filesystem
//!   - `from_file()` - Load theme from JSON path
//!   - `load_builtin_theme()` - Scan themes directory
//!   - `from_name()` - Load by name (tries file first, then embedded)
//!   - `available_themes()` - List all available themes
//!
//! # Usage
//!
//! ```ignore
//! use fresh::view::theme::Theme;
//!
//! // Use embedded theme (works everywhere, including WASM)
//! let theme = Theme::dark();
//!
//! // Load theme by name (runtime only - tries file first)
//! let theme = Theme::from_name("monokai");
//!
//! // Load from specific file (runtime only)
//! let theme = Theme::from_file("~/.config/fresh/themes/custom.json")?;
//! ```

mod types;

#[cfg(feature = "runtime")]
mod loader;

// Re-export all public types
pub use types::{
    color_to_rgb, ColorDef, DiagnosticColors, EditorColors, SearchColors, SyntaxColors, Theme,
    ThemeFile, UiColors,
};

// For non-runtime builds, provide the embedded-only versions of from_name and available_themes
#[cfg(not(feature = "runtime"))]
impl Theme {
    /// Get a theme by name, defaults to dark if not found
    ///
    /// Non-runtime version: only uses hardcoded themes
    pub fn from_name(name: &str) -> Self {
        let normalized_name = name.to_lowercase().replace('_', "-");
        Self::from_name_embedded(&normalized_name)
    }

    /// Get all available theme names (embedded only)
    ///
    /// Non-runtime version: only returns hardcoded themes
    pub fn available_themes() -> Vec<String> {
        Self::embedded_themes()
    }
}
