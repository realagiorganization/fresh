# WASM Browser Mode Design

This document outlines the design for running Fresh editor in a web browser using WebAssembly (WASM), with maximum code sharing between native and WASM builds.

## Overview

The goal is to compile Fresh to WASM and run it in a browser, sharing ~85% of the codebase with the native version. This enables:
- Running Fresh in any modern browser without installation
- Single codebase - bug fixes and features apply to both platforms
- Embedding Fresh in web-based IDEs or documentation sites
- Demo/playground functionality for the project website

## Current Status

### Completed Work

The following modules have been ungated and are now shared between native and WASM:

**Model layer (`src/model/`):**
- `buffer.rs` - Text buffer with FileSystem trait abstraction
- `cursor.rs` - Cursor state and operations
- `filesystem.rs` - FileSystem trait with StdFileSystem (native) and NoopFileSystem (WASM)
- `piece_tree.rs` - Piece table data structure
- `piece_tree_diff.rs` - Diff operations
- `marker_tree.rs` - Marker tree for selections/highlights
- `marker.rs` - Marker types
- `control_event.rs` - Control events
- `document_model.rs` - Document model
- `edit.rs` - Edit operations
- `event.rs` - Event system (streaming features gated internally)
- `line_diff.rs` - Line diffing

**Primitives layer (`src/primitives/`):**
- `line_iterator.rs` - Line iteration utilities
- `snippet.rs` - Snippet handling
- `text_property.rs` - Text properties
- `word_navigation.rs` - Word navigation logic
- `grammar_registry.rs` - TextMate grammar registry (file loading gated internally)
- `highlight_engine.rs` - Syntax highlighting via syntect (TextMate grammars)
- `ansi.rs`, `ansi_background.rs` - ANSI parsing
- `display_width.rs` - Unicode display width
- `grapheme.rs` - Grapheme handling
- `line_wrapping.rs` - Line wrapping logic
- `syntect_highlighter.rs` - Syntect integration
- `visual_layout.rs` - Visual layout calculations

**View layer (`src/view/`):**
- `overlay.rs` - Overlay rendering
- `markdown.rs` - Markdown parsing and rendering
- `color_support.rs` - Color support detection
- `composite_view.rs` - Composite view rendering
- `dimming.rs` - Text dimming effects
- `margin.rs` - Line number margins
- `scroll_sync.rs` - Scroll synchronization
- `text_content.rs` - Text content rendering
- `theme.rs` - Theme system
- `virtual_text.rs` - Virtual text (inline hints)

### Key Design Patterns Used

#### 1. FileSystem Trait Abstraction

The `Buffer` type accepts a `FileSystem` implementation, allowing WASM to use a no-op implementation:

```rust
// src/model/filesystem.rs
pub trait FileSystem: Send + Sync + std::fmt::Debug {
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn write(&self, path: &Path, contents: &str) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
    // ... more methods
}

// Native: real filesystem
pub struct StdFileSystem;

// WASM: no-op implementation
pub struct NoopFileSystem;
```

#### 2. Dependency Injection for WASM Compatibility

Instead of gating entire modules, we use dependency injection to make modules work in both environments:

```rust
// src/primitives/grammar_registry.rs

/// User-provided grammar (can be passed from JavaScript in WASM)
pub struct UserGrammar {
    pub content: String,
    pub extensions: Vec<String>,
    pub scope_name: String,
}

impl GrammarRegistry {
    /// Core constructor - works in both native and WASM
    pub fn new(user_grammars: Vec<UserGrammar>) -> Self { ... }

    /// Convenience for WASM (no user grammars)
    pub fn builtin_only() -> Self { Self::new(vec![]) }

    /// Native only: loads from ~/.config/fresh/grammars/
    #[cfg(feature = "runtime")]
    pub fn load_from_config_dir() -> Self { ... }

    /// Auto-selects based on feature
    pub fn for_editor() -> Arc<Self> {
        #[cfg(feature = "runtime")]
        { Arc::new(Self::load_from_config_dir()) }
        #[cfg(not(feature = "runtime"))]
        { Arc::new(Self::builtin_only()) }
    }
}
```

#### 3. Internal Feature Gating

For modules with optional features (like debug streaming), gate only the specific code that needs it:

```rust
// src/model/event.rs

pub struct EventLog {
    events: Vec<EditorEvent>,

    /// Optional file for streaming events (runtime-only, for debugging)
    #[cfg(feature = "runtime")]
    stream_file: Option<std::fs::File>,
}

impl EventLog {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            #[cfg(feature = "runtime")]
            stream_file: None,
        }
    }

    /// Enable streaming to file (runtime-only)
    #[cfg(feature = "runtime")]
    pub fn enable_streaming<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> { ... }
}
```

#### 4. Abstract Input Events (PLANNED)

Controls currently use crossterm input types directly. To make them WASM-compatible, we'll abstract input into platform-agnostic types:

```rust
// src/model/input_events.rs (WASM-compatible)

/// Platform-agnostic key codes
pub enum KeyCode {
    Enter, Esc, Backspace, Delete,
    Left, Right, Up, Down, Home, End,
    Tab, BackTab,
    Char(char),
    F(u8),
    // ...
}

/// Key modifiers
pub struct KeyModifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Platform-agnostic key event
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

/// Mouse button
pub enum MouseButton { Left, Right, Middle }

/// Mouse event kind
pub enum MouseEventKind {
    Down(MouseButton),
    Up(MouseButton),
    Moved,
    ScrollUp,
    ScrollDown,
}

/// Platform-agnostic mouse event
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub column: u16,
    pub row: u16,
    pub modifiers: KeyModifiers,
}
```

Then controls use these abstract types, with platform-specific conversion:

```rust
// Native (runtime-only): convert crossterm events
impl From<crossterm::event::KeyEvent> for KeyEvent { ... }
impl From<crossterm::event::MouseEvent> for MouseEvent { ... }

// WASM: convert from JavaScript events (via wasm-bindgen)
impl From<web_sys::KeyboardEvent> for KeyEvent { ... }
impl From<web_sys::MouseEvent> for MouseEvent { ... }
```

This allows `view/controls/*` to be fully WASM-compatible while keeping input handling in one place.

#### 5. Abstract FsManager for File Browser (PLANNED)

The `file_tree` and `file_browser` views import from `crate::services::fs`:

```rust
// Current (runtime-only)
use crate::services::fs::{FsEntry, FsManager};
```

The architecture already has:
- `FsBackend` trait (async, already abstract!)
- `FsEntry`, `FsMetadata` data types (pure structs)
- `FsManager` uses tokio primitives (the only blocker)

**The only blocker is tokio primitives**, not the design:

```rust
// Current FsManager
use tokio::sync::{oneshot, Mutex};  // <-- tokio-specific

pub struct FsManager {
    backend: Arc<dyn FsBackend>,  // Already abstract!
    pending_dir_requests: Arc<Mutex<...>>,  // tokio Mutex
}
```

**Solution**: Replace tokio primitives with `futures` equivalents:

```rust
// src/services/fs/manager.rs (becomes WASM-compatible)
use futures::lock::Mutex;           // instead of tokio::sync::Mutex
use futures::channel::oneshot;      // instead of tokio::sync::oneshot

pub struct FsManager {
    backend: Arc<dyn FsBackend>,
    pending_dir_requests: Arc<Mutex<...>>,
}
```

Then:
- `FsManager` becomes WASM-compatible (uses `futures` crate, works with `wasm-bindgen-futures`)
- `FsBackend` remains the platform-specific part:
  - Native: `StdFsBackend` (real filesystem via tokio)
  - WASM: `WasmFsBackend` (in-memory or JS-backed)
- `file_tree`, `file_browser` can be ungated

This is cleaner than moving data types - the abstraction boundary is already correct, just needs runtime-agnostic primitives.

## Architecture: Shared `editor-core`

The key insight is to extract platform-agnostic code into modules that compile for both native and WASM targets, with platform-specific code gated behind feature flags.

```
fresh/
├── src/
│   ├── lib.rs                 # Exports all modules
│   │
│   ├── core/                  # SHARED: Platform-agnostic editor core
│   │   ├── buffer.rs          # Text buffer, undo/redo, selections
│   │   ├── editor_state.rs    # Editor state machine
│   │   ├── input.rs           # Key/mouse event handling (abstract)
│   │   └── mod.rs
│   │
│   ├── view/                  # SHARED: All rendering and UI
│   │   ├── theme.rs           # Theme system (loads embedded or from disk)
│   │   ├── render.rs          # Main render function
│   │   ├── widgets/           # All UI widgets
│   │   └── mod.rs
│   │
│   ├── model/                 # SHARED: Data structures
│   │   ├── buffer.rs          # Buffer operations (gated file I/O)
│   │   └── mod.rs
│   │
│   ├── highlight/             # SHARED: Syntax highlighting
│   │   ├── syntect.rs         # syntect-based highlighting (works in WASM)
│   │   ├── tree_sitter.rs     # tree-sitter (native only, optional)
│   │   └── mod.rs
│   │
│   ├── services/              # NATIVE ONLY: System services
│   │   ├── fs.rs              # File system (native implementation)
│   │   ├── lsp.rs             # LSP client
│   │   ├── terminal.rs        # PTY/terminal emulation
│   │   └── mod.rs
│   │
│   ├── app/                   # NATIVE ONLY: Native application
│   │   ├── mod.rs             # Crossterm-based main loop
│   │   └── ...
│   │
│   └── wasm/                  # WASM ONLY: Browser entry point
│       ├── mod.rs             # Ratzilla integration, main loop
│       ├── fs_backend.rs      # In-memory filesystem
│       └── event_adapter.rs   # Ratzilla → internal event conversion
│
└── web/
    └── styles.css             # Browser styling
```

## Code Sharing Analysis

### Shared Between Native and WASM (~85%)

| Component | Description |
|-----------|-------------|
| `view/theme.rs` | Theme definitions, color parsing, embedded themes |
| `view/render.rs` | All rendering to ratatui Frame |
| `view/widgets/*` | Tab bar, status bar, popups, file browser UI, etc. |
| `model/buffer.rs` | Text buffer core (undo/redo, cursors, selections) |
| `core/editor_state.rs` | Editor state machine, mode handling |
| `core/input.rs` | Key/mouse event processing (abstract types) |
| `highlight/syntect.rs` | Syntax highlighting via syntect (pure Rust) |
| `config.rs` | Configuration system |
| `types.rs` | Core type definitions |

### Native Only (~10%)

| Component | Why Native Only |
|-----------|-----------------|
| `services/fs.rs` | Real filesystem access |
| `services/lsp.rs` | LSP protocol, subprocess spawning |
| `services/terminal.rs` | PTY, alacritty_terminal |
| `highlight/tree_sitter.rs` | C-based grammars (optional) |
| `app/mod.rs` | Crossterm event loop |

### WASM Only (~5%)

| Component | Purpose |
|-----------|---------|
| `wasm/mod.rs` | Ratzilla backend, browser main loop |
| `wasm/fs_backend.rs` | In-memory + IndexedDB filesystem |
| `wasm/event_adapter.rs` | Ratzilla event → internal event types |

## Syntax Highlighting Strategy

### GrammarRegistry (Both platforms)

The `GrammarRegistry` manages TextMate grammars via syntect. It's been refactored to work in both native and WASM using dependency injection:

```rust
// src/primitives/grammar_registry.rs

/// User-provided grammar definition (can come from JavaScript in WASM)
pub struct UserGrammar {
    pub content: String,        // sublime-syntax or tmLanguage content
    pub extensions: Vec<String>, // file extensions (without dot)
    pub scope_name: String,     // e.g., "source.rust"
}

impl GrammarRegistry {
    /// Core constructor - works everywhere
    /// Loads: syntect defaults (100+ languages) + embedded grammars + user grammars
    pub fn new(user_grammars: Vec<UserGrammar>) -> Self { ... }

    /// WASM convenience - just built-in grammars
    pub fn builtin_only() -> Self { Self::new(vec![]) }

    /// Native only - loads user grammars from ~/.config/fresh/grammars/
    #[cfg(feature = "runtime")]
    pub fn load_from_config_dir() -> Self { ... }

    /// Auto-selects appropriate constructor based on feature
    pub fn for_editor() -> Arc<Self> { ... }
}
```

**WASM usage:**
```rust
// In WASM, use built-in grammars (100+ languages from syntect)
let registry = GrammarRegistry::builtin_only();

// Or pass grammars from JavaScript
let registry = GrammarRegistry::new(vec![
    UserGrammar {
        content: toml_grammar_string,
        extensions: vec!["toml".to_string()],
        scope_name: "source.toml".to_string(),
    }
]);
```

### syntect (Both platforms)

[syntect](https://github.com/trishume/syntect) is pure Rust and compiles to WASM. We use `fancy-regex` backend instead of `onig` for WASM compatibility:

```toml
# Cargo.toml
syntect = { version = "5", default-features = false, features = [
    "default-syntaxes",
    "default-themes",
    "regex-fancy",  # Pure Rust regex (WASM-compatible)
] }
```

### tree-sitter (Native only)

tree-sitter grammars require C compilation, not available in WASM. Use syntect as the universal highlighter, with tree-sitter as a native-only enhancement for advanced features:

```rust
// src/primitives/mod.rs

// Tree-sitter modules are runtime-only (require native C code)
#[cfg(feature = "runtime")]
pub mod tree_sitter_scope;
#[cfg(feature = "runtime")]
pub mod tree_sitter_stack;
```

## Feature Flags

```toml
# Cargo.toml

[features]
default = ["runtime"]

# Full native runtime with all features
runtime = [
    "dep:crossterm",
    "dep:tokio",
    "tree-sitter",
    "lsp",
    "terminal",
    # ... other native deps
]

# Tree-sitter highlighting (native only, optional)
tree-sitter = [
    "dep:tree-sitter",
    "dep:tree-sitter-highlight",
    # ... grammar deps
]

# LSP support (native only)
lsp = ["dep:lsp-types", "dep:url"]

# Terminal emulator (native only)
terminal = ["dep:alacritty_terminal", "dep:portable-pty"]

# WASM browser build
wasm = [
    "dep:ratzilla",
    "dep:wasm-bindgen",
    "dep:wasm-bindgen-futures",
    "dep:console_error_panic_hook",
    "dep:web-sys",
    "dep:js-sys",
    "syntect-wasm",
]

# syntect for WASM (embedded assets)
syntect-wasm = ["dep:syntect"]

[dependencies]
# Always available
syntect = { version = "5.3", optional = true, default-features = false, features = ["default-syntaxes", "default-themes"] }
ratatui = { version = "0.29", default-features = false }

# Native only
crossterm = { version = "0.29", optional = true }
tokio = { version = "1", optional = true, features = ["full"] }

# WASM only
ratzilla = { version = "0.2", optional = true }
wasm-bindgen = { version = "0.2", optional = true }
```

## Implementation Plan

### Phase 1: Core Infrastructure ✅ COMPLETED

1. ✅ **FileSystem trait abstraction**
   - Created `FileSystem` trait in `model/filesystem.rs`
   - Implemented `StdFileSystem` for native, `NoopFileSystem` for WASM
   - Buffer now stores filesystem internally

2. ✅ **Buffer and piece_tree WASM-compatible**
   - `model/buffer.rs`, `model/piece_tree.rs` now compile for WASM
   - File I/O abstracted via FileSystem trait

3. ✅ **Ungate pure Rust model modules**
   - `marker_tree.rs`, `marker.rs`, `overlay.rs`
   - `control_event.rs`, `document_model.rs`, `edit.rs`, `line_diff.rs`
   - `event.rs` (with streaming features gated internally)

### Phase 2: Primitives Layer ✅ COMPLETED

4. ✅ **Ungate pure Rust primitives**
   - `line_iterator.rs`, `snippet.rs`, `text_property.rs`, `word_navigation.rs`

5. ✅ **GrammarRegistry refactored for WASM**
   - Dependency injection pattern: `new(user_grammars)` constructor
   - `builtin_only()` for WASM, `load_from_config_dir()` for native
   - File loading methods gated internally

6. ✅ **syntect with fancy-regex for WASM**
   - Replaced `onig` (C library) with `fancy-regex` (pure Rust)
   - All 100+ built-in syntaxes available in WASM

### Phase 3: View Layer (IN PROGRESS)

7. ✅ **Ungate pure view rendering modules**
   - `markdown.rs`, `theme.rs`, `overlay.rs`, `margin.rs`, `virtual_text.rs`
   - `color_support.rs`, `composite_view.rs`, `dimming.rs`, `scroll_sync.rs`, `text_content.rs`

8. ⬜ **Abstract input events for controls**
   - Create `model/input_events.rs` with platform-agnostic types
   - Update `view/controls/*` to use abstract types
   - Add crossterm conversion (runtime-only)
   - This unblocks: `text_input`, `checkbox`, `button`, `dropdown`, etc.

9. ⬜ **Abstract FsManager with runtime-agnostic primitives**
   - Replace `tokio::sync::{Mutex, oneshot}` with `futures::{lock::Mutex, channel::oneshot}`
   - Move `FsManager`, `FsEntry`, `FsMetadata` to WASM-compatible location
   - Keep `FsBackend` implementations platform-specific (StdFsBackend, WasmFsBackend)
   - This unblocks: `file_tree`, `file_browser`, and entire `services/fs` module

10. ⬜ **Ungate pure UI components** (quick wins)
    - `ui/scrollbar.rs` - pure ratatui widgets, no dependencies
    - `ui/scroll_panel.rs` - only uses `view/theme` (already ungated)
    - `ui/text_edit.rs` - only uses `primitives/word_navigation` (already ungated)
    - `ui/menu.rs` - mostly pure (uses config, theme)

    **UI Module Dependency Analysis:**
    ```
    Pure (can ungate now):
    ├── scrollbar.rs     → pure ratatui
    ├── scroll_panel.rs  → view/theme ✓
    ├── text_edit.rs     → primitives/word_navigation ✓
    └── menu.rs          → config, view/theme ✓

    Blocked by crossterm/input (needs #8):
    └── menu_input.rs    → crossterm, input/handler

    Blocked by EditorState (needs #11):
    ├── tabs.rs          → app::BufferMetadata, state::EditorState
    ├── status_bar.rs    → app::WarningLevel, state::EditorState
    └── split_rendering.rs → app, state, services::plugins

    Blocked by file_tree (needs #9):
    └── file_explorer.rs → view/file_tree (gated)

    Blocked by prompt (needs #8):
    └── suggestions.rs   → input/commands, view/prompt (gated)

    Blocked by services (needs #13):
    ├── view_pipeline.rs → services::plugins::api
    └── file_browser.rs  → app::file_open, app::HoverTarget
    ```

11. ⬜ **Abstract EditorState access for UI components**

    Many UI components (tabs, status_bar, split_rendering, file_browser) need access to EditorState, but EditorState contains runtime-only fields:
    - `indent_calculator: IndentCalculator` - tree-sitter based
    - `semantic_highlight_cache` - tree-sitter based
    - Imports from gated modules (popup, semantic_highlight)

    **Solution**: Create `EditorStateView` trait with only the fields UI needs:
    ```rust
    // src/model/editor_state_view.rs (WASM-compatible)
    pub trait EditorStateView {
        fn buffer(&self) -> &Buffer;
        fn cursors(&self) -> &Cursors;
        fn mode(&self) -> &str;
        fn highlighter(&self) -> &HighlightEngine;
        fn overlays(&self) -> &OverlayManager;
        // ... other pure view data
    }
    ```

    Then UI components use `impl EditorStateView` instead of `&EditorState`:
    - Trait is WASM-compatible (pure Rust types)
    - Concrete impl can be runtime-gated
    - UI components become ungatable

12. ⬜ **Factor out shared types from app/types.rs**

    `BufferMetadata`, `HoverTarget`, `ViewLineMapping` are used by UI but contain runtime deps:
    - `lsp_types::Uri` - LSP dependency
    - `crate::input::keybindings::Action` - input handling

    **Solution**: Extract pure data types to `model/buffer_types.rs`:
    ```rust
    // Remove lsp_types::Uri, use String for URI
    // Remove Action references, or make Action abstract
    ```

13. ⬜ **Abstract plugins API for view_pipeline**

    `view_pipeline.rs` and `split_rendering.rs` use `crate::services::plugins::api`:
    - Needs trait abstraction or stubbed implementation for WASM

### Phase 4: State Sharing (PLANNED)

14. ⬜ **Share EditorState between native and WASM**
    - Extract platform-agnostic state from current Editor
    - Create shared state machine

### Phase 5: Full Integration (PLANNED)

15. ⬜ **WASM entry point using shared core**
    - Update `wasm/mod.rs` to use shared modules
    - Replace WasmEditorState with shared EditorState

16. ⬜ **JavaScript interop**
    - Expose APIs for file loading, grammar injection
    - Event handling from browser

17. ⬜ **Testing and polish**
    - Visual parity verification
    - Performance optimization
    - Browser compatibility testing

## Module Gating Pattern

```rust
// Example: src/view/theme.rs

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

// Always available
#[derive(Debug, Clone)]
pub struct Theme {
    pub editor_bg: Color,
    pub editor_fg: Color,
    // ... all color fields
}

impl Theme {
    /// Default theme (always available)
    pub fn default() -> Self {
        Self::dracula()
    }

    /// Built-in Dracula theme
    pub fn dracula() -> Self {
        Self {
            editor_bg: Color::Rgb(40, 42, 54),
            editor_fg: Color::Rgb(248, 248, 242),
            // ...
        }
    }

    /// List of embedded theme names
    pub fn embedded_themes() -> &'static [&'static str] {
        &["dracula", "monokai", "solarized-dark", "solarized-light"]
    }
}

// Native only: file-based theme loading
#[cfg(feature = "runtime")]
impl Theme {
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&content)
    }

    pub fn available_themes() -> Vec<String> {
        // Scan themes directory
        // ...
    }
}
```

## Build Commands

```bash
# Native build (default)
cargo build --release

# WASM build
./scripts/wasm-build.sh
# or manually:
wasm-pack build --target web --no-default-features --features wasm

# Test both
cargo test                                    # Native tests
wasm-pack test --headless --firefox          # WASM tests
```

## Expected Outcomes

### Code Sharing
- **~85% shared** between native and WASM
- Single source of truth for rendering, themes, buffer operations
- Bug fixes automatically apply to both platforms

### Feature Parity

| Feature | Native | WASM |
|---------|--------|------|
| Text editing | ✅ | ✅ |
| Undo/redo | ✅ | ✅ |
| Themes | ✅ | ✅ |
| Syntax highlighting | ✅ (tree-sitter + syntect) | ✅ (syntect) |
| Multiple buffers | ✅ | ✅ |
| Split views | ✅ | ✅ |
| File browser UI | ✅ | ✅ |
| Search/replace | ✅ | ✅ |
| Mouse support | ✅ | ✅ |
| LSP | ✅ | ❌ (maybe via proxy) |
| Terminal emulator | ✅ | ❌ |
| Real filesystem | ✅ | ❌ (in-memory/IndexedDB) |
| Plugins | ✅ | ❌ (future: wasm plugins) |

### Binary Size
- Native: ~15-20MB (with all features)
- WASM: ~2-3MB (compressed), ~5-8MB uncompressed

## References

- [Ratzilla](https://github.com/orhun/ratzilla) - Browser terminal backend
- [syntect](https://github.com/trishume/syntect) - Pure Rust syntax highlighting
- [wasm-pack](https://rustwasm.github.io/wasm-pack/) - WASM build tool
