//! Type-safe Plugin API trait definition
//!
//! This module defines the `EditorApi` trait that specifies all methods
//! available to TypeScript plugins. The trait is annotated with `#[plugin_api]`
//! which generates:
//! - QuickJS binding registration code
//! - TypeScript definitions (.d.ts)
//! - JavaScript wrapper code for async methods
//!
//! ## Return Type Convention
//!
//! - `T` → Sync method, returns value directly to JS
//! - `Promise<T>` → Async method returning `Promise<T>` in JS
//! - `Thenable<T>` → Async method returning a cancellable thenable (with `.kill()`)
//!
//! ## Adding New API Methods
//!
//! 1. Add the method signature to the `EditorApi` trait
//! 2. Implement the method in `QuickJsBackend`
//! 3. Re-run `cargo build` - the proc macro regenerates bindings automatically
//! 4. The TypeScript definitions are auto-generated
//!
//! The compiler will error if the implementation doesn't match the trait.

use std::marker::PhantomData;

// ============================================================================
// Marker Types for Async Methods
// ============================================================================

/// Marker type for async methods that return `Promise<T>` in JavaScript.
///
/// Methods returning `Promise<T>` will:
/// - Generate a `_<methodName>Start` internal function that returns a callback ID
/// - Generate a JS wrapper that creates a Promise and stores resolve/reject callbacks
/// - Resolve when the Rust side calls `resolve_callback(id, result)`
pub struct Promise<T>(PhantomData<T>);

/// Marker type for async methods that return a cancellable thenable in JavaScript.
///
/// Methods returning `Thenable<T>` will:
/// - Generate a `_<methodName>Start` internal function that returns a callback ID
/// - Generate a JS wrapper that creates a thenable object with:
///   - `.result` - Promise that resolves to the result
///   - `.kill()` - Method to cancel the operation
///   - `.then()/.catch()` - Standard Promise interface
pub struct Thenable<T>(PhantomData<T>);

// ============================================================================
// Re-export types used in the API
// ============================================================================

// These are defined in api.rs and used in method signatures
pub use super::api::{BufferInfo, CursorInfo, ViewportInfo};

// ============================================================================
// The Plugin API Trait
// ============================================================================

use fresh_plugin_api_macros::{api, plugin_api};

/// The Editor API trait - all methods available to TypeScript plugins
///
/// This trait defines the complete plugin API. The `#[plugin_api]` macro generates:
/// - `EDITORAPI_TYPESCRIPT_DEFINITIONS` - Full .d.ts content
/// - `JS_ASYNC_WRAPPERS` - JavaScript code for async method wrappers
/// - `register_editorapi_bindings()` - Function to register all methods with QuickJS
#[plugin_api]
pub trait EditorApi {
    // ========================================================================
    // Status and Logging
    // ========================================================================

    /// Display a transient message in the editor's status bar
    fn set_status(&self, message: String);

    /// Log a debug message (appears in log file with RUST_LOG=debug)
    fn debug(&self, message: String);

    /// Log an error message
    fn error(&self, message: String);

    /// Log a warning message
    fn warn(&self, message: String);

    /// Log an info message
    fn info(&self, message: String);

    // ========================================================================
    // Buffer Queries (from snapshot - sync)
    // ========================================================================

    /// Get the active buffer ID (0 if none)
    fn get_active_buffer_id(&self) -> u32;

    /// Get the active split ID
    fn get_active_split_id(&self) -> u32;

    /// Get cursor byte position in active buffer
    fn get_cursor_position(&self) -> u32;

    /// Get cursor line number (1-indexed)
    fn get_cursor_line(&self) -> u32;

    /// Get file path for a buffer (empty string if no path)
    fn get_buffer_path(&self, buffer_id: u32) -> String;

    /// Get buffer length in bytes
    fn get_buffer_length(&self, buffer_id: u32) -> u32;

    /// Check if buffer has unsaved changes
    fn is_buffer_modified(&self, buffer_id: u32) -> bool;

    /// List all open buffers
    fn list_buffers(&self) -> Vec<BufferInfo>;

    /// Get primary cursor info as JSON
    fn get_primary_cursor(&self) -> String;

    /// Get all cursors as JSON
    fn get_all_cursors(&self) -> String;

    /// Get all cursor positions as array
    fn get_all_cursor_positions(&self) -> Vec<u32>;

    /// Get viewport info as JSON
    fn get_viewport(&self) -> String;

    /// Get buffer info (returns JSON, parsed in JS)
    fn get_buffer_info(&self, buffer_id: u32) -> String;

    /// Get text properties at cursor position (returns JSON array)
    fn get_text_properties_at_cursor(&self, buffer_id: u32) -> String;

    /// Get the current editor mode (for modal editing)
    fn get_editor_mode(&self) -> Option<String>;

    /// Get diff vs last saved snapshot (returns JSON)
    fn get_buffer_saved_diff(&self, buffer_id: u32) -> String;

    /// Get all LSP diagnostics (returns JSON array)
    fn get_all_diagnostics(&self) -> String;

    /// Check if a background process is still running
    fn is_process_running(&self, process_id: u64) -> bool;

    // ========================================================================
    // Configuration
    // ========================================================================

    /// Get merged config as JSON string
    fn get_config(&self) -> String;

    /// Get user config only as JSON string
    fn get_user_config(&self) -> String;

    /// Get config directory path
    fn get_config_dir(&self) -> String;

    /// Get themes directory path
    fn get_themes_dir(&self) -> String;

    /// Reload configuration from file
    fn reload_config(&self);

    /// Get the current locale
    fn get_current_locale(&self) -> String;

    // ========================================================================
    // Theme Operations
    // ========================================================================

    /// Get theme JSON schema
    fn get_theme_schema(&self) -> String;

    /// Get built-in themes as JSON
    fn get_builtin_themes(&self) -> String;

    /// Apply a theme by name
    fn apply_theme(&self, theme_name: String);

    /// Delete a theme (sync, returns success)
    fn delete_theme_sync(&self, name: String) -> bool;

    // ========================================================================
    // Clipboard
    // ========================================================================

    /// Copy text to system clipboard
    fn set_clipboard(&self, text: String);

    /// Copy text to clipboard (alias)
    fn copy_to_clipboard(&self, text: String);

    // ========================================================================
    // Text Editing (Commands)
    // ========================================================================

    /// Insert text at a byte position in a buffer
    fn insert_text(&self, buffer_id: u32, position: u32, text: String) -> bool;

    /// Delete a byte range from a buffer
    fn delete_range(&self, buffer_id: u32, start: u32, end: u32) -> bool;

    /// Insert text at the current cursor position
    fn insert_at_cursor(&self, text: String) -> bool;

    // ========================================================================
    // File Operations (Commands)
    // ========================================================================

    /// Open a file, optionally at a specific location
    fn open_file(&self, path: String, line: u32, column: u32) -> bool;

    /// Open a file in a specific split
    fn open_file_in_split(&self, split_id: u32, path: String, line: u32, column: u32) -> bool;

    /// Show a buffer in the current split
    fn show_buffer(&self, buffer_id: u32) -> bool;

    /// Close a buffer
    fn close_buffer(&self, buffer_id: u32) -> bool;

    /// Find a buffer by its file path
    fn find_buffer_by_path(&self, path: String) -> u32;

    // ========================================================================
    // Split Operations
    // ========================================================================

    /// Focus a specific split
    fn focus_split(&self, split_id: u32) -> bool;

    /// Set the buffer displayed in a split
    fn set_split_buffer(&self, split_id: u32, buffer_id: u32) -> bool;

    /// Close a split
    fn close_split(&self, split_id: u32) -> bool;

    /// Set cursor position in a buffer
    fn set_buffer_cursor(&self, buffer_id: u32, position: u32) -> bool;

    /// Set split scroll position
    fn set_split_scroll(&self, split_id: u32, top_byte: u32) -> bool;

    /// Set split ratio
    fn set_split_ratio(&self, split_id: u32, ratio: f32) -> bool;

    /// Distribute all splits evenly
    fn distribute_splits_evenly(&self) -> bool;

    // ========================================================================
    // Overlay Operations
    // ========================================================================

    /// Add a visual overlay (internal, takes JSON)
    #[api(js_name = "_addOverlayInternal")]
    fn add_overlay_internal(&self, json: String) -> bool;

    /// Clear all overlays in a namespace
    fn clear_namespace(&self, buffer_id: u32, namespace: String) -> bool;

    /// Clear all overlays from a buffer
    fn clear_all_overlays(&self, buffer_id: u32) -> bool;

    /// Remove a specific overlay by handle
    fn remove_overlay(&self, buffer_id: u32, handle: String) -> bool;

    /// Clear overlays in a byte range
    fn clear_overlays_in_range(&self, buffer_id: u32, start: u32, end: u32) -> bool;

    // ========================================================================
    // Virtual Text
    // ========================================================================

    /// Add virtual text at a position
    fn add_virtual_text(
        &self,
        buffer_id: u32,
        virtual_text_id: String,
        position: u32,
        text: String,
        r: u8,
        g: u8,
        b: u8,
        before: bool,
        use_bg: bool,
    ) -> bool;

    /// Remove virtual text by ID
    fn remove_virtual_text(&self, buffer_id: u32, virtual_text_id: String) -> bool;

    /// Remove virtual texts by prefix
    fn remove_virtual_texts_by_prefix(&self, buffer_id: u32, prefix: String) -> bool;

    /// Clear all virtual texts from a buffer
    fn clear_virtual_texts(&self, buffer_id: u32) -> bool;

    /// Clear virtual texts in a namespace
    fn clear_virtual_text_namespace(&self, buffer_id: u32, namespace: String) -> bool;

    // ========================================================================
    // Virtual Lines
    // ========================================================================

    /// Add a virtual line
    fn add_virtual_line(
        &self,
        buffer_id: u32,
        position: u32,
        text: String,
        fg_r: u8,
        fg_g: u8,
        fg_b: u8,
        bg_r: i32,
        bg_g: i32,
        bg_b: i32,
        above: bool,
        namespace: String,
        priority: i32,
    ) -> bool;

    // ========================================================================
    // Line Indicators
    // ========================================================================

    /// Set a line indicator (internal, takes JSON)
    #[api(js_name = "_setLineIndicatorInternal")]
    fn set_line_indicator_internal(&self, json: String) -> bool;

    /// Set a line indicator in the gutter
    fn set_line_indicator(
        &self,
        buffer_id: u32,
        line: u32,
        namespace: String,
        symbol: String,
        r: u8,
        g: u8,
        b: u8,
        priority: i32,
    ) -> bool;

    /// Clear line indicators in a namespace
    fn clear_line_indicators(&self, buffer_id: u32, namespace: String) -> bool;

    // ========================================================================
    // File Explorer
    // ========================================================================

    /// Set file explorer decorations (takes JSON)
    fn set_file_explorer_decorations(&self, namespace: String, decorations_json: String) -> bool;

    /// Clear file explorer decorations
    fn clear_file_explorer_decorations(&self, namespace: String) -> bool;

    // ========================================================================
    // Display
    // ========================================================================

    /// Enable/disable line numbers for a buffer
    fn set_line_numbers(&self, buffer_id: u32, enabled: bool) -> bool;

    /// Force refresh of line display
    fn refresh_lines(&self, buffer_id: u32) -> bool;

    // ========================================================================
    // Prompts
    // ========================================================================

    /// Start an interactive prompt
    fn start_prompt(&self, label: String, prompt_type: String) -> bool;

    /// Start a prompt with initial value
    fn start_prompt_with_initial(
        &self,
        label: String,
        prompt_type: String,
        initial_value: String,
    ) -> bool;

    /// Set suggestions for the current prompt (takes JSON)
    fn set_prompt_suggestions(&self, suggestions_json: String) -> bool;

    // ========================================================================
    // Commands and Actions
    // ========================================================================

    /// Register a custom command (internal)
    #[api(js_name = "_registerCommandInternal")]
    fn register_command_internal(
        &self,
        plugin_name: String,
        name: String,
        description: String,
        handler_name: String,
        context: Option<String>,
    ) -> bool;

    /// Unregister a command
    fn unregister_command(&self, name: String) -> bool;

    /// Set a custom context
    fn set_context(&self, name: String, active: bool) -> bool;

    /// Execute a built-in action
    fn execute_action(&self, action_name: String) -> bool;

    /// Execute multiple actions (takes JSON)
    fn execute_actions(&self, actions_json: String) -> bool;

    /// Set the global editor mode
    fn set_editor_mode(&self, mode: Option<String>) -> bool;

    // ========================================================================
    // Mode Definition
    // ========================================================================

    /// Define a buffer mode (takes JSON for bindings)
    fn define_mode(&self, name: String, parent: Option<String>, bindings_json: String) -> bool;

    // ========================================================================
    // Events
    // ========================================================================

    /// Subscribe to an editor event
    fn on(&self, event_name: String, handler_name: String) -> bool;

    /// Unsubscribe from an event
    fn off(&self, event_name: String, handler_name: String) -> bool;

    /// Get list of handlers for an event
    fn get_handlers(&self, event_name: String) -> Vec<String>;

    // ========================================================================
    // Environment
    // ========================================================================

    /// Get environment variable
    fn get_env(&self, name: String) -> Option<String>;

    /// Get current working directory
    fn get_cwd(&self) -> String;

    // ========================================================================
    // Path Operations
    // ========================================================================

    /// Join path segments
    fn path_join(&self, parts: Vec<String>) -> String;

    /// Get directory name
    fn path_dirname(&self, path: String) -> String;

    /// Get base name
    fn path_basename(&self, path: String) -> String;

    /// Get file extension
    fn path_extname(&self, path: String) -> String;

    /// Check if path is absolute
    fn path_is_absolute(&self, path: String) -> bool;

    // ========================================================================
    // File System (Sync)
    // ========================================================================

    /// Check if file exists
    fn file_exists(&self, path: String) -> bool;

    /// Read file contents (sync)
    fn read_file_sync(&self, path: String) -> Option<String>;

    /// Write file contents (sync)
    fn write_file_sync(&self, path: String, content: String) -> bool;

    /// Read directory contents as JSON
    fn read_dir(&self, path: String) -> String;

    /// Get file stat info as JSON
    fn file_stat(&self, path: String) -> String;

    // ========================================================================
    // i18n
    // ========================================================================

    /// Translate a plugin string (internal)
    #[api(js_name = "_pluginTranslate")]
    fn plugin_translate(&self, plugin_name: String, key: String, args_json: String) -> String;

    // ========================================================================
    // Scroll Sync
    // ========================================================================

    /// Create a scroll sync group
    fn create_scroll_sync_group(&self, group_id: u32, left_split: u32, right_split: u32) -> bool;

    /// Set scroll sync anchors (takes JSON)
    fn set_scroll_sync_anchors(&self, group_id: u32, anchors_json: String) -> bool;

    /// Remove a scroll sync group
    fn remove_scroll_sync_group(&self, group_id: u32) -> bool;

    // ========================================================================
    // Action Popup
    // ========================================================================

    /// Show an action popup (takes JSON)
    fn show_action_popup(&self, options_json: String) -> bool;

    // ========================================================================
    // LSP
    // ========================================================================

    /// Disable LSP for a language
    fn disable_lsp_for_language(&self, language: String) -> bool;

    // ========================================================================
    // View Transform
    // ========================================================================

    /// Submit a view transform (takes JSON)
    fn submit_view_transform(
        &self,
        buffer_id: u32,
        split_id: Option<u32>,
        start: u32,
        end: u32,
        tokens_json: String,
        layout_hints_json: Option<String>,
    ) -> bool;

    /// Clear view transform
    fn clear_view_transform(&self, buffer_id: u32, split_id: Option<u32>) -> bool;

    // ========================================================================
    // Async Methods - Promise<T>
    // ========================================================================

    /// Delay execution for specified milliseconds
    fn delay(&self, callback_id: u64, duration_ms: u64) -> Promise<()>;

    /// Get text from a buffer range
    fn get_buffer_text(
        &self,
        callback_id: u64,
        buffer_id: u32,
        start: u32,
        end: u32,
    ) -> Promise<String>;

    /// Read file contents (async)
    fn read_file(&self, callback_id: u64, path: String) -> Promise<String>;

    /// Write file contents (async)
    fn write_file(&self, callback_id: u64, path: String, content: String) -> Promise<()>;

    /// Delete a theme (async)
    fn delete_theme(&self, callback_id: u64, name: String) -> Promise<()>;

    /// Send an LSP request
    fn send_lsp_request(
        &self,
        callback_id: u64,
        language: String,
        method: String,
        params_json: Option<String>,
    ) -> Promise<String>;

    /// Get syntax highlights for a range
    fn get_highlights(
        &self,
        callback_id: u64,
        buffer_id: u32,
        start: u32,
        end: u32,
    ) -> Promise<String>;

    /// Kill a background process
    fn kill_process(&self, callback_id: u64, process_id: u64) -> Promise<bool>;

    /// Wait for a spawned process
    fn spawn_process_wait(&self, callback_id: u64, process_id: u64) -> Promise<String>;

    // ========================================================================
    // Async Methods - Promise (Virtual Buffers)
    // ========================================================================

    /// Create a virtual buffer in current split (takes JSON options)
    fn create_virtual_buffer(&self, callback_id: u64, options_json: String) -> Promise<u32>;

    /// Create a virtual buffer in a new split (takes JSON options)
    fn create_virtual_buffer_in_split(
        &self,
        callback_id: u64,
        options_json: String,
    ) -> Promise<String>;

    /// Create a virtual buffer in an existing split (takes JSON options)
    fn create_virtual_buffer_in_existing_split(
        &self,
        callback_id: u64,
        options_json: String,
    ) -> Promise<u32>;

    /// Set virtual buffer content (takes JSON entries)
    fn set_virtual_buffer_content(&self, buffer_id: u32, entries_json: String) -> bool;

    // ========================================================================
    // Async Methods - Promise (Composite Buffers)
    // ========================================================================

    /// Create a composite buffer (takes JSON options)
    fn create_composite_buffer(&self, callback_id: u64, options_json: String) -> Promise<u32>;

    /// Update composite buffer alignment (takes JSON hunks)
    fn update_composite_alignment(&self, buffer_id: u32, hunks_json: String) -> bool;

    /// Close a composite buffer
    fn close_composite_buffer(&self, buffer_id: u32) -> bool;

    // ========================================================================
    // Async Methods - Thenable<T> (Cancellable)
    // ========================================================================

    /// Spawn a process (cancellable)
    fn spawn_process(
        &self,
        callback_id: u64,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> Thenable<String>;

    /// Spawn a background process (cancellable)
    fn spawn_background_process(
        &self,
        callback_id: u64,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> Thenable<String>;
}

// ============================================================================
// Tests and TypeScript Generation
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that TypeScript definitions are generated
    #[test]
    fn typescript_definitions_generated() {
        // Just verify the constant exists and has content
        assert!(!EDITORAPI_TYPESCRIPT_DEFINITIONS.is_empty());
        assert!(EDITORAPI_TYPESCRIPT_DEFINITIONS.contains("interface EditorAPI"));
        println!(
            "Generated {} bytes of TypeScript definitions",
            EDITORAPI_TYPESCRIPT_DEFINITIONS.len()
        );
    }

    /// Test that JS method names list is generated
    #[test]
    fn js_methods_list_generated() {
        assert!(!EDITORAPI_JS_METHODS.is_empty());
        println!("Generated {} API methods", EDITORAPI_JS_METHODS.len());
        for method in EDITORAPI_JS_METHODS {
            println!("  - {}", method);
        }
    }

    /// Write TypeScript definitions to file (run with `cargo test write_dts -- --ignored`)
    #[test]
    #[ignore]
    fn write_dts_file() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/lib/fresh.d.ts");
        std::fs::write(path, EDITORAPI_TYPESCRIPT_DEFINITIONS).expect("Failed to write fresh.d.ts");
        println!("Wrote TypeScript definitions to {}", path);
    }

    /// Verify that all API methods have corresponding QuickJS bindings.
    ///
    /// This test reads quickjs_backend.rs and checks that for each method in
    /// EDITORAPI_JS_METHODS, there's a corresponding registration.
    ///
    /// # Pattern Matching
    ///
    /// The test looks for the following patterns:
    ///
    /// ## Sync methods
    /// Should have: `editor.set("methodName", ...)`
    ///
    /// ## Async methods (Promise<T>)
    /// Should have either:
    /// - `editor.set("_methodNameStart", ...)` (internal function), OR
    /// - `_editorCore.methodName = _wrapAsync(...)` (JS wrapper)
    ///
    /// ## Async methods (Thenable<T>)
    /// Should have either:
    /// - `editor.set("_methodNameStart", ...)` (internal function), OR
    /// - `_editorCore.methodName = _wrapAsyncThenable(...)` (JS wrapper)
    ///
    /// # Known Gaps
    ///
    /// Some methods have different names or patterns in the backend. These are
    /// tracked here to avoid false positives while we work to standardize names.
    #[test]
    fn verify_api_coverage() {
        // Read the quickjs_backend.rs source file
        let backend_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/services/plugins/backend/quickjs_backend.rs"
        );
        let backend_source =
            std::fs::read_to_string(backend_path).expect("Failed to read quickjs_backend.rs");

        // Known method name mappings (trait JS name -> backend name or pattern)
        // TODO: Standardize these names to match the trait definitions
        let name_mappings: std::collections::HashMap<&str, &str> = [
            // Methods with JSON suffix in backend
            ("listBuffers", "_listBuffersJson"),
            ("readDir", "_readDirJson"),
            (
                "getTextPropertiesAtCursor",
                "_getTextPropertiesAtCursorJson",
            ),
            // Methods with different naming
            ("deleteThemeSync", "_deleteThemeSync"),
            // Sync readFile/writeFile have different names
            ("readFileSync", "readFile"),
            ("writeFileSync", "writeFile"),
        ]
        .into_iter()
        .collect();

        // Methods that are implemented as JS wrappers only (not direct bindings)
        let js_wrapper_methods: std::collections::HashSet<&str> = [
            "listBuffers",               // Wraps _listBuffersJson with JSON.parse
            "readDir",                   // Wraps _readDirJson with JSON.parse
            "getTextPropertiesAtCursor", // Wraps _getTextPropertiesAtCursorJson
            "deleteTheme",               // Wraps _deleteThemeSync in Promise
            "addOverlay",                // Wraps _addOverlayInternal
            "setLineIndicator",          // Wraps _setLineIndicatorInternal
        ]
        .into_iter()
        .collect();

        // Known async methods (from trait return types)
        // These should have _methodNameStart internal functions
        let async_methods: std::collections::HashSet<&str> = [
            "delay",
            "getBufferText",
            "readFile",  // async readFile, not readFileSync
            "writeFile", // async writeFile, not writeFileSync
            "deleteTheme",
            "sendLspRequest",
            "getHighlights",
            "killProcess",
            "spawnProcessWait",
            "createVirtualBuffer",
            "createVirtualBufferInSplit",
            "createVirtualBufferInExistingSplit",
            "createCompositeBuffer",
            "spawnProcess",
            "spawnBackgroundProcess",
        ]
        .into_iter()
        .collect();

        // Methods that are not yet implemented in the backend
        // These are methods defined in the trait but not yet bound in quickjs_backend.rs
        let not_implemented: std::collections::HashSet<&str> = [
            // Async read/write - sync versions exist, but async versions need
            // proper file I/O infrastructure with callbacks
            "readFile",
            "writeFile",
        ]
        .into_iter()
        .collect();

        let mut missing = Vec::new();
        let mut found = Vec::new();

        for method in EDITORAPI_JS_METHODS {
            // Skip methods known to be unimplemented
            if not_implemented.contains(method) {
                continue;
            }

            // Get the backend name (may be different from JS name)
            let backend_name = name_mappings.get(method).unwrap_or(method);

            // Check for the method registration
            let is_found = if async_methods.contains(method) {
                // Async method: look for _methodStart or the JS wrapper
                let internal_name = format!("_{}Start", method);
                let wrapper_pattern = format!("_editorCore.{}", method);
                backend_source.contains(&format!("\"{}\"", internal_name))
                    || backend_source.contains(&wrapper_pattern)
                    || backend_source.contains(&format!("\"{}\"", backend_name))
            } else if js_wrapper_methods.contains(method) {
                // JS wrapper method: look for the wrapper pattern
                let wrapper_pattern = format!("_editorCore.{}", method);
                backend_source.contains(&wrapper_pattern)
                    || backend_source.contains(&format!("\"{}\"", backend_name))
            } else {
                // Sync method: look for direct registration
                backend_source.contains(&format!("\"{}\"", backend_name))
                    || backend_source.contains(&format!("\"{}\"", method))
            };

            if is_found {
                found.push(*method);
            } else {
                missing.push(*method);
            }
        }

        println!("\n=== API Coverage Report ===");
        println!("Total methods in trait: {}", EDITORAPI_JS_METHODS.len());
        println!("Not yet implemented: {}", not_implemented.len());
        println!("Found in backend: {}", found.len());
        println!("Missing from backend: {}", missing.len());

        if !missing.is_empty() {
            println!("\nMissing methods:");
            for method in &missing {
                println!("  - {}", method);
            }
            println!("\n=== MAINTENANCE NOTE ===");
            println!(
                "If a method is intentionally not implemented, add it to 'not_implemented' set."
            );
            println!("If a method has a different name in backend, add it to 'name_mappings'.");
            println!(
                "If a method is implemented via JS wrapper, add it to 'js_wrapper_methods'.\n"
            );
        }

        // This assertion will fail if there are missing methods
        // Comment out temporarily if you need to see the full report
        assert!(
            missing.is_empty(),
            "Missing API bindings for {} methods. See report above.",
            missing.len()
        );
    }
}

// ============================================================================
// Maintenance Guide and Common Pitfalls
// ============================================================================
//
// ## How to Add a New API Method
//
// 1. Add the method signature to the `EditorApi` trait above
// 2. Implement the method in `QuickJsBackend` (backend/quickjs_backend.rs)
// 3. Register the binding in `setup_global_api()`:
//    - For sync methods: `editor.set("methodName", Function::new(...))`
//    - For async methods: `editor.set("_methodNameStart", ...)` + JS wrapper
// 4. Run `cargo test verify_api_coverage` to check coverage
// 5. Run `cargo test write_dts -- --ignored` to regenerate TypeScript definitions
//
// ## Common Pitfalls to Avoid
//
// ### 1. Forgetting to Register the QuickJS Binding
//
// SYMPTOM: Method exists in trait but JS calls fail silently or throw "not a function"
// FIX: Add `editor.set("methodName", ...)` in setup_global_api()
//
// ### 2. Wrong Method Name in Binding
//
// SYMPTOM: TypeScript autocomplete shows method, but runtime error
// FIX: Ensure JS name in .set() matches camelCase version of trait method
// NOTE: Use `#[api(js_name = "...")]` for custom names that don't follow convention
//
// ### 3. Missing callback_id for Async Methods
//
// SYMPTOM: Async method doesn't resolve/reject Promise
// FIX: Async trait methods must have `callback_id: u64` as FIRST parameter
//      The proc macro filters it out from TypeScript but it must exist in Rust
//
// ### 4. Async Method Not Wrapped
//
// SYMPTOM: Calling async method returns a number instead of Promise
// FIX: Add JS wrapper in setup_global_api:
//      `_editorCore.methodName = _wrapAsync(_editorCore._methodNameStart, "methodName");`
//
// ### 5. Type Mismatch Between Trait and Binding
//
// SYMPTOM: Compile error in quickjs_backend.rs or runtime type errors
// FIX: Ensure closure parameter types match trait method signature exactly
//
// ### 6. Forgetting to Update TypeScript Definitions
//
// SYMPTOM: TypeScript shows old API, new methods missing from autocomplete
// FIX: Run `cargo test write_dts -- --ignored` after any trait changes
//
// ### 7. Return Type Marker Confusion
//
// Promise<T>  = Simple async, wraps callback in Promise
// Thenable<T> = Cancellable async, has .result property and .kill() method
//
// Choose the right marker based on whether the operation is cancellable.
//
// ## Verification Checklist for New Methods
//
// [ ] Method added to EditorApi trait with doc comment
// [ ] Return type uses correct marker (sync, Promise<T>, or Thenable<T>)
// [ ] Async methods have callback_id: u64 as first parameter
// [ ] QuickJS binding registered in setup_global_api()
// [ ] Async methods have JS wrapper (_wrapAsync or _wrapAsyncThenable)
// [ ] `cargo test verify_api_coverage` passes
// [ ] `cargo test write_dts -- --ignored` regenerates fresh.d.ts
// [ ] Actual functionality tested in a plugin
