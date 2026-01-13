//! QuickJS JavaScript runtime backend for TypeScript plugins
//!
//! This module provides a JavaScript runtime using QuickJS for executing
//! TypeScript plugins. TypeScript is transpiled to JavaScript using oxc.

use crate::config_io::DirectoryContext;
use crate::input::commands::{Command, CommandSource};
use crate::input::keybindings::Action;
use crate::model::event::{BufferId, SplitId};
use crate::primitives::text_property::TextPropertyEntry;
#[cfg(test)]
use crate::services::plugins::api::CursorInfo;
use crate::services::plugins::api::{BufferInfo, EditorStateSnapshot, PluginCommand, PluginResponse};
use crate::services::plugins::transpile::{
    bundle_module, has_es_imports, has_es_module_syntax, strip_imports_and_exports,
    transpile_typescript,
};
use crate::view::overlay::OverlayNamespace;
use anyhow::{anyhow, Result};
use rquickjs::{Context, Function, Object, Runtime, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{mpsc, Arc, RwLock};

/// Convert a QuickJS Value to serde_json::Value
fn js_to_json(ctx: &rquickjs::Ctx<'_>, val: Value<'_>) -> serde_json::Value {
    use rquickjs::Type;
    match val.type_of() {
        Type::Null | Type::Undefined | Type::Uninitialized => serde_json::Value::Null,
        Type::Bool => val
            .as_bool()
            .map(serde_json::Value::Bool)
            .unwrap_or(serde_json::Value::Null),
        Type::Int => val
            .as_int()
            .map(|n| serde_json::Value::Number(n.into()))
            .unwrap_or(serde_json::Value::Null),
        Type::Float => val
            .as_float()
            .and_then(|f| serde_json::Number::from_f64(f))
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Type::String => val
            .as_string()
            .and_then(|s| s.to_string().ok())
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
        Type::Array => {
            if let Some(arr) = val.as_array() {
                let items: Vec<serde_json::Value> = arr
                    .iter()
                    .filter_map(|item| item.ok())
                    .map(|item| js_to_json(ctx, item))
                    .collect();
                serde_json::Value::Array(items)
            } else {
                serde_json::Value::Null
            }
        }
        Type::Object | Type::Constructor | Type::Function => {
            if let Some(obj) = val.as_object() {
                let mut map = serde_json::Map::new();
                for key in obj.keys::<String>().flatten() {
                    if let Ok(v) = obj.get::<_, Value>(&key) {
                        map.insert(key, js_to_json(ctx, v));
                    }
                }
                serde_json::Value::Object(map)
            } else {
                serde_json::Value::Null
            }
        }
        _ => serde_json::Value::Null,
    }
}

/// Convert a serde_json::Value to a QuickJS Value
fn json_to_js<'js>(
    ctx: &rquickjs::Ctx<'js>,
    val: serde_json::Value,
) -> rquickjs::Result<Value<'js>> {
    match val {
        serde_json::Value::Null => Ok(Value::new_null(ctx.clone())),
        serde_json::Value::Bool(b) => Ok(Value::new_bool(ctx.clone(), b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::new_int(ctx.clone(), i as i32))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::new_float(ctx.clone(), f))
            } else {
                Ok(Value::new_null(ctx.clone()))
            }
        }
        serde_json::Value::String(s) => {
            Ok(rquickjs::String::from_str(ctx.clone(), &s)?.into_value())
        }
        serde_json::Value::Array(arr) => {
            let js_arr = rquickjs::Array::new(ctx.clone())?;
            for (i, item) in arr.into_iter().enumerate() {
                js_arr.set(i, json_to_js(ctx, item)?)?;
            }
            Ok(js_arr.into_value())
        }
        serde_json::Value::Object(map) => {
            let obj = rquickjs::Object::new(ctx.clone())?;
            for (key, value) in map {
                obj.set(key.as_str(), json_to_js(ctx, value)?)?;
            }
            Ok(obj.into_value())
        }
    }
}

/// Get text properties at cursor position, returning JSON string
fn get_text_properties_at_cursor_json(
    snapshot: &Arc<RwLock<EditorStateSnapshot>>,
    buffer_id: u32,
) -> String {
    let empty = "[]".to_string();
    let snap = match snapshot.read() {
        Ok(s) => s,
        Err(_) => return empty,
    };
    let buffer_id_typed = BufferId(buffer_id as usize);
    let cursor_pos = match snap
        .buffer_cursor_positions
        .get(&buffer_id_typed)
        .copied()
        .or_else(|| {
            if snap.active_buffer_id == buffer_id_typed {
                snap.primary_cursor.as_ref().map(|c| c.position)
            } else {
                None
            }
        }) {
        Some(pos) => pos,
        None => return empty,
    };

    let properties = match snap.buffer_text_properties.get(&buffer_id_typed) {
        Some(p) => p,
        None => return empty,
    };

    // Find all properties at cursor position
    let result: Vec<_> = properties
        .iter()
        .filter(|prop| prop.start <= cursor_pos && cursor_pos < prop.end)
        .map(|prop| {
            serde_json::Value::Object(
                prop.properties
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        })
        .collect();

    serde_json::to_string(&result).unwrap_or(empty)
}

/// Convert a JavaScript value to a string representation for console output
fn js_value_to_string(ctx: &rquickjs::Ctx<'_>, val: &Value<'_>) -> String {
    use rquickjs::Type;
    match val.type_of() {
        Type::Null => "null".to_string(),
        Type::Undefined => "undefined".to_string(),
        Type::Bool => val.as_bool().map(|b| b.to_string()).unwrap_or_default(),
        Type::Int => val.as_int().map(|n| n.to_string()).unwrap_or_default(),
        Type::Float => val.as_float().map(|f| f.to_string()).unwrap_or_default(),
        Type::String => val
            .as_string()
            .and_then(|s| s.to_string().ok())
            .unwrap_or_default(),
        Type::Object | Type::Exception => {
            // Check if this is an Error object (has message/stack properties)
            if let Some(obj) = val.as_object() {
                // Try to get error properties
                let name: Option<String> = obj.get("name").ok();
                let message: Option<String> = obj.get("message").ok();
                let stack: Option<String> = obj.get("stack").ok();

                if message.is_some() || name.is_some() {
                    // This looks like an Error object
                    let name = name.unwrap_or_else(|| "Error".to_string());
                    let message = message.unwrap_or_default();
                    if let Some(stack) = stack {
                        return format!("{}: {}\n{}", name, message, stack);
                    } else {
                        return format!("{}: {}", name, message);
                    }
                }

                // Regular object - convert to JSON
                let json = js_to_json(ctx, val.clone());
                serde_json::to_string(&json).unwrap_or_else(|_| "[object]".to_string())
            } else {
                "[object]".to_string()
            }
        }
        Type::Array => {
            let json = js_to_json(ctx, val.clone());
            serde_json::to_string(&json).unwrap_or_else(|_| "[array]".to_string())
        }
        Type::Function | Type::Constructor => "[function]".to_string(),
        Type::Symbol => "[symbol]".to_string(),
        Type::BigInt => val
            .as_big_int()
            .and_then(|b| b.clone().to_i64().ok())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "[bigint]".to_string()),
        _ => format!("[{}]", val.type_name()),
    }
}

/// Format a JavaScript error with full details including stack trace
fn format_js_error(
    ctx: &rquickjs::Ctx<'_>,
    err: rquickjs::Error,
    source_name: &str,
) -> anyhow::Error {
    // Check if this is an exception that we can catch for more details
    if err.is_exception() {
        // Try to catch the exception to get the full error object
        let exc = ctx.catch();
        if !exc.is_undefined() && !exc.is_null() {
            // Try to get error message and stack from the exception object
            if let Some(exc_obj) = exc.as_object() {
                let message: String = exc_obj
                    .get::<_, String>("message")
                    .unwrap_or_else(|_| "Unknown error".to_string());
                let stack: String = exc_obj.get::<_, String>("stack").unwrap_or_default();
                let name: String = exc_obj
                    .get::<_, String>("name")
                    .unwrap_or_else(|_| "Error".to_string());

                if !stack.is_empty() {
                    return anyhow::anyhow!(
                        "JS error in {}: {}: {}\nStack trace:\n{}",
                        source_name,
                        name,
                        message,
                        stack
                    );
                } else {
                    return anyhow::anyhow!("JS error in {}: {}: {}", source_name, name, message);
                }
            } else {
                // Exception is not an object, try to convert to string
                let exc_str: String = exc
                    .as_string()
                    .and_then(|s: &rquickjs::String| s.to_string().ok())
                    .unwrap_or_else(|| format!("{:?}", exc));
                return anyhow::anyhow!("JS error in {}: {}", source_name, exc_str);
            }
        }
    }

    // Fall back to the basic error message
    anyhow::anyhow!("JS error in {}: {}", source_name, err)
}

/// Log a JavaScript error with full details
/// If panic_on_js_errors is enabled, this will panic to surface JS errors immediately
fn log_js_error(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error, context: &str) {
    let error = format_js_error(ctx, err, context);
    tracing::error!("{}", error);

    // When enabled, panic on JS errors to make them visible and fail fast
    if should_panic_on_js_errors() {
        panic!("JavaScript error in {}: {}", context, error);
    }
}

/// Global flag to panic on JS errors (enabled during testing)
static PANIC_ON_JS_ERRORS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Enable panicking on JS errors (call this from test setup)
pub fn set_panic_on_js_errors(enabled: bool) {
    PANIC_ON_JS_ERRORS.store(enabled, std::sync::atomic::Ordering::SeqCst);
}

/// Check if panic on JS errors is enabled
fn should_panic_on_js_errors() -> bool {
    PANIC_ON_JS_ERRORS.load(std::sync::atomic::Ordering::SeqCst)
}

/// Run all pending jobs and check for unhandled exceptions
/// If panic_on_js_errors is enabled, this will panic on unhandled exceptions
fn run_pending_jobs_checked(ctx: &rquickjs::Ctx<'_>, context: &str) -> usize {
    let mut count = 0;
    loop {
        // Check for unhandled exception before running more jobs
        let exc: rquickjs::Value = ctx.catch();
        // Only treat it as an exception if it's actually an Error object
        if exc.is_exception() {
            let error_msg = if let Some(err) = exc.as_exception() {
                format!(
                    "{}: {}",
                    err.message().unwrap_or_default(),
                    err.stack().unwrap_or_default()
                )
            } else {
                format!("{:?}", exc)
            };
            tracing::error!("Unhandled JS exception during {}: {}", context, error_msg);
            if should_panic_on_js_errors() {
                panic!("Unhandled JS exception during {}: {}", context, error_msg);
            }
        }

        if !ctx.execute_pending_job() {
            break;
        }
        count += 1;
    }

    // Final check for exceptions after all jobs completed
    let exc: rquickjs::Value = ctx.catch();
    if exc.is_exception() {
        let error_msg = if let Some(err) = exc.as_exception() {
            format!(
                "{}: {}",
                err.message().unwrap_or_default(),
                err.stack().unwrap_or_default()
            )
        } else {
            format!("{:?}", exc)
        };
        tracing::error!(
            "Unhandled JS exception after running jobs in {}: {}",
            context,
            error_msg
        );
        if should_panic_on_js_errors() {
            panic!(
                "Unhandled JS exception after running jobs in {}: {}",
                context, error_msg
            );
        }
    }

    count
}

/// Parse a TextPropertyEntry from a JS Object
fn parse_text_property_entry(
    ctx: &rquickjs::Ctx<'_>,
    obj: &Object<'_>,
) -> Option<TextPropertyEntry> {
    let text: String = obj.get("text").ok()?;
    let properties: HashMap<String, serde_json::Value> = obj
        .get::<_, Object>("properties")
        .ok()
        .map(|props_obj| {
            let mut map = HashMap::new();
            for key in props_obj.keys::<String>().flatten() {
                if let Ok(v) = props_obj.get::<_, Value>(&key) {
                    map.insert(key, js_to_json(ctx, v));
                }
            }
            map
        })
        .unwrap_or_default();
    Some(TextPropertyEntry { text, properties })
}

/// Pending response senders type alias
pub type PendingResponses =
    Arc<std::sync::Mutex<HashMap<u64, tokio::sync::oneshot::Sender<PluginResponse>>>>;

/// Information about a loaded plugin
#[derive(Debug, Clone)]
pub struct TsPluginInfo {
    pub name: String,
    pub path: PathBuf,
    pub enabled: bool,
}

/// JavaScript-exposed Editor API using rquickjs class system
/// This allows proper lifetime handling for methods returning JS values
#[derive(rquickjs::class::Trace, rquickjs::JsLifetime)]
#[rquickjs::class]
pub struct JsEditorApi {
    #[qjs(skip_trace)]
    state_snapshot: Arc<RwLock<EditorStateSnapshot>>,
    #[qjs(skip_trace)]
    command_sender: mpsc::Sender<PluginCommand>,
    #[qjs(skip_trace)]
    registered_actions: Rc<RefCell<HashMap<String, String>>>,
    #[qjs(skip_trace)]
    event_handlers: Rc<RefCell<HashMap<String, Vec<String>>>>,
    #[qjs(skip_trace)]
    dir_context: DirectoryContext,
    #[qjs(skip_trace)]
    next_request_id: Rc<RefCell<u64>>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl JsEditorApi {
    // === Buffer Queries ===

    /// Get the active buffer ID (0 if none)
    pub fn get_active_buffer_id(&self) -> u32 {
        self.state_snapshot.read().map(|s| s.active_buffer_id.0 as u32).unwrap_or(0)
    }

    /// Get the active split ID
    pub fn get_active_split_id(&self) -> u32 {
        self.state_snapshot.read().map(|s| s.active_split_id as u32).unwrap_or(0)
    }

    /// List all open buffers - returns array of BufferInfo objects
    pub fn list_buffers<'js>(&self, ctx: rquickjs::Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let buffers: Vec<BufferInfo> = if let Ok(s) = self.state_snapshot.read() {
            s.buffers.values().cloned().collect()
        } else {
            Vec::new()
        };
        rquickjs_serde::to_value(ctx, &buffers)
            .map_err(|e| rquickjs::Error::new_from_js_message("serialize", "", &e.to_string()))
    }

    // === Logging ===

    pub fn debug(&self, msg: String) {
        tracing::info!("Plugin.debug: {}", msg);
    }

    pub fn info(&self, msg: String) {
        tracing::info!("Plugin: {}", msg);
    }

    pub fn warn(&self, msg: String) {
        tracing::warn!("Plugin: {}", msg);
    }

    pub fn error(&self, msg: String) {
        tracing::error!("Plugin: {}", msg);
    }

    // === Status ===

    pub fn set_status(&self, msg: String) {
        let _ = self.command_sender.send(PluginCommand::SetStatus { message: msg });
    }

    // === Clipboard ===

    pub fn copy_to_clipboard(&self, text: String) {
        let _ = self.command_sender.send(PluginCommand::SetClipboard { text });
    }

    pub fn set_clipboard(&self, text: String) {
        let _ = self.command_sender.send(PluginCommand::SetClipboard { text: text });
    }

    // === Command Registration ===

    /// Register a command - reads plugin name from __currentPluginName__ global
    pub fn register_command<'js>(
        &self,
        ctx: rquickjs::Ctx<'js>,
        name: String,
        description: String,
        handler_name: String,
        context: Option<String>,
    ) -> rquickjs::Result<bool> {
        // Get plugin name from global context
        let globals = ctx.globals();
        let plugin_name: String = globals
            .get("__currentPluginName__")
            .unwrap_or_else(|_| "unknown".to_string());

        tracing::debug!(
            "registerCommand: plugin='{}', name='{}', handler='{}'",
            plugin_name,
            name,
            handler_name
        );

        // Store action handler mapping
        self.registered_actions
            .borrow_mut()
            .insert(handler_name.clone(), handler_name.clone());

        // Register with editor
        let command = Command {
            name: name.clone(),
            description,
            action: Action::PluginAction(handler_name),
            contexts: vec![],
            custom_contexts: context.into_iter().collect(),
            source: CommandSource::Plugin(plugin_name),
        };

        Ok(self
            .command_sender
            .send(PluginCommand::RegisterCommand { command })
            .is_ok())
    }

    /// Unregister a command by name
    pub fn unregister_command(&self, name: String) -> bool {
        self.command_sender
            .send(PluginCommand::UnregisterCommand { name })
            .is_ok()
    }

    /// Set a context (for keybinding conditions)
    pub fn set_context(&self, name: String, active: bool) -> bool {
        self.command_sender
            .send(PluginCommand::SetContext { name, active })
            .is_ok()
    }

    /// Execute a built-in action
    pub fn execute_action(&self, action_name: String) -> bool {
        self.command_sender
            .send(PluginCommand::ExecuteAction { action_name })
            .is_ok()
    }

    // === Translation ===

    /// Translate a string - reads plugin name from __currentPluginName__ global
    /// Args is optional - can be omitted, undefined, null, or an object
    pub fn t<'js>(
        &self,
        ctx: rquickjs::Ctx<'js>,
        key: String,
        args: rquickjs::function::Rest<Value<'js>>,
    ) -> String {
        // Get plugin name from global context
        let globals = ctx.globals();
        let plugin_name: String = globals
            .get("__currentPluginName__")
            .unwrap_or_else(|_| "unknown".to_string());

        // Convert args to HashMap - args.0 is a Vec of the rest arguments
        let args_map: HashMap<String, String> = if let Some(first_arg) = args.0.first() {
            if let Some(obj) = first_arg.as_object() {
                let mut map = HashMap::new();
                for key_result in obj.keys::<String>() {
                    if let Ok(k) = key_result {
                        if let Ok(v) = obj.get::<_, String>(&k) {
                            map.insert(k, v);
                        }
                    }
                }
                map
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        crate::i18n::translate_plugin_string(&plugin_name, &key, &args_map)
    }

    // === Buffer Queries (additional) ===

    /// Get cursor position in active buffer
    pub fn get_cursor_position(&self) -> u32 {
        self.state_snapshot
            .read()
            .ok()
            .and_then(|s| s.primary_cursor.as_ref().map(|c| c.position as u32))
            .unwrap_or(0)
    }

    /// Get file path for a buffer
    pub fn get_buffer_path(&self, buffer_id: u32) -> String {
        if let Ok(s) = self.state_snapshot.read() {
            if let Some(b) = s.buffers.get(&BufferId(buffer_id as usize)) {
                if let Some(p) = &b.path {
                    return p.to_string_lossy().to_string();
                }
            }
        }
        String::new()
    }

    /// Get buffer length in bytes
    pub fn get_buffer_length(&self, buffer_id: u32) -> u32 {
        if let Ok(s) = self.state_snapshot.read() {
            if let Some(b) = s.buffers.get(&BufferId(buffer_id as usize)) {
                return b.length as u32;
            }
        }
        0
    }

    /// Check if buffer has unsaved changes
    pub fn is_buffer_modified(&self, buffer_id: u32) -> bool {
        if let Ok(s) = self.state_snapshot.read() {
            if let Some(b) = s.buffers.get(&BufferId(buffer_id as usize)) {
                return b.modified;
            }
        }
        false
    }

    // === Text Editing ===

    /// Insert text at a position in a buffer
    pub fn insert_text(&self, buffer_id: u32, position: u32, text: String) -> bool {
        self.command_sender
            .send(PluginCommand::InsertText {
                buffer_id: BufferId(buffer_id as usize),
                position: position as usize,
                text,
            })
            .is_ok()
    }

    /// Delete a range from a buffer
    pub fn delete_range(&self, buffer_id: u32, start: u32, end: u32) -> bool {
        self.command_sender
            .send(PluginCommand::DeleteRange {
                buffer_id: BufferId(buffer_id as usize),
                range: (start as usize)..(end as usize),
            })
            .is_ok()
    }

    /// Insert text at cursor position in active buffer
    pub fn insert_at_cursor(&self, text: String) -> bool {
        self.command_sender
            .send(PluginCommand::InsertAtCursor { text })
            .is_ok()
    }

    // === File Operations ===

    /// Open a file, optionally at a specific line/column
    pub fn open_file(&self, path: String, line: Option<u32>, column: Option<u32>) -> bool {
        self.command_sender
            .send(PluginCommand::OpenFileAtLocation {
                path: PathBuf::from(path),
                line: line.map(|l| l as usize),
                column: column.map(|c| c as usize),
            })
            .is_ok()
    }

    /// Show a buffer in the current split
    pub fn show_buffer(&self, buffer_id: u32) -> bool {
        self.command_sender
            .send(PluginCommand::ShowBuffer {
                buffer_id: BufferId(buffer_id as usize),
            })
            .is_ok()
    }

    /// Close a buffer
    pub fn close_buffer(&self, buffer_id: u32) -> bool {
        self.command_sender
            .send(PluginCommand::CloseBuffer {
                buffer_id: BufferId(buffer_id as usize),
            })
            .is_ok()
    }

    // === Event Handling ===

    /// Subscribe to an editor event
    pub fn on(&self, event_name: String, handler_name: String) {
        self.event_handlers
            .borrow_mut()
            .entry(event_name)
            .or_default()
            .push(handler_name);
    }

    /// Unsubscribe from an event
    pub fn off(&self, event_name: String, handler_name: String) {
        let mut handlers = self.event_handlers.borrow_mut();
        if let Some(list) = handlers.get_mut(&event_name) {
            list.retain(|h| h != &handler_name);
        }
    }

    // === Environment ===

    /// Get an environment variable
    pub fn get_env(&self, name: String) -> Option<String> {
        std::env::var(&name).ok()
    }

    /// Get current working directory
    pub fn get_cwd(&self) -> String {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string())
    }

    // === Path Operations ===

    /// Join path components
    pub fn path_join(&self, parts: Vec<String>) -> String {
        let mut path = PathBuf::new();
        for part in parts {
            if Path::new(&part).is_absolute() {
                path = PathBuf::from(part);
            } else {
                path.push(part);
            }
        }
        path.to_string_lossy().to_string()
    }

    /// Get directory name from path
    pub fn path_dirname(&self, path: String) -> String {
        Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// Get file name from path
    pub fn path_basename(&self, path: String) -> String {
        Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// Get file extension
    pub fn path_extname(&self, path: String) -> String {
        Path::new(&path)
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default()
    }

    /// Check if path is absolute
    pub fn path_is_absolute(&self, path: String) -> bool {
        Path::new(&path).is_absolute()
    }

    // === File System ===

    /// Check if file exists
    pub fn file_exists(&self, path: String) -> bool {
        Path::new(&path).exists()
    }

    /// Read file contents
    pub fn read_file(&self, path: String) -> Option<String> {
        std::fs::read_to_string(&path).ok()
    }

    /// Write file contents
    pub fn write_file(&self, path: String, content: String) -> bool {
        std::fs::write(&path, content).is_ok()
    }

    /// Read directory contents (returns array of {name, is_file, is_dir})
    pub fn read_dir<'js>(&self, ctx: rquickjs::Ctx<'js>, path: String) -> rquickjs::Result<Value<'js>> {
        #[derive(serde::Serialize)]
        struct DirEntry {
            name: String,
            is_file: bool,
            is_dir: bool,
        }

        let entries: Vec<DirEntry> = match std::fs::read_dir(&path) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|entry| {
                    let file_type = entry.file_type().ok();
                    DirEntry {
                        name: entry.file_name().to_string_lossy().to_string(),
                        is_file: file_type.map(|ft| ft.is_file()).unwrap_or(false),
                        is_dir: file_type.map(|ft| ft.is_dir()).unwrap_or(false),
                    }
                })
                .collect(),
            Err(e) => {
                tracing::warn!("readDir failed for '{}': {}", path, e);
                Vec::new()
            }
        };

        rquickjs_serde::to_value(ctx, &entries)
            .map_err(|e| rquickjs::Error::new_from_js_message("serialize", "", &e.to_string()))
    }

    // === Config ===

    /// Get current config as JS object
    pub fn get_config<'js>(&self, ctx: rquickjs::Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let config: serde_json::Value = self.state_snapshot
            .read()
            .map(|s| s.config.clone())
            .unwrap_or_else(|_| serde_json::json!({}));

        rquickjs_serde::to_value(ctx, &config)
            .map_err(|e| rquickjs::Error::new_from_js_message("serialize", "", &e.to_string()))
    }

    /// Get user config as JS object
    pub fn get_user_config<'js>(&self, ctx: rquickjs::Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let config: serde_json::Value = self.state_snapshot
            .read()
            .map(|s| s.user_config.clone())
            .unwrap_or_else(|_| serde_json::json!({}));

        rquickjs_serde::to_value(ctx, &config)
            .map_err(|e| rquickjs::Error::new_from_js_message("serialize", "", &e.to_string()))
    }

    /// Reload configuration from file
    pub fn reload_config(&self) {
        let _ = self.command_sender.send(PluginCommand::ReloadConfig);
    }

    /// Get config directory path
    pub fn get_config_dir(&self) -> String {
        self.dir_context.config_dir.to_string_lossy().to_string()
    }

    /// Get themes directory path
    pub fn get_themes_dir(&self) -> String {
        self.dir_context.config_dir.join("themes").to_string_lossy().to_string()
    }

    /// Apply a theme by name
    pub fn apply_theme(&self, theme_name: String) -> bool {
        self.command_sender
            .send(PluginCommand::ApplyTheme { theme_name })
            .is_ok()
    }

    /// Get theme schema as JS object
    pub fn get_theme_schema<'js>(&self, ctx: rquickjs::Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let schema = crate::view::theme::get_theme_schema();
        rquickjs_serde::to_value(ctx, &schema)
            .map_err(|e| rquickjs::Error::new_from_js_message("serialize", "", &e.to_string()))
    }

    /// Get list of builtin themes as JS object
    pub fn get_builtin_themes<'js>(&self, ctx: rquickjs::Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let themes = crate::view::theme::get_builtin_themes();
        rquickjs_serde::to_value(ctx, &themes)
            .map_err(|e| rquickjs::Error::new_from_js_message("serialize", "", &e.to_string()))
    }

    /// Delete a custom theme file (sync)
    #[qjs(rename = "_deleteThemeSync")]
    pub fn delete_theme_sync(&self, name: String) -> bool {
        // Security: only allow deleting from the themes directory
        let themes_dir = self.dir_context.config_dir.join("themes");
        let theme_path = themes_dir.join(format!("{}.json", name));

        // Verify the file is actually in the themes directory (prevent path traversal)
        if let Ok(canonical) = theme_path.canonicalize() {
            if let Ok(themes_canonical) = themes_dir.canonicalize() {
                if canonical.starts_with(&themes_canonical) {
                    return std::fs::remove_file(&canonical).is_ok();
                }
            }
        }
        false
    }

    // === Overlays ===

    /// Add an overlay with styling
    #[allow(clippy::too_many_arguments)]
    pub fn add_overlay(
        &self,
        buffer_id: u32,
        namespace: String,
        start: u32,
        end: u32,
        r: i32,
        g: i32,
        b: i32,
        underline: bool,
        bold: bool,
        italic: bool,
        bg_r: i32,
        bg_g: i32,
        bg_b: i32,
        extend_to_line_end: bool,
    ) -> bool {
        // -1 means use default color (white)
        let color = if r >= 0 && g >= 0 && b >= 0 {
            (r as u8, g as u8, b as u8)
        } else {
            (255, 255, 255)
        };

        // -1 for bg means no background
        let bg_color = if bg_r >= 0 && bg_g >= 0 && bg_b >= 0 {
            Some((bg_r as u8, bg_g as u8, bg_b as u8))
        } else {
            None
        };

        self.command_sender
            .send(PluginCommand::AddOverlay {
                buffer_id: BufferId(buffer_id as usize),
                namespace: Some(OverlayNamespace::from_string(namespace)),
                range: (start as usize)..(end as usize),
                color,
                bg_color,
                underline,
                bold,
                italic,
                extend_to_line_end,
            })
            .is_ok()
    }

    /// Clear all overlays in a namespace
    pub fn clear_namespace(&self, buffer_id: u32, namespace: String) -> bool {
        self.command_sender
            .send(PluginCommand::ClearNamespace {
                buffer_id: BufferId(buffer_id as usize),
                namespace: OverlayNamespace::from_string(namespace),
            })
            .is_ok()
    }

    /// Clear all overlays from a buffer
    pub fn clear_all_overlays(&self, buffer_id: u32) -> bool {
        self.command_sender
            .send(PluginCommand::ClearAllOverlays {
                buffer_id: BufferId(buffer_id as usize),
            })
            .is_ok()
    }

    // === Prompts ===

    /// Start an interactive prompt
    pub fn start_prompt(&self, label: String, prompt_type: String) -> bool {
        self.command_sender
            .send(PluginCommand::StartPrompt { label, prompt_type })
            .is_ok()
    }

    /// Start a prompt with initial value
    pub fn start_prompt_with_initial(
        &self,
        label: String,
        prompt_type: String,
        initial_value: String,
    ) -> bool {
        self.command_sender
            .send(PluginCommand::StartPromptWithInitial {
                label,
                prompt_type,
                initial_value,
            })
            .is_ok()
    }

    /// Set suggestions for the current prompt (takes array of suggestion objects)
    pub fn set_prompt_suggestions<'js>(
        &self,
        ctx: rquickjs::Ctx<'js>,
        suggestions_arr: Vec<rquickjs::Object<'js>>,
    ) -> rquickjs::Result<bool> {
        let suggestions: Vec<crate::input::commands::Suggestion> = suggestions_arr
            .into_iter()
            .map(|obj| crate::input::commands::Suggestion {
                text: obj.get("text").unwrap_or_default(),
                description: obj.get("description").ok(),
                value: obj.get("value").ok(),
                disabled: obj.get("disabled").unwrap_or(false),
                keybinding: obj.get("keybinding").ok(),
                source: None,
            })
            .collect();
        Ok(self
            .command_sender
            .send(PluginCommand::SetPromptSuggestions { suggestions })
            .is_ok())
    }

    // === Modes ===

    /// Define a buffer mode (takes bindings as array of [key, command] pairs)
    pub fn define_mode(
        &self,
        name: String,
        parent: Option<String>,
        bindings_arr: Vec<Vec<String>>,
    ) -> bool {
        let bindings: Vec<(String, String)> = bindings_arr
            .into_iter()
            .filter_map(|arr| {
                if arr.len() >= 2 {
                    Some((arr[0].clone(), arr[1].clone()))
                } else {
                    None
                }
            })
            .collect();

        self.command_sender
            .send(PluginCommand::DefineMode {
                name,
                parent,
                bindings,
                read_only: false,
            })
            .is_ok()
    }

    /// Set the global editor mode
    pub fn set_editor_mode(&self, mode: Option<String>) -> bool {
        self.command_sender
            .send(PluginCommand::SetEditorMode { mode })
            .is_ok()
    }

    /// Get the current editor mode
    pub fn get_editor_mode(&self) -> Option<String> {
        self.state_snapshot
            .read()
            .ok()
            .and_then(|s| s.editor_mode.clone())
    }

    // === Splits ===

    /// Close a split
    pub fn close_split(&self, split_id: u32) -> bool {
        self.command_sender
            .send(PluginCommand::CloseSplit {
                split_id: SplitId(split_id as usize),
            })
            .is_ok()
    }

    /// Set the buffer displayed in a split
    pub fn set_split_buffer(&self, split_id: u32, buffer_id: u32) -> bool {
        self.command_sender
            .send(PluginCommand::SetSplitBuffer {
                split_id: SplitId(split_id as usize),
                buffer_id: BufferId(buffer_id as usize),
            })
            .is_ok()
    }

    /// Focus a specific split
    pub fn focus_split(&self, split_id: u32) -> bool {
        self.command_sender
            .send(PluginCommand::FocusSplit {
                split_id: SplitId(split_id as usize),
            })
            .is_ok()
    }

    /// Set cursor position in a buffer
    pub fn set_buffer_cursor(&self, buffer_id: u32, position: u32) -> bool {
        self.command_sender
            .send(PluginCommand::SetBufferCursor {
                buffer_id: BufferId(buffer_id as usize),
                position: position as usize,
            })
            .is_ok()
    }

    // === Line Indicators ===

    /// Set a line indicator in the gutter
    #[allow(clippy::too_many_arguments)]
    pub fn set_line_indicator(
        &self,
        buffer_id: u32,
        line: u32,
        namespace: String,
        symbol: String,
        r: u8,
        g: u8,
        b: u8,
        priority: i32,
    ) -> bool {
        self.command_sender
            .send(PluginCommand::SetLineIndicator {
                buffer_id: BufferId(buffer_id as usize),
                line: line as usize,
                namespace,
                symbol,
                color: (r, g, b),
                priority,
            })
            .is_ok()
    }

    /// Clear line indicators in a namespace
    pub fn clear_line_indicators(&self, buffer_id: u32, namespace: String) -> bool {
        self.command_sender
            .send(PluginCommand::ClearLineIndicators {
                buffer_id: BufferId(buffer_id as usize),
                namespace,
            })
            .is_ok()
    }

    // === Virtual Buffers ===

    /// Create a virtual buffer in current split (async, returns request_id)
    #[qjs(rename = "_createVirtualBufferStart")]
    pub fn create_virtual_buffer_start<'js>(
        &self,
        ctx: rquickjs::Ctx<'js>,
        opts: rquickjs::Object<'js>,
    ) -> rquickjs::Result<u64> {
        let id = {
            let mut id_ref = self.next_request_id.borrow_mut();
            let id = *id_ref;
            *id_ref += 1;
            id
        };

        let name: String = opts.get("name")?;
        let mode: String = opts.get("mode").unwrap_or_default();
        let read_only: bool = opts.get("readOnly").unwrap_or(false);
        let show_line_numbers: bool = opts.get("showLineNumbers").unwrap_or(false);
        let show_cursors: bool = opts.get("showCursors").unwrap_or(true);
        let editing_disabled: bool = opts.get("editingDisabled").unwrap_or(false);
        let hidden_from_tabs: bool = opts.get("hiddenFromTabs").unwrap_or(false);

        // entries is array of {text: string, properties?: object}
        let entries_arr: Vec<rquickjs::Object> = opts.get("entries").unwrap_or_default();
        let entries: Vec<TextPropertyEntry> = entries_arr
            .iter()
            .filter_map(|obj| parse_text_property_entry(&ctx, obj))
            .collect();

        tracing::debug!(
            "_createVirtualBufferStart: sending CreateVirtualBufferWithContent command, request_id={}",
            id
        );
        let _ = self.command_sender.send(PluginCommand::CreateVirtualBufferWithContent {
            name,
            mode,
            read_only,
            entries,
            show_line_numbers,
            show_cursors,
            editing_disabled,
            hidden_from_tabs,
            request_id: Some(id),
        });
        Ok(id)
    }

    /// Create a virtual buffer in a new split (async, returns request_id)
    #[qjs(rename = "_createVirtualBufferInSplitStart")]
    pub fn create_virtual_buffer_in_split_start<'js>(
        &self,
        ctx: rquickjs::Ctx<'js>,
        opts: rquickjs::Object<'js>,
    ) -> rquickjs::Result<u64> {
        let id = {
            let mut id_ref = self.next_request_id.borrow_mut();
            let id = *id_ref;
            *id_ref += 1;
            id
        };

        let name: String = opts.get("name")?;
        let mode: String = opts.get("mode").unwrap_or_default();
        let read_only: bool = opts.get("readOnly").unwrap_or(false);
        let ratio: f32 = opts.get("ratio").unwrap_or(0.5);
        let direction: Option<String> = opts.get("direction").ok();
        let panel_id: Option<String> = opts.get("panelId").ok();
        let show_line_numbers: bool = opts.get("showLineNumbers").unwrap_or(true);
        let show_cursors: bool = opts.get("showCursors").unwrap_or(true);
        let editing_disabled: bool = opts.get("editingDisabled").unwrap_or(false);
        let line_wrap: Option<bool> = opts.get("lineWrap").ok();

        // entries is array of {text: string, properties?: object}
        let entries_arr: Vec<rquickjs::Object> = opts.get("entries").unwrap_or_default();
        let entries: Vec<TextPropertyEntry> = entries_arr
            .iter()
            .filter_map(|obj| parse_text_property_entry(&ctx, obj))
            .collect();

        let _ = self.command_sender.send(PluginCommand::CreateVirtualBufferInSplit {
            name,
            mode,
            read_only,
            entries,
            ratio,
            direction,
            panel_id,
            show_line_numbers,
            show_cursors,
            editing_disabled,
            line_wrap,
            request_id: Some(id),
        });
        Ok(id)
    }

    /// Set virtual buffer content (takes array of entry objects)
    pub fn set_virtual_buffer_content<'js>(
        &self,
        ctx: rquickjs::Ctx<'js>,
        buffer_id: u32,
        entries_arr: Vec<rquickjs::Object<'js>>,
    ) -> rquickjs::Result<bool> {
        let entries: Vec<TextPropertyEntry> = entries_arr
            .iter()
            .filter_map(|obj| parse_text_property_entry(&ctx, obj))
            .collect();
        Ok(self
            .command_sender
            .send(PluginCommand::SetVirtualBufferContent {
                buffer_id: BufferId(buffer_id as usize),
                entries,
            })
            .is_ok())
    }

    /// Get text properties at cursor position (returns JS array)
    pub fn get_text_properties_at_cursor<'js>(
        &self,
        ctx: rquickjs::Ctx<'js>,
        buffer_id: u32,
    ) -> rquickjs::Result<Value<'js>> {
        let json_str = get_text_properties_at_cursor_json(&self.state_snapshot, buffer_id);
        // Parse JSON and convert to JS value
        let json_value: serde_json::Value =
            serde_json::from_str(&json_str).unwrap_or(serde_json::json!([]));
        rquickjs_serde::to_value(ctx, &json_value)
            .map_err(|e| rquickjs::Error::new_from_js_message("serialize", "", &e.to_string()))
    }

    // === Async Operations ===

    /// Spawn a process (async, returns request_id)
    #[qjs(rename = "_spawnProcessStart")]
    pub fn spawn_process_start(
        &self,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> u64 {
        let id = {
            let mut id_ref = self.next_request_id.borrow_mut();
            let id = *id_ref;
            *id_ref += 1;
            id
        };
        let _ = self.command_sender.send(PluginCommand::SpawnProcess {
            callback_id: id,
            command,
            args,
            cwd,
        });
        id
    }

    /// Get buffer text range (async, returns request_id)
    #[qjs(rename = "_getBufferTextStart")]
    pub fn get_buffer_text_start(&self, buffer_id: u32, start: u32, end: u32) -> u64 {
        let id = {
            let mut id_ref = self.next_request_id.borrow_mut();
            let id = *id_ref;
            *id_ref += 1;
            id
        };
        let _ = self.command_sender.send(PluginCommand::GetBufferText {
            buffer_id: BufferId(buffer_id as usize),
            start: start as usize,
            end: end as usize,
            request_id: id,
        });
        id
    }

    /// Delay/sleep (async, returns request_id)
    #[qjs(rename = "_delayStart")]
    pub fn delay_start(&self, duration_ms: u64) -> u64 {
        let id = {
            let mut id_ref = self.next_request_id.borrow_mut();
            let id = *id_ref;
            *id_ref += 1;
            id
        };
        let _ = self.command_sender.send(PluginCommand::Delay {
            callback_id: id,
            duration_ms,
        });
        id
    }

    /// Send LSP request (async, returns request_id)
    #[qjs(rename = "_sendLspRequestStart")]
    pub fn send_lsp_request_start<'js>(
        &self,
        ctx: rquickjs::Ctx<'js>,
        language: String,
        method: String,
        params: Option<rquickjs::Object<'js>>,
    ) -> rquickjs::Result<u64> {
        let id = {
            let mut id_ref = self.next_request_id.borrow_mut();
            let id = *id_ref;
            *id_ref += 1;
            id
        };
        // Convert params object to serde_json::Value
        let params_json: Option<serde_json::Value> = params.map(|obj| {
            let val = obj.into_value();
            js_to_json(&ctx, val)
        });
        let _ = self.command_sender.send(PluginCommand::SendLspRequest {
            request_id: id,
            language,
            method,
            params: params_json,
        });
        Ok(id)
    }

    /// Spawn a background process (async, returns request_id which is also process_id)
    #[qjs(rename = "_spawnBackgroundProcessStart")]
    pub fn spawn_background_process_start(
        &self,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> u64 {
        let callback_id = {
            let mut id_ref = self.next_request_id.borrow_mut();
            let id = *id_ref;
            *id_ref += 1;
            id
        };
        // Use callback_id as process_id for simplicity
        let process_id = callback_id;
        let _ = self.command_sender.send(PluginCommand::SpawnBackgroundProcess {
            process_id,
            command,
            args,
            cwd,
            callback_id,
        });
        callback_id
    }

    /// Kill a background process
    pub fn kill_background_process(&self, process_id: u64) -> bool {
        self.command_sender
            .send(PluginCommand::KillBackgroundProcess { process_id })
            .is_ok()
    }

    // === Misc ===

    /// Force refresh of line display
    pub fn refresh_lines(&self, buffer_id: u32) -> bool {
        self.command_sender
            .send(PluginCommand::RefreshLines {
                buffer_id: BufferId(buffer_id as usize),
            })
            .is_ok()
    }

    /// Get the current locale
    pub fn get_current_locale(&self) -> String {
        crate::i18n::current_locale()
    }
}

/// QuickJS-based JavaScript runtime for plugins
pub struct QuickJsBackend {
    runtime: Runtime,
    context: Context,
    /// Event handlers: event_name -> list of handler function names
    event_handlers: Rc<RefCell<HashMap<String, Vec<String>>>>,
    /// Registered actions: action_name -> handler function name
    registered_actions: Rc<RefCell<HashMap<String, String>>>,
    /// Editor state snapshot (read-only access)
    state_snapshot: Arc<RwLock<EditorStateSnapshot>>,
    /// Command sender for write operations
    command_sender: mpsc::Sender<PluginCommand>,
    /// Pending response senders for async operations
    pending_responses: PendingResponses,
    /// Next request ID for async operations
    next_request_id: Rc<RefCell<u64>>,
    /// Directory context for system paths
    dir_context: DirectoryContext,
}

impl QuickJsBackend {
    /// Create a new QuickJS backend (standalone, for testing)
    pub fn new() -> Result<Self> {
        let (tx, _rx) = mpsc::channel();
        let state_snapshot = Arc::new(RwLock::new(EditorStateSnapshot::new()));
        let dir_context = DirectoryContext::for_testing(Path::new("/tmp"));
        Self::with_state(state_snapshot, tx, dir_context)
    }

    /// Create a new QuickJS backend with editor state
    pub fn with_state(
        state_snapshot: Arc<RwLock<EditorStateSnapshot>>,
        command_sender: mpsc::Sender<PluginCommand>,
        dir_context: DirectoryContext,
    ) -> Result<Self> {
        let pending_responses: PendingResponses = Arc::new(std::sync::Mutex::new(HashMap::new()));
        Self::with_state_and_responses(
            state_snapshot,
            command_sender,
            pending_responses,
            dir_context,
        )
    }

    /// Create a new QuickJS backend with editor state and shared pending responses
    pub fn with_state_and_responses(
        state_snapshot: Arc<RwLock<EditorStateSnapshot>>,
        command_sender: mpsc::Sender<PluginCommand>,
        pending_responses: PendingResponses,
        dir_context: DirectoryContext,
    ) -> Result<Self> {
        tracing::debug!("QuickJsBackend::new: creating QuickJS runtime");

        let runtime =
            Runtime::new().map_err(|e| anyhow!("Failed to create QuickJS runtime: {}", e))?;

        // Set up promise rejection tracker to catch unhandled rejections
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            |_ctx, _promise, reason, is_handled| {
                if !is_handled {
                    // Format the rejection reason
                    let error_msg = if let Some(exc) = reason.as_exception() {
                        format!(
                            "{}: {}",
                            exc.message().unwrap_or_default(),
                            exc.stack().unwrap_or_default()
                        )
                    } else {
                        format!("{:?}", reason)
                    };

                    tracing::error!("Unhandled Promise rejection: {}", error_msg);

                    if should_panic_on_js_errors() {
                        panic!("Unhandled Promise rejection: {}", error_msg);
                    }
                }
            },
        )));

        let context = Context::full(&runtime)
            .map_err(|e| anyhow!("Failed to create QuickJS context: {}", e))?;

        let event_handlers = Rc::new(RefCell::new(HashMap::new()));
        let registered_actions = Rc::new(RefCell::new(HashMap::new()));
        let next_request_id = Rc::new(RefCell::new(1u64));

        let mut backend = Self {
            runtime,
            context,
            event_handlers,
            registered_actions,
            state_snapshot,
            command_sender,
            pending_responses,
            next_request_id,
            dir_context,
        };

        backend.setup_global_api()?;

        tracing::debug!("QuickJsBackend::new: runtime created successfully");
        Ok(backend)
    }

    /// Set up the global editor API in the JavaScript context
    fn setup_global_api(&mut self) -> Result<()> {
        let state_snapshot = Arc::clone(&self.state_snapshot);
        let command_sender = self.command_sender.clone();
        let event_handlers = Rc::clone(&self.event_handlers);
        let registered_actions = Rc::clone(&self.registered_actions);
        let next_request_id = Rc::clone(&self.next_request_id);
        let dir_context = self.dir_context.clone();

        self.context.with(|ctx| {
            let globals = ctx.globals();

            // Create the editor object using JsEditorApi class
            // This provides proper lifetime handling for methods returning JS values
            let js_api = JsEditorApi {
                state_snapshot: Arc::clone(&state_snapshot),
                command_sender: command_sender.clone(),
                registered_actions: Rc::clone(&registered_actions),
                event_handlers: Rc::clone(&event_handlers),
                dir_context: dir_context.clone(),
                next_request_id: Rc::clone(&next_request_id),
            };
            let editor = rquickjs::Class::<JsEditorApi>::instance(ctx.clone(), js_api)?;

            // All methods are now in JsEditorApi - export editor as global
            globals.set("editor", editor)?;

            // Provide console.log for debugging
            // Use Rest<T> to handle variadic arguments like console.log('a', 'b', obj)
            let console = Object::new(ctx.clone())?;
            console.set("log", Function::new(ctx.clone(), |ctx: rquickjs::Ctx, args: rquickjs::function::Rest<rquickjs::Value>| {
                let parts: Vec<String> = args.0.iter().map(|v| js_value_to_string(&ctx, v)).collect();
                tracing::info!("console.log: {}", parts.join(" "));
            })?)?;
            console.set("warn", Function::new(ctx.clone(), |ctx: rquickjs::Ctx, args: rquickjs::function::Rest<rquickjs::Value>| {
                let parts: Vec<String> = args.0.iter().map(|v| js_value_to_string(&ctx, v)).collect();
                tracing::warn!("console.warn: {}", parts.join(" "));
            })?)?;
            console.set("error", Function::new(ctx.clone(), |ctx: rquickjs::Ctx, args: rquickjs::function::Rest<rquickjs::Value>| {
                let parts: Vec<String> = args.0.iter().map(|v| js_value_to_string(&ctx, v)).collect();
                tracing::error!("console.error: {}", parts.join(" "));
            })?)?;
            globals.set("console", console)?;

            // Bootstrap: Promise infrastructure (getEditor is defined per-plugin in execute_js)
            ctx.eval::<(), _>(r#"
                // Pending promise callbacks: callbackId -> { resolve, reject }
                globalThis._pendingCallbacks = new Map();

                // Resolve a pending callback (called from Rust)
                globalThis._resolveCallback = function(callbackId, result) {
                    console.log('[JS] _resolveCallback called with callbackId=' + callbackId + ', pendingCallbacks.size=' + globalThis._pendingCallbacks.size);
                    const cb = globalThis._pendingCallbacks.get(callbackId);
                    if (cb) {
                        console.log('[JS] _resolveCallback: found callback, calling resolve()');
                        globalThis._pendingCallbacks.delete(callbackId);
                        cb.resolve(result);
                        console.log('[JS] _resolveCallback: resolve() called');
                    } else {
                        console.log('[JS] _resolveCallback: NO callback found for id=' + callbackId);
                    }
                };

                // Reject a pending callback (called from Rust)
                globalThis._rejectCallback = function(callbackId, error) {
                    const cb = globalThis._pendingCallbacks.get(callbackId);
                    if (cb) {
                        globalThis._pendingCallbacks.delete(callbackId);
                        cb.reject(new Error(error));
                    }
                };

                // Generic async wrapper decorator
                // Wraps a function that returns a callbackId into a promise-returning function
                // Usage: editor.foo = _wrapAsync(editor._fooStart, "foo");
                globalThis._wrapAsync = function(startFn, fnName) {
                    if (typeof startFn !== 'function') {
                        // Return a function that always throws - catches missing implementations
                        return function(...args) {
                            const error = new Error(`editor.${fnName || 'unknown'} is not implemented (missing _${fnName}Start)`);
                            editor.debug(`[ASYNC ERROR] ${error.message}`);
                            throw error;
                        };
                    }
                    return function(...args) {
                        const callbackId = startFn.apply(this, args);
                        return new Promise((resolve, reject) => {
                            // NOTE: setTimeout not available in QuickJS - timeout disabled for now
                            // TODO: Implement setTimeout polyfill using editor.delay() or similar
                            globalThis._pendingCallbacks.set(callbackId, { resolve, reject });
                        });
                    };
                };

                // Async wrapper that returns a thenable object (for APIs like spawnProcess)
                // The returned object has .result promise and is itself thenable
                globalThis._wrapAsyncThenable = function(startFn, fnName) {
                    if (typeof startFn !== 'function') {
                        // Return a function that always throws - catches missing implementations
                        return function(...args) {
                            const error = new Error(`editor.${fnName || 'unknown'} is not implemented (missing _${fnName}Start)`);
                            editor.debug(`[ASYNC ERROR] ${error.message}`);
                            throw error;
                        };
                    }
                    return function(...args) {
                        const callbackId = startFn.apply(this, args);
                        const resultPromise = new Promise((resolve, reject) => {
                            // NOTE: setTimeout not available in QuickJS - timeout disabled for now
                            globalThis._pendingCallbacks.set(callbackId, { resolve, reject });
                        });
                        return {
                            get result() { return resultPromise; },
                            then(onFulfilled, onRejected) {
                                return resultPromise.then(onFulfilled, onRejected);
                            },
                            catch(onRejected) {
                                return resultPromise.catch(onRejected);
                            }
                        };
                    };
                };

                // Apply wrappers to async functions on editor
                editor.spawnProcess = _wrapAsyncThenable(editor._spawnProcessStart, "spawnProcess");
                editor.delay = _wrapAsync(editor._delayStart, "delay");
                editor.createVirtualBuffer = _wrapAsync(editor._createVirtualBufferStart, "createVirtualBuffer");
                editor.createVirtualBufferInSplit = _wrapAsyncThenable(editor._createVirtualBufferInSplitStart, "createVirtualBufferInSplit");
                editor.sendLspRequest = _wrapAsync(editor._sendLspRequestStart, "sendLspRequest");
                editor.spawnBackgroundProcess = _wrapAsyncThenable(editor._spawnBackgroundProcessStart, "spawnBackgroundProcess");
                editor.getBufferText = _wrapAsync(editor._getBufferTextStart, "getBufferText");

                // Wrapper for deleteTheme - wraps sync function in Promise
                editor.deleteTheme = function(name) {
                    return new Promise(function(resolve, reject) {
                        const success = editor._deleteThemeSync(name);
                        if (success) {
                            resolve();
                        } else {
                            reject(new Error("Failed to delete theme: " + name));
                        }
                    });
                };
            "#.as_bytes())?;

            Ok::<_, rquickjs::Error>(())
        }).map_err(|e| anyhow!("Failed to set up global API: {}", e))?;

        Ok(())
    }

    /// Load and execute a TypeScript/JavaScript plugin from a file path
    pub async fn load_module_with_source(
        &mut self,
        path: &str,
        _plugin_source: &str,
    ) -> Result<()> {
        let path_buf = PathBuf::from(path);
        let source = std::fs::read_to_string(&path_buf)
            .map_err(|e| anyhow!("Failed to read plugin {}: {}", path, e))?;

        let filename = path_buf
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin.ts");

        // Check for ES imports - these need bundling to resolve dependencies
        if has_es_imports(&source) {
            // Try to bundle (this also strips imports and exports)
            match bundle_module(&path_buf) {
                Ok(bundled) => {
                    self.execute_js(&bundled, path)?;
                }
                Err(e) => {
                    tracing::warn!(
                        "Plugin {} uses ES imports but bundling failed: {}. Skipping.",
                        path,
                        e
                    );
                    return Ok(()); // Skip plugins with unresolvable imports
                }
            }
        } else if has_es_module_syntax(&source) {
            // Has exports but no imports - strip exports and transpile
            let stripped = strip_imports_and_exports(&source);
            let js_code = if filename.ends_with(".ts") {
                transpile_typescript(&stripped, filename)?
            } else {
                stripped
            };
            self.execute_js(&js_code, path)?;
        } else {
            // Plain code - just transpile if TypeScript
            let js_code = if filename.ends_with(".ts") {
                transpile_typescript(&source, filename)?
            } else {
                source
            };
            self.execute_js(&js_code, path)?;
        }

        Ok(())
    }

    /// Execute JavaScript code in the context
    fn execute_js(&mut self, code: &str, source_name: &str) -> Result<()> {
        // Extract plugin name from path (filename without extension)
        let plugin_name = Path::new(source_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        tracing::debug!(
            "execute_js: starting for plugin '{}' from '{}'",
            plugin_name,
            source_name
        );

        // Define getEditor() for this plugin - returns the editor global with plugin context
        let escaped_name = plugin_name.replace('\\', "\\\\").replace('"', "\\\"");
        let define_get_editor = format!(
            r#"
            globalThis.getEditor = function() {{
                // Set current plugin context for plugin-aware methods
                globalThis.__currentPluginName__ = "{}";
                return globalThis.editor;
            }};
        "#,
            escaped_name
        );

        // Wrap plugin code in IIFE for scope isolation
        let wrapped = format!("(function() {{\n{}\n}})();", code);

        self.context.with(|ctx| {
            // Define getEditor for this plugin
            ctx.eval::<(), _>(define_get_editor.as_bytes())
                .map_err(|e| format_js_error(&ctx, e, source_name))?;

            tracing::debug!(
                "execute_js: getEditor defined, now executing plugin code for '{}'",
                plugin_name
            );

            // Execute the plugin code
            let result = ctx
                .eval::<(), _>(wrapped.as_bytes())
                .map_err(|e| format_js_error(&ctx, e, source_name));

            tracing::debug!(
                "execute_js: plugin code execution finished for '{}', result: {:?}",
                plugin_name,
                result.is_ok()
            );

            result
        })
    }

    /// Emit an event to all registered handlers
    pub async fn emit(&mut self, event_name: &str, event_data: &str) -> Result<bool> {
        tracing::debug!(
            "emit: event '{}' with {} bytes of data",
            event_name,
            event_data.len()
        );

        // Track execution state for signal handler debugging
        crate::services::signal_handler::set_js_execution_state(format!("hook '{}'", event_name));

        let handlers = self.event_handlers.borrow().get(event_name).cloned();

        if let Some(handler_names) = handlers {
            if handler_names.is_empty() {
                crate::services::signal_handler::clear_js_execution_state();
                return Ok(true);
            }

            for handler_name in &handler_names {
                // Call the handler and properly handle both sync and async errors
                // Async handlers return Promises - we attach .catch() to surface rejections
                let code = format!(
                    r#"
                    (function() {{
                        try {{
                            const data = JSON.parse({});
                            if (typeof globalThis.{} === 'function') {{
                                const result = globalThis.{}(data);
                                // If handler returns a Promise, catch rejections
                                if (result && typeof result.then === 'function') {{
                                    result.catch(function(e) {{
                                        console.error('Handler {} async error:', e);
                                        // Re-throw to make it an unhandled rejection for the runtime to catch
                                        throw e;
                                    }});
                                }}
                            }}
                        }} catch (e) {{
                            console.error('Handler {} sync error:', e);
                            throw e;
                        }}
                    }})();
                    "#,
                    serde_json::to_string(event_data)?,
                    handler_name,
                    handler_name,
                    handler_name,
                    handler_name
                );

                self.context.with(|ctx| {
                    if let Err(e) = ctx.eval::<(), _>(code.as_bytes()) {
                        log_js_error(&ctx, e, &format!("handler {}", handler_name));
                    }
                    // Run pending jobs to process any Promise continuations and catch errors
                    run_pending_jobs_checked(&ctx, &format!("emit handler {}", handler_name));
                });
            }
        }

        crate::services::signal_handler::clear_js_execution_state();
        Ok(true)
    }

    /// Check if any handlers are registered for an event
    pub fn has_handlers(&self, event_name: &str) -> bool {
        self.event_handlers
            .borrow()
            .get(event_name)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    /// Start an action without waiting for async operations to complete.
    /// This is useful when the calling thread needs to continue processing
    /// ResolveCallback requests that the action may be waiting for.
    pub fn start_action(&mut self, action_name: &str) -> Result<()> {
        let handler_name = self.registered_actions.borrow().get(action_name).cloned();
        let function_name = handler_name.unwrap_or_else(|| action_name.to_string());

        // Track execution state for signal handler debugging
        crate::services::signal_handler::set_js_execution_state(format!(
            "action '{}' (fn: {})",
            action_name, function_name
        ));

        tracing::info!(
            "start_action: BEGIN '{}' -> function '{}'",
            action_name,
            function_name
        );

        // Just call the function - don't try to await or drive Promises
        let code = format!(
            r#"
            (function() {{
                console.log('[JS] start_action: calling {fn}');
                try {{
                    if (typeof globalThis.{fn} === 'function') {{
                        console.log('[JS] start_action: {fn} is a function, invoking...');
                        globalThis.{fn}();
                        console.log('[JS] start_action: {fn} invoked (may be async)');
                    }} else {{
                        console.error('[JS] Action {action} is not defined as a global function');
                    }}
                }} catch (e) {{
                    console.error('[JS] Action {action} error:', e);
                }}
            }})();
            "#,
            fn = function_name,
            action = action_name
        );

        tracing::info!("start_action: evaluating JS code");
        self.context.with(|ctx| {
            if let Err(e) = ctx.eval::<rquickjs::Value, _>(code.as_bytes()) {
                log_js_error(&ctx, e, &format!("action {}", action_name));
            }
            tracing::info!("start_action: running pending microtasks");
            // Run any immediate microtasks
            let count = run_pending_jobs_checked(&ctx, &format!("start_action {}", action_name));
            tracing::info!("start_action: executed {} pending jobs", count);
        });

        tracing::info!("start_action: END '{}'", action_name);

        // Clear execution state (action started, may still be running async)
        crate::services::signal_handler::clear_js_execution_state();

        Ok(())
    }

    /// Execute a registered action by name
    pub async fn execute_action(&mut self, action_name: &str) -> Result<()> {
        // First check if there's a registered command mapping
        let handler_name = self.registered_actions.borrow().get(action_name).cloned();
        // Use the registered handler name if found, otherwise try the action name directly
        // (defineMode bindings use global function names directly)
        let function_name = handler_name.unwrap_or_else(|| action_name.to_string());

        tracing::debug!(
            "execute_action: '{}' -> function '{}'",
            action_name,
            function_name
        );

        // Call the function and await if it returns a Promise
        // We use a global _executeActionResult to pass the result back
        let code = format!(
            r#"
            (async function() {{
                try {{
                    if (typeof globalThis.{fn} === 'function') {{
                        const result = globalThis.{fn}();
                        // If it's a Promise, await it
                        if (result && typeof result.then === 'function') {{
                            await result;
                        }}
                    }} else {{
                        console.error('Action {action} is not defined as a global function');
                    }}
                }} catch (e) {{
                    console.error('Action {action} error:', e);
                }}
            }})();
            "#,
            fn = function_name,
            action = action_name
        );

        self.context.with(|ctx| {
            // Eval returns a Promise for the async IIFE, which we need to drive
            match ctx.eval::<rquickjs::Value, _>(code.as_bytes()) {
                Ok(value) => {
                    // If it's a Promise, we need to drive the runtime to completion
                    if value.is_object() {
                        if let Some(obj) = value.as_object() {
                            // Check if it's a Promise by looking for 'then' method
                            if obj.get::<_, rquickjs::Function>("then").is_ok() {
                                // Drive the runtime to process the promise
                                // QuickJS processes promises synchronously when we call execute_pending_job
                                run_pending_jobs_checked(
                                    &ctx,
                                    &format!("execute_action {} promise", action_name),
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    log_js_error(&ctx, e, &format!("action {}", action_name));
                }
            }
        });

        Ok(())
    }

    /// Poll the event loop once to run any pending microtasks
    pub fn poll_event_loop_once(&mut self) -> bool {
        let mut had_work = false;
        self.context.with(|ctx| {
            // Run any pending microtasks (Promise continuations, etc.)
            let count = run_pending_jobs_checked(&ctx, "poll_event_loop");
            had_work = count > 0;
        });
        had_work
    }

    /// Send a status message to the editor
    pub fn send_status(&self, message: String) {
        let _ = self
            .command_sender
            .send(PluginCommand::SetStatus { message });
    }

    /// Resolve a pending async callback with a result (called from Rust when async op completes)
    pub fn resolve_callback(&mut self, callback_id: u64, result_json: &str) {
        tracing::debug!("resolve_callback: starting for callback_id={}", callback_id);
        let code = format!(
            "globalThis._resolveCallback({}, {});",
            callback_id, result_json
        );
        self.context.with(|ctx| {
            tracing::debug!("resolve_callback: evaluating JS code: {}", code);
            if let Err(e) = ctx.eval::<(), _>(code.as_bytes()) {
                log_js_error(&ctx, e, &format!("resolving callback {}", callback_id));
            }
            // IMPORTANT: Run pending jobs to process Promise continuations
            let job_count =
                run_pending_jobs_checked(&ctx, &format!("resolve_callback {}", callback_id));
            tracing::info!(
                "resolve_callback: executed {} pending jobs for callback_id={}",
                job_count,
                callback_id
            );
        });
    }

    /// Reject a pending async callback with an error (called from Rust when async op fails)
    pub fn reject_callback(&mut self, callback_id: u64, error: &str) {
        let code = format!(
            "globalThis._rejectCallback({}, {});",
            callback_id,
            serde_json::to_string(error).unwrap_or_else(|_| "\"Unknown error\"".to_string())
        );
        self.context.with(|ctx| {
            if let Err(e) = ctx.eval::<(), _>(code.as_bytes()) {
                log_js_error(&ctx, e, &format!("rejecting callback {}", callback_id));
            }
            // IMPORTANT: Run pending jobs to process Promise continuations
            run_pending_jobs_checked(&ctx, &format!("reject_callback {}", callback_id));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::plugins::api::BufferInfo;
    use std::sync::mpsc;

    /// Helper to create a backend with a command receiver for testing
    fn create_test_backend() -> (QuickJsBackend, mpsc::Receiver<PluginCommand>) {
        let (tx, rx) = mpsc::channel();
        let state_snapshot = Arc::new(RwLock::new(EditorStateSnapshot::new()));
        let dir_context = DirectoryContext::for_testing(Path::new("/tmp"));
        let backend = QuickJsBackend::with_state(state_snapshot, tx, dir_context).unwrap();
        (backend, rx)
    }

    #[test]
    fn test_quickjs_backend_creation() {
        let backend = QuickJsBackend::new();
        assert!(backend.is_ok());
    }

    #[test]
    fn test_execute_simple_js() {
        let mut backend = QuickJsBackend::new().unwrap();
        let result = backend.execute_js("const x = 1 + 2;", "test.js");
        assert!(result.is_ok());
    }

    #[test]
    fn test_event_handler_registration() {
        let backend = QuickJsBackend::new().unwrap();

        // Initially no handlers
        assert!(!backend.has_handlers("test_event"));

        // Register a handler
        backend
            .event_handlers
            .borrow_mut()
            .entry("test_event".to_string())
            .or_default()
            .push("testHandler".to_string());

        // Now has handlers
        assert!(backend.has_handlers("test_event"));
    }

    // ==================== API Tests ====================

    #[test]
    fn test_api_set_status() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.setStatus("Hello from test");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetStatus { message } => {
                assert_eq!(message, "Hello from test");
            }
            _ => panic!("Expected SetStatus command, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_register_command() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis.myTestHandler = function() { };
            editor.registerCommand("Test Command", "A test command", "myTestHandler", null);
        "#,
                "test_plugin.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::RegisterCommand { command } => {
                assert_eq!(command.name, "Test Command");
                assert_eq!(command.description, "A test command");
                // Check that source contains the plugin name (derived from filename)
                match command.source {
                    CommandSource::Plugin(name) => {
                        assert_eq!(name, "test_plugin");
                    }
                    _ => panic!("Expected Plugin source"),
                }
            }
            _ => panic!("Expected RegisterCommand, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_define_mode() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.defineMode("test-mode", null, [
                ["a", "action_a"],
                ["b", "action_b"]
            ]);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::DefineMode {
                name,
                parent,
                bindings,
                read_only,
            } => {
                assert_eq!(name, "test-mode");
                assert!(parent.is_none());
                assert_eq!(bindings.len(), 2);
                assert_eq!(bindings[0], ("a".to_string(), "action_a".to_string()));
                assert_eq!(bindings[1], ("b".to_string(), "action_b".to_string()));
                assert!(!read_only);
            }
            _ => panic!("Expected DefineMode, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_set_editor_mode() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.setEditorMode("vi-normal");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetEditorMode { mode } => {
                assert_eq!(mode, Some("vi-normal".to_string()));
            }
            _ => panic!("Expected SetEditorMode, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_clear_editor_mode() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.setEditorMode(null);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetEditorMode { mode } => {
                assert!(mode.is_none());
            }
            _ => panic!("Expected SetEditorMode with None, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_insert_at_cursor() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.insertAtCursor("Hello, World!");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::InsertAtCursor { text } => {
                assert_eq!(text, "Hello, World!");
            }
            _ => panic!("Expected InsertAtCursor, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_set_context() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.setContext("myContext", true);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetContext { name, active } => {
                assert_eq!(name, "myContext");
                assert!(active);
            }
            _ => panic!("Expected SetContext, got {:?}", cmd),
        }
    }

    #[tokio::test]
    async fn test_execute_action_sync_function() {
        let (mut backend, rx) = create_test_backend();

        // Define a sync function and register it
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis.my_sync_action = function() {
                editor.setStatus("sync action executed");
            };
        "#,
                "test.js",
            )
            .unwrap();

        // Drain any setup commands
        while rx.try_recv().is_ok() {}

        // Execute the action
        backend.execute_action("my_sync_action").await.unwrap();

        // Check the command was sent
        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetStatus { message } => {
                assert_eq!(message, "sync action executed");
            }
            _ => panic!("Expected SetStatus from action, got {:?}", cmd),
        }
    }

    #[tokio::test]
    async fn test_execute_action_async_function() {
        let (mut backend, rx) = create_test_backend();

        // Define an async function
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis.my_async_action = async function() {
                await Promise.resolve();
                editor.setStatus("async action executed");
            };
        "#,
                "test.js",
            )
            .unwrap();

        // Drain any setup commands
        while rx.try_recv().is_ok() {}

        // Execute the action
        backend.execute_action("my_async_action").await.unwrap();

        // Check the command was sent (async should complete)
        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetStatus { message } => {
                assert_eq!(message, "async action executed");
            }
            _ => panic!("Expected SetStatus from async action, got {:?}", cmd),
        }
    }

    #[tokio::test]
    async fn test_execute_action_with_registered_handler() {
        let (mut backend, rx) = create_test_backend();

        // Register an action with a different handler name
        backend.registered_actions.borrow_mut().insert(
            "my_action".to_string(),
            "actual_handler_function".to_string(),
        );

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis.actual_handler_function = function() {
                editor.setStatus("handler executed");
            };
        "#,
                "test.js",
            )
            .unwrap();

        // Drain any setup commands
        while rx.try_recv().is_ok() {}

        // Execute the action by name (should resolve to handler)
        backend.execute_action("my_action").await.unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetStatus { message } => {
                assert_eq!(message, "handler executed");
            }
            _ => panic!("Expected SetStatus, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_on_event_registration() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis.myEventHandler = function() { };
            editor.on("bufferSave", "myEventHandler");
        "#,
                "test.js",
            )
            .unwrap();

        assert!(backend.has_handlers("bufferSave"));
    }

    #[test]
    fn test_api_off_event_unregistration() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis.myEventHandler = function() { };
            editor.on("bufferSave", "myEventHandler");
            editor.off("bufferSave", "myEventHandler");
        "#,
                "test.js",
            )
            .unwrap();

        // Handler should be removed
        assert!(!backend.has_handlers("bufferSave"));
    }

    #[tokio::test]
    async fn test_emit_event() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis.onSaveHandler = function(data) {
                editor.setStatus("saved: " + JSON.stringify(data));
            };
            editor.on("bufferSave", "onSaveHandler");
        "#,
                "test.js",
            )
            .unwrap();

        // Drain setup commands
        while rx.try_recv().is_ok() {}

        // Emit the event
        backend
            .emit("bufferSave", r#"{"path": "/test.txt"}"#)
            .await
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetStatus { message } => {
                assert!(message.contains("/test.txt"));
            }
            _ => panic!("Expected SetStatus from event handler, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_copy_to_clipboard() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.copyToClipboard("clipboard text");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetClipboard { text } => {
                assert_eq!(text, "clipboard text");
            }
            _ => panic!("Expected SetClipboard, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_open_file() {
        let (mut backend, rx) = create_test_backend();

        // openFile takes (path, line?, column?)
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.openFile("/path/to/file.txt", null, null);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::OpenFileAtLocation { path, line, column } => {
                assert_eq!(path.to_str().unwrap(), "/path/to/file.txt");
                assert!(line.is_none());
                assert!(column.is_none());
            }
            _ => panic!("Expected OpenFileAtLocation, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_delete_range() {
        let (mut backend, rx) = create_test_backend();

        // deleteRange takes (buffer_id, start, end)
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.deleteRange(0, 10, 20);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::DeleteRange { range, .. } => {
                assert_eq!(range.start, 10);
                assert_eq!(range.end, 20);
            }
            _ => panic!("Expected DeleteRange, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_insert_text() {
        let (mut backend, rx) = create_test_backend();

        // insertText takes (buffer_id, position, text)
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.insertText(0, 5, "inserted");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::InsertText { position, text, .. } => {
                assert_eq!(position, 5);
                assert_eq!(text, "inserted");
            }
            _ => panic!("Expected InsertText, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_set_buffer_cursor() {
        let (mut backend, rx) = create_test_backend();

        // setBufferCursor takes (buffer_id, position)
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.setBufferCursor(0, 100);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetBufferCursor { position, .. } => {
                assert_eq!(position, 100);
            }
            _ => panic!("Expected SetBufferCursor, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_get_cursor_position_from_state() {
        let (tx, _rx) = mpsc::channel();
        let state_snapshot = Arc::new(RwLock::new(EditorStateSnapshot::new()));

        // Set up cursor position in state
        {
            let mut state = state_snapshot.write().unwrap();
            state.primary_cursor = Some(CursorInfo {
                position: 42,
                selection: None,
            });
        }

        let dir_context = DirectoryContext::for_testing(Path::new("/tmp"));
        let mut backend = QuickJsBackend::with_state(state_snapshot, tx, dir_context).unwrap();

        // Execute JS that reads and stores cursor position
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            const pos = editor.getCursorPosition();
            globalThis._testResult = pos;
        "#,
                "test.js",
            )
            .unwrap();

        // Verify by reading back - getCursorPosition returns byte offset as u32
        backend.context.with(|ctx| {
            let global = ctx.globals();
            let result: u32 = global.get("_testResult").unwrap();
            assert_eq!(result, 42);
        });
    }

    #[test]
    fn test_api_path_functions() {
        let (mut backend, _rx) = create_test_backend();

        // pathJoin takes an array of path parts
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis._dirname = editor.pathDirname("/foo/bar/baz.txt");
            globalThis._basename = editor.pathBasename("/foo/bar/baz.txt");
            globalThis._extname = editor.pathExtname("/foo/bar/baz.txt");
            globalThis._isAbsolute = editor.pathIsAbsolute("/foo/bar");
            globalThis._isRelative = editor.pathIsAbsolute("foo/bar");
            globalThis._joined = editor.pathJoin(["/foo", "bar", "baz"]);
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            assert_eq!(global.get::<_, String>("_dirname").unwrap(), "/foo/bar");
            assert_eq!(global.get::<_, String>("_basename").unwrap(), "baz.txt");
            assert_eq!(global.get::<_, String>("_extname").unwrap(), ".txt");
            assert!(global.get::<_, bool>("_isAbsolute").unwrap());
            assert!(!global.get::<_, bool>("_isRelative").unwrap());
            assert_eq!(global.get::<_, String>("_joined").unwrap(), "/foo/bar/baz");
        });
    }

    #[test]
    fn test_typescript_transpilation() {
        use crate::services::plugins::transpile::transpile_typescript;

        let (mut backend, rx) = create_test_backend();

        // TypeScript code with type annotations
        let ts_code = r#"
            const editor = getEditor();
            function greet(name: string): string {
                return "Hello, " + name;
            }
            editor.setStatus(greet("TypeScript"));
        "#;

        // Transpile to JavaScript first
        let js_code = transpile_typescript(ts_code, "test.ts").unwrap();

        // Execute the transpiled JavaScript
        backend.execute_js(&js_code, "test.js").unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetStatus { message } => {
                assert_eq!(message, "Hello, TypeScript");
            }
            _ => panic!("Expected SetStatus, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_get_buffer_text_sends_command() {
        let (mut backend, rx) = create_test_backend();

        // Call getBufferText - this returns a Promise and sends the command
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            // Store the promise for later
            globalThis._textPromise = editor.getBufferText(0, 10, 20);
        "#,
                "test.js",
            )
            .unwrap();

        // Verify the GetBufferText command was sent
        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::GetBufferText {
                buffer_id,
                start,
                end,
                request_id,
            } => {
                assert_eq!(buffer_id.0, 0);
                assert_eq!(start, 10);
                assert_eq!(end, 20);
                assert!(request_id > 0); // Should have a valid request ID
            }
            _ => panic!("Expected GetBufferText, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_get_buffer_text_resolves_callback() {
        let (mut backend, rx) = create_test_backend();

        // Call getBufferText and set up a handler for when it resolves
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis._resolvedText = null;
            editor.getBufferText(0, 0, 100).then(text => {
                globalThis._resolvedText = text;
            });
        "#,
                "test.js",
            )
            .unwrap();

        // Get the request_id from the command
        let request_id = match rx.try_recv().unwrap() {
            PluginCommand::GetBufferText { request_id, .. } => request_id,
            cmd => panic!("Expected GetBufferText, got {:?}", cmd),
        };

        // Simulate the editor responding with the text
        backend.resolve_callback(request_id, "\"hello world\"");

        // Drive the Promise to completion
        backend.context.with(|ctx| {
            run_pending_jobs_checked(&ctx, "test async getText");
        });

        // Verify the Promise resolved with the text
        backend.context.with(|ctx| {
            let global = ctx.globals();
            let result: String = global.get("_resolvedText").unwrap();
            assert_eq!(result, "hello world");
        });
    }

    #[test]
    fn test_plugin_translation() {
        let (mut backend, _rx) = create_test_backend();

        // The t() function should work (returns key if translation not found)
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis._translated = editor.t("test.key");
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            // Without actual translations, it returns the key
            let result: String = global.get("_translated").unwrap();
            assert_eq!(result, "test.key");
        });
    }

    // ==================== Line Indicator Tests ====================

    #[test]
    fn test_api_set_line_indicator() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.setLineIndicator(1, 5, "test-ns", "●", 255, 0, 0, 10);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetLineIndicator {
                buffer_id,
                line,
                namespace,
                symbol,
                color,
                priority,
            } => {
                assert_eq!(buffer_id.0, 1);
                assert_eq!(line, 5);
                assert_eq!(namespace, "test-ns");
                assert_eq!(symbol, "●");
                assert_eq!(color, (255, 0, 0));
                assert_eq!(priority, 10);
            }
            _ => panic!("Expected SetLineIndicator, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_clear_line_indicators() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.clearLineIndicators(1, "test-ns");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::ClearLineIndicators {
                buffer_id,
                namespace,
            } => {
                assert_eq!(buffer_id.0, 1);
                assert_eq!(namespace, "test-ns");
            }
            _ => panic!("Expected ClearLineIndicators, got {:?}", cmd),
        }
    }

    // ==================== Virtual Buffer Tests ====================

    #[test]
    fn test_api_create_virtual_buffer_sends_command() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.createVirtualBuffer({
                name: "*Test Buffer*",
                mode: "test-mode",
                readOnly: true,
                entries: [
                    { text: "Line 1\n", properties: { type: "header" } },
                    { text: "Line 2\n", properties: { type: "content" } }
                ],
                showLineNumbers: false,
                showCursors: true,
                editingDisabled: true
            });
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::CreateVirtualBufferWithContent {
                name,
                mode,
                read_only,
                entries,
                show_line_numbers,
                show_cursors,
                editing_disabled,
                ..
            } => {
                assert_eq!(name, "*Test Buffer*");
                assert_eq!(mode, "test-mode");
                assert!(read_only);
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].text, "Line 1\n");
                assert!(!show_line_numbers);
                assert!(show_cursors);
                assert!(editing_disabled);
            }
            _ => panic!("Expected CreateVirtualBufferWithContent, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_set_virtual_buffer_content() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.setVirtualBufferContent(5, [
                { text: "New content\n", properties: { type: "updated" } }
            ]);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetVirtualBufferContent { buffer_id, entries } => {
                assert_eq!(buffer_id.0, 5);
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].text, "New content\n");
            }
            _ => panic!("Expected SetVirtualBufferContent, got {:?}", cmd),
        }
    }

    // ==================== Overlay Tests ====================

    #[test]
    fn test_api_add_overlay() {
        let (mut backend, rx) = create_test_backend();

        backend.execute_js(r#"
            const editor = getEditor();
            editor.addOverlay(1, "highlight", 10, 20, 255, 128, 0, false, true, false, 50, 50, 50, false);
        "#, "test.js").unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::AddOverlay {
                buffer_id,
                namespace,
                range,
                color,
                bg_color,
                underline,
                bold,
                italic,
                extend_to_line_end,
            } => {
                assert_eq!(buffer_id.0, 1);
                assert!(namespace.is_some());
                assert_eq!(namespace.unwrap().as_str(), "highlight");
                assert_eq!(range, 10..20);
                assert_eq!(color, (255, 128, 0));
                assert_eq!(bg_color, Some((50, 50, 50)));
                assert!(!underline);
                assert!(bold);
                assert!(!italic);
                assert!(!extend_to_line_end);
            }
            _ => panic!("Expected AddOverlay, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_clear_namespace() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.clearNamespace(1, "highlight");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::ClearNamespace {
                buffer_id,
                namespace,
            } => {
                assert_eq!(buffer_id.0, 1);
                assert_eq!(namespace.as_str(), "highlight");
            }
            _ => panic!("Expected ClearNamespace, got {:?}", cmd),
        }
    }

    // ==================== Theme Tests ====================

    #[test]
    fn test_api_get_theme_schema() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            const schema = editor.getThemeSchema();
            globalThis._isObject = typeof schema === 'object' && schema !== null;
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let is_object: bool = global.get("_isObject").unwrap();
            // getThemeSchema should return an object
            assert!(is_object);
        });
    }

    #[test]
    fn test_api_get_builtin_themes() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            const themes = editor.getBuiltinThemes();
            globalThis._isObject = typeof themes === 'object' && themes !== null;
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let is_object: bool = global.get("_isObject").unwrap();
            // getBuiltinThemes should return an object
            assert!(is_object);
        });
    }

    #[test]
    fn test_api_apply_theme() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.applyTheme("dark");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::ApplyTheme { theme_name } => {
                assert_eq!(theme_name, "dark");
            }
            _ => panic!("Expected ApplyTheme, got {:?}", cmd),
        }
    }

    // ==================== Buffer Operations Tests ====================

    #[test]
    fn test_api_close_buffer() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.closeBuffer(3);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::CloseBuffer { buffer_id } => {
                assert_eq!(buffer_id.0, 3);
            }
            _ => panic!("Expected CloseBuffer, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_focus_split() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.focusSplit(2);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::FocusSplit { split_id } => {
                assert_eq!(split_id.0, 2);
            }
            _ => panic!("Expected FocusSplit, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_list_buffers() {
        let (tx, _rx) = mpsc::channel();
        let state_snapshot = Arc::new(RwLock::new(EditorStateSnapshot::new()));

        // Add some buffers to state
        {
            let mut state = state_snapshot.write().unwrap();
            state.buffers.insert(
                BufferId(0),
                BufferInfo {
                    id: BufferId(0),
                    path: Some(PathBuf::from("/test1.txt")),
                    modified: false,
                    length: 100,
                },
            );
            state.buffers.insert(
                BufferId(1),
                BufferInfo {
                    id: BufferId(1),
                    path: Some(PathBuf::from("/test2.txt")),
                    modified: true,
                    length: 200,
                },
            );
        }

        let dir_context = DirectoryContext::for_testing(Path::new("/tmp"));
        let mut backend = QuickJsBackend::with_state(state_snapshot, tx, dir_context).unwrap();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            const buffers = editor.listBuffers();
            globalThis._isArray = Array.isArray(buffers);
            globalThis._length = buffers.length;
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let is_array: bool = global.get("_isArray").unwrap();
            let length: u32 = global.get("_length").unwrap();
            assert!(is_array);
            assert_eq!(length, 2);
        });
    }

    // ==================== Prompt Tests ====================

    #[test]
    fn test_api_start_prompt() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.startPrompt("Enter value:", "test-prompt");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::StartPrompt { label, prompt_type } => {
                assert_eq!(label, "Enter value:");
                assert_eq!(prompt_type, "test-prompt");
            }
            _ => panic!("Expected StartPrompt, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_start_prompt_with_initial() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.startPromptWithInitial("Enter value:", "test-prompt", "default");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::StartPromptWithInitial {
                label,
                prompt_type,
                initial_value,
            } => {
                assert_eq!(label, "Enter value:");
                assert_eq!(prompt_type, "test-prompt");
                assert_eq!(initial_value, "default");
            }
            _ => panic!("Expected StartPromptWithInitial, got {:?}", cmd),
        }
    }

    #[test]
    fn test_api_set_prompt_suggestions() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.setPromptSuggestions([
                { text: "Option 1", value: "opt1" },
                { text: "Option 2", value: "opt2" }
            ]);
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::SetPromptSuggestions { suggestions } => {
                assert_eq!(suggestions.len(), 2);
                assert_eq!(suggestions[0].text, "Option 1");
                assert_eq!(suggestions[0].value, Some("opt1".to_string()));
            }
            _ => panic!("Expected SetPromptSuggestions, got {:?}", cmd),
        }
    }

    // ==================== State Query Tests ====================

    #[test]
    fn test_api_get_active_buffer_id() {
        let (tx, _rx) = mpsc::channel();
        let state_snapshot = Arc::new(RwLock::new(EditorStateSnapshot::new()));

        {
            let mut state = state_snapshot.write().unwrap();
            state.active_buffer_id = BufferId(42);
        }

        let dir_context = DirectoryContext::for_testing(Path::new("/tmp"));
        let mut backend = QuickJsBackend::with_state(state_snapshot, tx, dir_context).unwrap();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis._activeId = editor.getActiveBufferId();
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let result: u32 = global.get("_activeId").unwrap();
            assert_eq!(result, 42);
        });
    }

    #[test]
    fn test_api_get_active_split_id() {
        let (tx, _rx) = mpsc::channel();
        let state_snapshot = Arc::new(RwLock::new(EditorStateSnapshot::new()));

        {
            let mut state = state_snapshot.write().unwrap();
            state.active_split_id = 7;
        }

        let dir_context = DirectoryContext::for_testing(Path::new("/tmp"));
        let mut backend = QuickJsBackend::with_state(state_snapshot, tx, dir_context).unwrap();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis._splitId = editor.getActiveSplitId();
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let result: u32 = global.get("_splitId").unwrap();
            assert_eq!(result, 7);
        });
    }

    // ==================== File System Tests ====================

    #[test]
    fn test_api_file_exists() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            // Test with a path that definitely exists
            globalThis._exists = editor.fileExists("/");
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let result: bool = global.get("_exists").unwrap();
            assert!(result);
        });
    }

    #[test]
    fn test_api_get_cwd() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis._cwd = editor.getCwd();
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let result: String = global.get("_cwd").unwrap();
            // Should return some path
            assert!(!result.is_empty());
        });
    }

    #[test]
    fn test_api_get_env() {
        let (mut backend, _rx) = create_test_backend();

        // Set a test environment variable
        std::env::set_var("TEST_PLUGIN_VAR", "test_value");

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis._envVal = editor.getEnv("TEST_PLUGIN_VAR");
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let result: Option<String> = global.get("_envVal").unwrap();
            assert_eq!(result, Some("test_value".to_string()));
        });

        std::env::remove_var("TEST_PLUGIN_VAR");
    }

    #[test]
    fn test_api_get_config() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            const config = editor.getConfig();
            globalThis._isObject = typeof config === 'object';
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let is_object: bool = global.get("_isObject").unwrap();
            // getConfig should return an object, not a string
            assert!(is_object);
        });
    }

    #[test]
    fn test_api_get_themes_dir() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            globalThis._themesDir = editor.getThemesDir();
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let result: String = global.get("_themesDir").unwrap();
            // Should return some path
            assert!(!result.is_empty());
        });
    }

    // ==================== Read Dir Test ====================

    #[test]
    fn test_api_read_dir() {
        let (mut backend, _rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            const entries = editor.readDir("/tmp");
            globalThis._isArray = Array.isArray(entries);
            globalThis._length = entries.length;
        "#,
                "test.js",
            )
            .unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            let is_array: bool = global.get("_isArray").unwrap();
            let length: u32 = global.get("_length").unwrap();
            // /tmp should exist and readDir should return an array
            assert!(is_array);
            // Length is checked (could be 0 or more)
            assert!(length >= 0);
        });
    }

    // ==================== Execute Action Test ====================

    #[test]
    fn test_api_execute_action() {
        let (mut backend, rx) = create_test_backend();

        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.executeAction("move_cursor_up");
        "#,
                "test.js",
            )
            .unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::ExecuteAction { action_name } => {
                assert_eq!(action_name, "move_cursor_up");
            }
            _ => panic!("Expected ExecuteAction, got {:?}", cmd),
        }
    }

    // ==================== Debug Test ====================

    #[test]
    fn test_api_debug() {
        let (mut backend, _rx) = create_test_backend();

        // debug() should not panic and should work with any input
        backend
            .execute_js(
                r#"
            const editor = getEditor();
            editor.debug("Test debug message");
            editor.debug("Another message with special chars: <>&\"'");
        "#,
                "test.js",
            )
            .unwrap();
        // If we get here without panic, the test passes
    }
}
