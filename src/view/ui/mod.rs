//! UI rendering modules
//!
//! This module contains all rendering logic for the editor UI,
//! separated into focused submodules:
//! - `scrollbar` - Reusable scrollbar widget (WASM-compatible)
//! - `scroll_panel` - Reusable scrollable panel (WASM-compatible)
//! - `text_edit` - Text input widget (WASM-compatible)
//! - `menu` - Menu bar rendering (WASM-compatible)
//! - `tabs` - Tab bar rendering (runtime-only)
//! - `status_bar` - Status bar display (runtime-only)
//! - `suggestions` - Autocomplete UI (runtime-only)
//! - `split_rendering` - Split pane layout (runtime-only)
//! - `file_explorer` - File tree explorer (runtime-only)
//! - `file_browser` - File open dialog (runtime-only)

// Pure Rust UI widgets (WASM-compatible)
pub mod scroll_panel;
pub mod scrollbar;
pub mod text_edit;

// Runtime-only UI modules
#[cfg(feature = "runtime")]
pub mod file_browser;
#[cfg(feature = "runtime")]
pub mod file_explorer;
#[cfg(feature = "runtime")]
pub mod menu;
#[cfg(feature = "runtime")]
pub mod menu_input;
#[cfg(feature = "runtime")]
pub mod split_rendering;
#[cfg(feature = "runtime")]
pub mod status_bar;
#[cfg(feature = "runtime")]
pub mod suggestions;
#[cfg(feature = "runtime")]
pub mod tabs;
#[cfg(feature = "runtime")]
pub mod view_pipeline;

// Re-export pure types (always available)
pub use scroll_panel::{
    FocusRegion, RenderInfo, ScrollItem, ScrollState, ScrollablePanel, ScrollablePanelLayout,
};
pub use scrollbar::{render_scrollbar, ScrollbarColors, ScrollbarState};
pub use text_edit::TextEdit;

// Re-export runtime-only types
#[cfg(feature = "runtime")]
pub use file_browser::{FileBrowserLayout, FileBrowserRenderer};
#[cfg(feature = "runtime")]
pub use file_explorer::FileExplorerRenderer;
#[cfg(feature = "runtime")]
pub use menu::{context_keys, MenuContext, MenuRenderer, MenuState};
#[cfg(feature = "runtime")]
pub use menu_input::MenuInputHandler;
#[cfg(feature = "runtime")]
pub use split_rendering::SplitRenderer;
#[cfg(feature = "runtime")]
pub use status_bar::{truncate_path, StatusBarLayout, StatusBarRenderer, TruncatedPath};
#[cfg(feature = "runtime")]
pub use suggestions::SuggestionsRenderer;
#[cfg(feature = "runtime")]
pub use tabs::TabsRenderer;
