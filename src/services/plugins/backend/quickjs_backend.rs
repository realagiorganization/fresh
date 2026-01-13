//! QuickJS JavaScript runtime backend for TypeScript plugins
//!
//! This module provides a JavaScript runtime using QuickJS for executing
//! TypeScript plugins. TypeScript is transpiled to JavaScript using oxc.

use crate::config_io::DirectoryContext;
use crate::input::commands::{Command, CommandSource};
use crate::input::keybindings::Action;
use crate::model::event::{BufferId, SplitId};
use crate::primitives::text_property::TextPropertyEntry;
use crate::services::plugins::api::{EditorStateSnapshot, PluginCommand, PluginResponse};
#[cfg(test)]
use crate::services::plugins::api::CursorInfo;
use crate::services::plugins::transpile::{bundle_module, has_es_imports, has_es_module_syntax, strip_imports_and_exports, transpile_typescript};
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
        Type::Bool => val.as_bool().map(serde_json::Value::Bool).unwrap_or(serde_json::Value::Null),
        Type::Int => val.as_int().map(|n| serde_json::Value::Number(n.into())).unwrap_or(serde_json::Value::Null),
        Type::Float => {
            val.as_float()
                .and_then(|f| serde_json::Number::from_f64(f))
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        }
        Type::String => {
            val.as_string()
                .and_then(|s| s.to_string().ok())
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null)
        }
        Type::Array => {
            if let Some(arr) = val.as_array() {
                let items: Vec<serde_json::Value> = arr.iter()
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
fn json_to_js<'js>(ctx: &rquickjs::Ctx<'js>, val: serde_json::Value) -> rquickjs::Result<Value<'js>> {
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
    let cursor_pos = match snap.buffer_cursor_positions.get(&buffer_id_typed).copied()
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
    let result: Vec<_> = properties.iter()
        .filter(|prop| prop.start <= cursor_pos && cursor_pos < prop.end)
        .map(|prop| serde_json::Value::Object(
            prop.properties.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        ))
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
        Type::String => val.as_string()
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
        Type::Function | Type::Constructor => {
            "[function]".to_string()
        }
        Type::Symbol => "[symbol]".to_string(),
        Type::BigInt => val.as_big_int()
            .and_then(|b| b.clone().to_i64().ok())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "[bigint]".to_string()),
        _ => format!("[{}]", val.type_name()),
    }
}

/// Format a JavaScript error with full details including stack trace
fn format_js_error(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error, source_name: &str) -> anyhow::Error {
    // Check if this is an exception that we can catch for more details
    if err.is_exception() {
        // Try to catch the exception to get the full error object
        let exc = ctx.catch();
        if !exc.is_undefined() && !exc.is_null() {
            // Try to get error message and stack from the exception object
            if let Some(exc_obj) = exc.as_object() {
                let message: String = exc_obj.get::<_, String>("message").unwrap_or_else(|_| "Unknown error".to_string());
                let stack: String = exc_obj.get::<_, String>("stack").unwrap_or_default();
                let name: String = exc_obj.get::<_, String>("name").unwrap_or_else(|_| "Error".to_string());

                if !stack.is_empty() {
                    return anyhow::anyhow!(
                        "JS error in {}: {}: {}\nStack trace:\n{}",
                        source_name, name, message, stack
                    );
                } else {
                    return anyhow::anyhow!(
                        "JS error in {}: {}: {}",
                        source_name, name, message
                    );
                }
            } else {
                // Exception is not an object, try to convert to string
                let exc_str: String = exc.as_string()
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
static PANIC_ON_JS_ERRORS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
                format!("{}: {}", err.message().unwrap_or_default(), err.stack().unwrap_or_default())
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
            format!("{}: {}", err.message().unwrap_or_default(), err.stack().unwrap_or_default())
        } else {
            format!("{:?}", exc)
        };
        tracing::error!("Unhandled JS exception after running jobs in {}: {}", context, error_msg);
        if should_panic_on_js_errors() {
            panic!("Unhandled JS exception after running jobs in {}: {}", context, error_msg);
        }
    }

    count
}

/// Parse a TextPropertyEntry from a JS Object
fn parse_text_property_entry(ctx: &rquickjs::Ctx<'_>, obj: &Object<'_>) -> Option<TextPropertyEntry> {
    let text: String = obj.get("text").ok()?;
    let properties: HashMap<String, serde_json::Value> = obj.get::<_, Object>("properties")
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
        Self::with_state_and_responses(state_snapshot, command_sender, pending_responses, dir_context)
    }

    /// Create a new QuickJS backend with editor state and shared pending responses
    pub fn with_state_and_responses(
        state_snapshot: Arc<RwLock<EditorStateSnapshot>>,
        command_sender: mpsc::Sender<PluginCommand>,
        pending_responses: PendingResponses,
        dir_context: DirectoryContext,
    ) -> Result<Self> {
        tracing::debug!("QuickJsBackend::new: creating QuickJS runtime");

        let runtime = Runtime::new().map_err(|e| anyhow!("Failed to create QuickJS runtime: {}", e))?;

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

        let context = Context::full(&runtime).map_err(|e| anyhow!("Failed to create QuickJS context: {}", e))?;

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

            // Create the editor object
            let editor = Object::new(ctx.clone())?;

            // === Logging ===
            editor.set("debug", Function::new(ctx.clone(), |msg: String| {
                tracing::info!("Plugin.debug: {}", msg);
            })?)?;

            editor.set("info", Function::new(ctx.clone(), |msg: String| {
                tracing::info!("Plugin: {}", msg);
            })?)?;

            editor.set("warn", Function::new(ctx.clone(), |msg: String| {
                tracing::warn!("Plugin: {}", msg);
            })?)?;

            editor.set("error", Function::new(ctx.clone(), |msg: String| {
                tracing::error!("Plugin: {}", msg);
            })?)?;

            // === Status ===
            let cmd_sender = command_sender.clone();
            editor.set("setStatus", Function::new(ctx.clone(), move |msg: String| {
                let _ = cmd_sender.send(PluginCommand::SetStatus { message: msg });
            })?)?;

            // === Clipboard ===
            let cmd_sender = command_sender.clone();
            editor.set("copyToClipboard", Function::new(ctx.clone(), move |text: String| {
                let _ = cmd_sender.send(PluginCommand::SetClipboard { text });
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("setClipboard", Function::new(ctx.clone(), move |text: String| {
                let _ = cmd_sender.send(PluginCommand::SetClipboard { text });
            })?)?;

            // === Buffer queries ===
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getActiveBufferId", Function::new(ctx.clone(), move || -> u32 {
                snapshot.read().map(|s| s.active_buffer_id.0 as u32).unwrap_or(0)
            })?)?;

            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getActiveSplitId", Function::new(ctx.clone(), move || -> u32 {
                snapshot.read().map(|s| s.active_split_id as u32).unwrap_or(0)
            })?)?;

            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getCursorPosition", Function::new(ctx.clone(), move || -> u32 {
                snapshot.read()
                    .ok()
                    .and_then(|s| s.primary_cursor.as_ref().map(|c| c.position as u32))
                    .unwrap_or(0)
            })?)?;

            // getCursorLine - returns cursor line number (1-indexed)
            // Note: Line number is computed from byte position, requires buffer content
            // For now returns 0 as placeholder - proper implementation needs buffer access
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getCursorLine", Function::new(ctx.clone(), move || -> u32 {
                // TODO: Implement proper line counting when buffer content is available
                // For now, return 1 as a reasonable default
                snapshot.read()
                    .ok()
                    .and_then(|_s| Some(1u32))
                    .unwrap_or(1)
            })?)?;

            // getPrimaryCursor - returns primary cursor info as JSON
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getPrimaryCursor", Function::new(ctx.clone(), move || -> String {
                snapshot.read()
                    .ok()
                    .and_then(|s| s.primary_cursor.as_ref().map(|c| {
                        let selection = c.selection.as_ref().map(|sel| {
                            serde_json::json!({"start": sel.start as u32, "end": sel.end as u32})
                        });
                        serde_json::json!({
                            "position": c.position as u32,
                            "selection": selection
                        }).to_string()
                    }))
                    .unwrap_or_else(|| "null".to_string())
            })?)?;

            // getAllCursors - returns all cursors as JSON array
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getAllCursors", Function::new(ctx.clone(), move || -> String {
                snapshot.read()
                    .map(|s| {
                        let cursors: Vec<serde_json::Value> = s.all_cursors.iter().map(|c| {
                            let selection = c.selection.as_ref().map(|sel| {
                                serde_json::json!({"start": sel.start as u32, "end": sel.end as u32})
                            });
                            serde_json::json!({
                                "position": c.position as u32,
                                "selection": selection
                            })
                        }).collect();
                        serde_json::to_string(&cursors).unwrap_or_else(|_| "[]".to_string())
                    })
                    .unwrap_or_else(|_| "[]".to_string())
            })?)?;

            // getAllCursorPositions - returns cursor positions as array of u32
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getAllCursorPositions", Function::new(ctx.clone(), move || -> Vec<u32> {
                snapshot.read()
                    .map(|s| s.all_cursors.iter().map(|c| c.position as u32).collect())
                    .unwrap_or_default()
            })?)?;

            // getViewport - returns viewport info as JSON
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getViewport", Function::new(ctx.clone(), move || -> String {
                snapshot.read()
                    .ok()
                    .and_then(|s| s.viewport.as_ref().map(|v| {
                        serde_json::json!({
                            "top_byte": v.top_byte as u32,
                            "left_column": v.left_column as u32,
                            "width": v.width as u32,
                            "height": v.height as u32
                        }).to_string()
                    }))
                    .unwrap_or_else(|| "null".to_string())
            })?)?;

            // getBufferInfo - returns buffer info as JSON
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getBufferInfo", Function::new(ctx.clone(), move |buffer_id: u32| -> String {
                snapshot.read()
                    .ok()
                    .and_then(|s| s.buffers.get(&BufferId(buffer_id as usize)).map(|b| {
                        serde_json::json!({
                            "id": b.id.0 as u32,
                            "path": b.path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                            "modified": b.modified,
                            "length": b.length as u32
                        }).to_string()
                    }))
                    .unwrap_or_else(|| "null".to_string())
            })?)?;

            // getBufferSavedDiff - returns diff vs last saved as JSON
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getBufferSavedDiff", Function::new(ctx.clone(), move |buffer_id: u32| -> String {
                snapshot.read()
                    .ok()
                    .and_then(|s| s.buffer_saved_diffs.get(&BufferId(buffer_id as usize)).map(|diff| {
                        let ranges: Vec<serde_json::Value> = diff.byte_ranges.iter().map(|r| {
                            serde_json::json!({"start": r.start as u32, "end": r.end as u32})
                        }).collect();
                        serde_json::json!({
                            "equal": diff.equal,
                            "byte_ranges": ranges
                        }).to_string()
                    }))
                    .unwrap_or_else(|| serde_json::json!({"equal": true, "byte_ranges": []}).to_string())
            })?)?;

            // getAllDiagnostics - returns all LSP diagnostics as JSON
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getAllDiagnostics", Function::new(ctx.clone(), move || -> String {
                snapshot.read()
                    .map(|s| {
                        serde_json::to_string(&s.diagnostics).unwrap_or_else(|_| "{}".to_string())
                    })
                    .unwrap_or_else(|_| "{}".to_string())
            })?)?;

            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getBufferPath", Function::new(ctx.clone(), move |buffer_id: u32| -> String {
                if let Ok(s) = snapshot.read() {
                    if let Some(b) = s.buffers.get(&BufferId(buffer_id as usize)) {
                        if let Some(p) = &b.path {
                            return p.to_string_lossy().to_string();
                        }
                    }
                }
                String::new()
            })?)?;

            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getBufferLength", Function::new(ctx.clone(), move |buffer_id: u32| -> u32 {
                if let Ok(s) = snapshot.read() {
                    if let Some(b) = s.buffers.get(&BufferId(buffer_id as usize)) {
                        return b.length as u32;
                    }
                }
                0
            })?)?;

            let snapshot = Arc::clone(&state_snapshot);
            editor.set("isBufferModified", Function::new(ctx.clone(), move |buffer_id: u32| -> bool {
                if let Ok(s) = snapshot.read() {
                    if let Some(b) = s.buffers.get(&BufferId(buffer_id as usize)) {
                        return b.modified;
                    }
                }
                false
            })?)?;

            // listBuffers - returns JSON array of {id, path, modified, length}
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("_listBuffersJson", Function::new(ctx.clone(), move || -> String {
                if let Ok(s) = snapshot.read() {
                    let buffers: Vec<serde_json::Value> = s.buffers.iter().map(|(id, info)| {
                        serde_json::json!({
                            "id": id.0 as u32,
                            "path": info.path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                            "modified": info.modified,
                            "length": info.length as u32
                        })
                    }).collect();
                    serde_json::to_string(&buffers).unwrap_or_else(|_| "[]".to_string())
                } else {
                    "[]".to_string()
                }
            })?)?;

            // === Text editing ===
            let cmd_sender = command_sender.clone();
            editor.set("insertText", Function::new(ctx.clone(), move |buffer_id: u32, position: u32, text: String| -> bool {
                cmd_sender.send(PluginCommand::InsertText {
                    buffer_id: BufferId(buffer_id as usize),
                    position: position as usize,
                    text,
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("deleteRange", Function::new(ctx.clone(), move |buffer_id: u32, start: u32, end: u32| -> bool {
                cmd_sender.send(PluginCommand::DeleteRange {
                    buffer_id: BufferId(buffer_id as usize),
                    range: (start as usize)..(end as usize),
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("insertAtCursor", Function::new(ctx.clone(), move |text: String| -> bool {
                cmd_sender.send(PluginCommand::InsertAtCursor { text }).is_ok()
            })?)?;

            // === File operations ===
            let cmd_sender = command_sender.clone();
            editor.set("openFile", Function::new(ctx.clone(), move |path: String, line: Option<u32>, column: Option<u32>| -> bool {
                cmd_sender.send(PluginCommand::OpenFileAtLocation {
                    path: PathBuf::from(path),
                    line: line.map(|l| l as usize),
                    column: column.map(|c| c as usize),
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("showBuffer", Function::new(ctx.clone(), move |buffer_id: u32| -> bool {
                cmd_sender.send(PluginCommand::ShowBuffer {
                    buffer_id: BufferId(buffer_id as usize),
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("closeBuffer", Function::new(ctx.clone(), move |buffer_id: u32| -> bool {
                cmd_sender.send(PluginCommand::CloseBuffer {
                    buffer_id: BufferId(buffer_id as usize),
                }).is_ok()
            })?)?;

            // === Event handling ===
            let handlers = Rc::clone(&event_handlers);
            editor.set("on", Function::new(ctx.clone(), move |event_name: String, handler_name: String| {
                let mut h = handlers.borrow_mut();
                h.entry(event_name).or_default().push(handler_name);
            })?)?;

            let handlers = Rc::clone(&event_handlers);
            editor.set("off", Function::new(ctx.clone(), move |event_name: String, handler_name: String| {
                let mut h = handlers.borrow_mut();
                if let Some(handlers) = h.get_mut(&event_name) {
                    handlers.retain(|h| h != &handler_name);
                }
            })?)?;

            // === Command registration ===
            // _registerCommandInternal(pluginName, name, description, handler_name, context)
            // Called by JS wrapper which provides plugin name
            let cmd_sender = command_sender.clone();
            let actions = Rc::clone(&registered_actions);
            editor.set("_registerCommandInternal", Function::new(ctx.clone(), move |plugin_name: String, name: String, description: String, handler_name: String, context: Option<String>| -> bool {
                tracing::debug!("registerCommand: plugin='{}', name='{}', handler='{}', context={:?}", plugin_name, name, handler_name, context);

                // Store action handler mapping (handler_name -> handler_name for direct lookup)
                actions.borrow_mut().insert(handler_name.clone(), handler_name.clone());

                // Register with editor - action uses handler_name so execute_action can find it
                // source uses plugin_name for proper i18n localization
                let command = Command {
                    name: name.clone(),
                    description,
                    action: Action::PluginAction(handler_name.clone()),
                    contexts: vec![],
                    custom_contexts: context.into_iter().collect(),
                    source: CommandSource::Plugin(plugin_name),
                };

                cmd_sender.send(PluginCommand::RegisterCommand { command }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("unregisterCommand", Function::new(ctx.clone(), move |name: String| -> bool {
                cmd_sender.send(PluginCommand::UnregisterCommand { name }).is_ok()
            })?)?;

            // === Context ===
            let cmd_sender = command_sender.clone();
            editor.set("setContext", Function::new(ctx.clone(), move |name: String, active: bool| -> bool {
                cmd_sender.send(PluginCommand::SetContext { name, active }).is_ok()
            })?)?;

            // === Action execution ===
            let cmd_sender = command_sender.clone();
            editor.set("executeAction", Function::new(ctx.clone(), move |action_name: String| -> bool {
                cmd_sender.send(PluginCommand::ExecuteAction { action_name }).is_ok()
            })?)?;

            // executeActions - execute multiple actions from JSON array
            let cmd_sender = command_sender.clone();
            editor.set("executeActions", Function::new(ctx.clone(), move |actions_json: String| -> bool {
                #[derive(serde::Deserialize)]
                struct ActionSpec {
                    action: String,
                    #[serde(default = "default_count")]
                    count: u32,
                }
                fn default_count() -> u32 { 1 }

                let actions: Vec<ActionSpec> = match serde_json::from_str(&actions_json) {
                    Ok(a) => a,
                    Err(_) => return false,
                };
                let specs: Vec<crate::services::plugins::api::ActionSpec> = actions.into_iter()
                    .map(|a| crate::services::plugins::api::ActionSpec {
                        action: a.action,
                        count: a.count,
                    })
                    .collect();
                cmd_sender.send(PluginCommand::ExecuteActions { actions: specs }).is_ok()
            })?)?;

            // getHandlers - get list of handlers for an event
            let handlers = Rc::clone(&event_handlers);
            editor.set("getHandlers", Function::new(ctx.clone(), move |event_name: String| -> Vec<String> {
                handlers.borrow().get(&event_name).cloned().unwrap_or_default()
            })?)?;

            // === i18n ===
            // _pluginTranslate(pluginName, key, args) - internal function for plugin translations
            editor.set("_pluginTranslate", Function::new(ctx.clone(), |plugin_name: String, key: String, args: Value| -> String {
                // Convert args Value to HashMap<String, String>
                let mut args_map = std::collections::HashMap::<String, String>::new();
                if let Some(args_obj) = args.as_object() {
                    for arg_key in args_obj.keys::<String>().flatten() {
                        if let Ok(val) = args_obj.get::<_, Value>(&arg_key) {
                            let s = match val.type_of() {
                                rquickjs::Type::String => val.as_string()
                                    .and_then(|s| s.to_string().ok())
                                    .unwrap_or_default(),
                                rquickjs::Type::Int => val.as_int()
                                    .map(|n| n.to_string())
                                    .unwrap_or_default(),
                                rquickjs::Type::Float => val.as_float()
                                    .map(|n| n.to_string())
                                    .unwrap_or_default(),
                                rquickjs::Type::Bool => val.as_bool()
                                    .map(|b| b.to_string())
                                    .unwrap_or_default(),
                                _ => String::new(),
                            };
                            args_map.insert(arg_key, s);
                        }
                    }
                }
                crate::i18n::translate_plugin_string(&plugin_name, &key, &args_map)
            })?)?;

            // === Environment ===
            editor.set("getEnv", Function::new(ctx.clone(), |name: String| -> Option<String> {
                std::env::var(&name).ok()
            })?)?;

            // getCwd returns current working directory (from std::env, not dir_context)
            editor.set("getCwd", Function::new(ctx.clone(), || -> String {
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| ".".to_string())
            })?)?;

            // === Path operations ===
            editor.set("pathJoin", Function::new(ctx.clone(), |parts: Vec<String>| -> String {
                let mut path = PathBuf::new();
                for part in parts {
                    if Path::new(&part).is_absolute() {
                        path = PathBuf::from(part);
                    } else {
                        path.push(part);
                    }
                }
                path.to_string_lossy().to_string()
            })?)?;

            editor.set("pathDirname", Function::new(ctx.clone(), |path: String| -> String {
                Path::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            })?)?;

            editor.set("pathBasename", Function::new(ctx.clone(), |path: String| -> String {
                Path::new(&path)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            })?)?;

            editor.set("pathExtname", Function::new(ctx.clone(), |path: String| -> String {
                Path::new(&path)
                    .extension()
                    .map(|s| format!(".{}", s.to_string_lossy()))
                    .unwrap_or_default()
            })?)?;

            editor.set("pathIsAbsolute", Function::new(ctx.clone(), |path: String| -> bool {
                Path::new(&path).is_absolute()
            })?)?;

            // === File system ===
            editor.set("fileExists", Function::new(ctx.clone(), |path: String| -> bool {
                Path::new(&path).exists()
            })?)?;

            editor.set("readFile", Function::new(ctx.clone(), |path: String| -> Option<String> {
                std::fs::read_to_string(&path).ok()
            })?)?;

            editor.set("writeFile", Function::new(ctx.clone(), |path: String, content: String| -> bool {
                std::fs::write(&path, content).is_ok()
            })?)?;

            // readDir - returns JSON array of {name, is_file, is_dir}
            editor.set("_readDirJson", Function::new(ctx.clone(), |path: String| -> String {
                match std::fs::read_dir(&path) {
                    Ok(entries) => {
                        let dir_entries: Vec<serde_json::Value> = entries
                            .filter_map(|e| e.ok())
                            .map(|entry| {
                                let file_type = entry.file_type().ok();
                                serde_json::json!({
                                    "name": entry.file_name().to_string_lossy().to_string(),
                                    "is_file": file_type.map(|ft| ft.is_file()).unwrap_or(false),
                                    "is_dir": file_type.map(|ft| ft.is_dir()).unwrap_or(false)
                                })
                            })
                            .collect();
                        serde_json::to_string(&dir_entries).unwrap_or_else(|_| "[]".to_string())
                    }
                    Err(e) => {
                        tracing::warn!("readDir failed for '{}': {}", path, e);
                        "[]".to_string()
                    }
                }
            })?)?;

            // fileStat - returns file stat info as JSON
            editor.set("fileStat", Function::new(ctx.clone(), |path: String| -> String {
                let p = Path::new(&path);
                match std::fs::metadata(&p) {
                    Ok(metadata) => {
                        serde_json::json!({
                            "exists": true,
                            "is_file": metadata.is_file(),
                            "is_dir": metadata.is_dir(),
                            "size": metadata.len() as u64,
                            "readonly": metadata.permissions().readonly()
                        }).to_string()
                    }
                    Err(_) => {
                        serde_json::json!({
                            "exists": false,
                            "is_file": false,
                            "is_dir": false,
                            "size": 0,
                            "readonly": false
                        }).to_string()
                    }
                }
            })?)?;

            // findBufferByPath - returns buffer ID for given path, or 0 if not found
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("findBufferByPath", Function::new(ctx.clone(), move |path: String| -> u32 {
                let search_path = PathBuf::from(&path);
                snapshot.read()
                    .ok()
                    .and_then(|s| {
                        s.buffers.iter().find_map(|(id, info)| {
                            if let Some(buf_path) = &info.path {
                                // Try exact match first
                                if buf_path == &search_path {
                                    return Some(id.0 as u32);
                                }
                                // Try canonical paths
                                if let (Ok(canonical_buf), Ok(canonical_search)) =
                                    (buf_path.canonicalize(), search_path.canonicalize()) {
                                    if canonical_buf == canonical_search {
                                        return Some(id.0 as u32);
                                    }
                                }
                            }
                            None
                        })
                    })
                    .unwrap_or(0)
            })?)?;

            // === Config ===
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getConfig", Function::new(ctx.clone(), move || -> String {
                snapshot.read()
                    .map(|s| s.config.to_string())
                    .unwrap_or_else(|_| "{}".to_string())
            })?)?;

            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getUserConfig", Function::new(ctx.clone(), move || -> String {
                snapshot.read()
                    .map(|s| s.user_config.to_string())
                    .unwrap_or_else(|_| "{}".to_string())
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("reloadConfig", Function::new(ctx.clone(), move || {
                let _ = cmd_sender.send(PluginCommand::ReloadConfig);
            })?)?;

            let dir_ctx = dir_context.clone();
            editor.set("getConfigDir", Function::new(ctx.clone(), move || -> String {
                dir_ctx.config_dir.to_string_lossy().to_string()
            })?)?;

            let dir_ctx = dir_context.clone();
            editor.set("getThemesDir", Function::new(ctx.clone(), move || -> String {
                dir_ctx.config_dir.join("themes").to_string_lossy().to_string()
            })?)?;

            // === Theme ===
            let cmd_sender = command_sender.clone();
            editor.set("applyTheme", Function::new(ctx.clone(), move |theme_name: String| -> bool {
                cmd_sender.send(PluginCommand::ApplyTheme { theme_name }).is_ok()
            })?)?;

            editor.set("getThemeSchema", Function::new(ctx.clone(), || -> String {
                let schema = crate::view::theme::get_theme_schema();
                serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
            })?)?;

            editor.set("getBuiltinThemes", Function::new(ctx.clone(), || -> String {
                let themes = crate::view::theme::get_builtin_themes();
                serde_json::to_string(&themes).unwrap_or_else(|_| "{}".to_string())
            })?)?;

            // deleteTheme - deletes theme file from user themes directory (sync, returns bool)
            let dir_ctx = dir_context.clone();
            editor.set("_deleteThemeSync", Function::new(ctx.clone(), move |name: String| -> bool {
                // Security: only allow deleting from the themes directory
                let themes_dir = dir_ctx.config_dir.join("themes");
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
            })?)?;

            // === Overlays ===
            // _addOverlayInternal takes JSON string to avoid arg limit
            let cmd_sender = command_sender.clone();
            editor.set("_addOverlayInternal", Function::new(ctx.clone(), move |json: String| -> bool {
                #[derive(serde::Deserialize)]
                struct OverlayArgs {
                    buffer_id: u32,
                    namespace: String,
                    start: u32,
                    end: u32,
                    r: i32, g: i32, b: i32,
                    underline: bool, bold: bool, italic: bool,
                    bg_r: i32, bg_g: i32, bg_b: i32,
                    extend_to_line_end: bool,
                }

                let args: OverlayArgs = match serde_json::from_str(&json) {
                    Ok(a) => a,
                    Err(_) => return false,
                };

                // -1 means use default color (white)
                let color = if args.r >= 0 && args.g >= 0 && args.b >= 0 {
                    (args.r as u8, args.g as u8, args.b as u8)
                } else {
                    (255, 255, 255)
                };

                // -1 for bg means no background
                let bg_color = if args.bg_r >= 0 && args.bg_g >= 0 && args.bg_b >= 0 {
                    Some((args.bg_r as u8, args.bg_g as u8, args.bg_b as u8))
                } else {
                    None
                };

                cmd_sender.send(PluginCommand::AddOverlay {
                    buffer_id: BufferId(args.buffer_id as usize),
                    namespace: Some(OverlayNamespace::from_string(args.namespace)),
                    range: (args.start as usize)..(args.end as usize),
                    color,
                    bg_color,
                    underline: args.underline,
                    bold: args.bold,
                    italic: args.italic,
                    extend_to_line_end: args.extend_to_line_end,
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("clearNamespace", Function::new(ctx.clone(), move |buffer_id: u32, namespace: String| -> bool {
                cmd_sender.send(PluginCommand::ClearNamespace {
                    buffer_id: BufferId(buffer_id as usize),
                    namespace: OverlayNamespace::from_string(namespace),
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("clearAllOverlays", Function::new(ctx.clone(), move |buffer_id: u32| -> bool {
                cmd_sender.send(PluginCommand::ClearAllOverlays {
                    buffer_id: BufferId(buffer_id as usize),
                }).is_ok()
            })?)?;

            // removeOverlay - remove a specific overlay by handle
            let cmd_sender = command_sender.clone();
            editor.set("removeOverlay", Function::new(ctx.clone(), move |buffer_id: u32, handle: String| -> bool {
                cmd_sender.send(PluginCommand::RemoveOverlay {
                    buffer_id: BufferId(buffer_id as usize),
                    handle: crate::view::overlay::OverlayHandle::from_string(handle),
                }).is_ok()
            })?)?;

            // clearOverlaysInRange - clear overlays in a byte range
            let cmd_sender = command_sender.clone();
            editor.set("clearOverlaysInRange", Function::new(ctx.clone(), move |buffer_id: u32, start: u32, end: u32| -> bool {
                cmd_sender.send(PluginCommand::ClearOverlaysInRange {
                    buffer_id: BufferId(buffer_id as usize),
                    start: start as usize,
                    end: end as usize,
                }).is_ok()
            })?)?;

            // === Virtual Text ===
            // addVirtualText - add virtual text at position (takes JSON to avoid arg limit)
            let cmd_sender = command_sender.clone();
            editor.set("addVirtualText", Function::new(ctx.clone(), move |json: String| -> bool {
                #[derive(serde::Deserialize)]
                struct Args {
                    buffer_id: u32,
                    virtual_text_id: String,
                    position: u32,
                    text: String,
                    color: (u8, u8, u8),
                    #[serde(default)]
                    before: bool,
                    #[serde(default)]
                    use_bg: bool,
                }
                let args: Args = match serde_json::from_str(&json) {
                    Ok(a) => a,
                    Err(_) => return false,
                };
                cmd_sender.send(PluginCommand::AddVirtualText {
                    buffer_id: BufferId(args.buffer_id as usize),
                    virtual_text_id: args.virtual_text_id,
                    position: args.position as usize,
                    text: args.text,
                    color: args.color,
                    before: args.before,
                    use_bg: args.use_bg,
                }).is_ok()
            })?)?;

            // removeVirtualText - remove virtual text by ID
            let cmd_sender = command_sender.clone();
            editor.set("removeVirtualText", Function::new(ctx.clone(), move |buffer_id: u32, virtual_text_id: String| -> bool {
                cmd_sender.send(PluginCommand::RemoveVirtualText {
                    buffer_id: BufferId(buffer_id as usize),
                    virtual_text_id,
                }).is_ok()
            })?)?;

            // removeVirtualTextsByPrefix - remove virtual texts by prefix
            let cmd_sender = command_sender.clone();
            editor.set("removeVirtualTextsByPrefix", Function::new(ctx.clone(), move |buffer_id: u32, prefix: String| -> bool {
                cmd_sender.send(PluginCommand::RemoveVirtualTextsByPrefix {
                    buffer_id: BufferId(buffer_id as usize),
                    prefix,
                }).is_ok()
            })?)?;

            // clearVirtualTexts - clear all virtual texts from buffer
            let cmd_sender = command_sender.clone();
            editor.set("clearVirtualTexts", Function::new(ctx.clone(), move |buffer_id: u32| -> bool {
                cmd_sender.send(PluginCommand::ClearVirtualTexts {
                    buffer_id: BufferId(buffer_id as usize),
                }).is_ok()
            })?)?;

            // clearVirtualTextNamespace - clear virtual texts in namespace
            let cmd_sender = command_sender.clone();
            editor.set("clearVirtualTextNamespace", Function::new(ctx.clone(), move |buffer_id: u32, namespace: String| -> bool {
                cmd_sender.send(PluginCommand::ClearVirtualTextNamespace {
                    buffer_id: BufferId(buffer_id as usize),
                    namespace,
                }).is_ok()
            })?)?;

            // === Virtual Lines ===
            // addVirtualLine - add a virtual line (takes JSON to avoid arg limit)
            let cmd_sender = command_sender.clone();
            editor.set("addVirtualLine", Function::new(ctx.clone(), move |json: String| -> bool {
                #[derive(serde::Deserialize)]
                struct Args {
                    buffer_id: u32,
                    position: u32,
                    text: String,
                    fg_color: (u8, u8, u8),
                    #[serde(default)]
                    bg_color: Option<(u8, u8, u8)>,
                    #[serde(default)]
                    above: bool,
                    namespace: String,
                    #[serde(default)]
                    priority: i32,
                }
                let args: Args = match serde_json::from_str(&json) {
                    Ok(a) => a,
                    Err(_) => return false,
                };
                cmd_sender.send(PluginCommand::AddVirtualLine {
                    buffer_id: BufferId(args.buffer_id as usize),
                    position: args.position as usize,
                    text: args.text,
                    fg_color: args.fg_color,
                    bg_color: args.bg_color,
                    above: args.above,
                    namespace: args.namespace,
                    priority: args.priority,
                }).is_ok()
            })?)?;

            // === Prompt (stub with warning) ===
            let cmd_sender = command_sender.clone();
            editor.set("startPrompt", Function::new(ctx.clone(), move |label: String, prompt_type: String| -> bool {
                cmd_sender.send(PluginCommand::StartPrompt { label, prompt_type }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("startPromptWithInitial", Function::new(ctx.clone(), move |label: String, prompt_type: String, initial_value: String| -> bool {
                cmd_sender.send(PluginCommand::StartPromptWithInitial { label, prompt_type, initial_value }).is_ok()
            })?)?;

            // setPromptSuggestions takes array of {text, description?, value?, disabled?, keybinding?}
            let cmd_sender = command_sender.clone();
            editor.set("setPromptSuggestions", Function::new(ctx.clone(), move |suggestions_arr: Vec<Object>| -> rquickjs::Result<bool> {
                let suggestions: Vec<crate::input::commands::Suggestion> = suggestions_arr.into_iter().map(|obj| {
                    crate::input::commands::Suggestion {
                        text: obj.get("text").unwrap_or_default(),
                        description: obj.get("description").ok(),
                        value: obj.get("value").ok(),
                        disabled: obj.get("disabled").unwrap_or(false),
                        keybinding: obj.get("keybinding").ok(),
                        source: None,
                    }
                }).collect();
                Ok(cmd_sender.send(PluginCommand::SetPromptSuggestions { suggestions }).is_ok())
            })?)?;

            // === Mode definition ===
            // defineMode(name: string, parent: string | null, bindings: [key, command][])
            let cmd_sender = command_sender.clone();
            editor.set("defineMode", Function::new(ctx.clone(), move |name: String, parent: Option<String>, bindings_arr: Vec<Vec<String>>| -> bool {
                // bindings is array of [key, command] pairs
                let bindings: Vec<(String, String)> = bindings_arr.into_iter()
                    .filter_map(|arr| {
                        if arr.len() >= 2 {
                            Some((arr[0].clone(), arr[1].clone()))
                        } else {
                            None
                        }
                    })
                    .collect();

                cmd_sender.send(PluginCommand::DefineMode {
                    name,
                    parent,
                    bindings,
                    read_only: false,
                }).is_ok()
            })?)?;

            // === Virtual buffers ===

            // createVirtualBuffer is async - creates in current split, returns buffer_id
            // _createVirtualBufferStart(opts) -> callbackId
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_createVirtualBufferStart", Function::new(ctx.clone(), move |ctx: rquickjs::Ctx, opts: Object| -> rquickjs::Result<u64> {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
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
                let entries_arr: Vec<Object> = opts.get("entries").unwrap_or_default();
                let entries: Vec<TextPropertyEntry> = entries_arr.iter()
                    .filter_map(|obj| parse_text_property_entry(&ctx, obj))
                    .collect();

                tracing::debug!("_createVirtualBufferStart: sending CreateVirtualBufferWithContent command, request_id={}", id);
                let _ = cmd_sender.send(PluginCommand::CreateVirtualBufferWithContent {
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
            })?)?;

            // createVirtualBufferInSplit is async - returns callback_id, resolves to {bufferId, splitId}
            // _createVirtualBufferInSplitStart(opts) -> callbackId
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_createVirtualBufferInSplitStart", Function::new(ctx.clone(), move |ctx: rquickjs::Ctx, opts: Object| -> rquickjs::Result<u64> {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
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
                let entries_arr: Vec<Object> = opts.get("entries").unwrap_or_default();
                let entries: Vec<TextPropertyEntry> = entries_arr.iter()
                    .filter_map(|obj| parse_text_property_entry(&ctx, obj))
                    .collect();

                let _ = cmd_sender.send(PluginCommand::CreateVirtualBufferInSplit {
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
            })?)?;

            // setVirtualBufferContent(bufferId, entries)
            let cmd_sender = command_sender.clone();
            editor.set("setVirtualBufferContent", Function::new(ctx.clone(), move |ctx: rquickjs::Ctx, buffer_id: u32, entries_arr: Vec<Object>| -> rquickjs::Result<bool> {
                let entries: Vec<TextPropertyEntry> = entries_arr.iter()
                    .filter_map(|obj| parse_text_property_entry(&ctx, obj))
                    .collect();
                Ok(cmd_sender.send(PluginCommand::SetVirtualBufferContent {
                    buffer_id: BufferId(buffer_id as usize),
                    entries,
                }).is_ok())
            })?)?;

            // _getTextPropertiesAtCursorJson(bufferId) - reads from state snapshot, returns JSON
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("_getTextPropertiesAtCursorJson", Function::new(ctx.clone(), move |buffer_id: u32| -> String {
                get_text_properties_at_cursor_json(&snapshot, buffer_id)
            })?)?;

            // === Split operations ===
            let cmd_sender = command_sender.clone();
            editor.set("closeSplit", Function::new(ctx.clone(), move |split_id: u32| -> bool {
                cmd_sender.send(PluginCommand::CloseSplit {
                    split_id: SplitId(split_id as usize),
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("setSplitBuffer", Function::new(ctx.clone(), move |split_id: u32, buffer_id: u32| -> bool {
                cmd_sender.send(PluginCommand::SetSplitBuffer {
                    split_id: SplitId(split_id as usize),
                    buffer_id: BufferId(buffer_id as usize),
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("focusSplit", Function::new(ctx.clone(), move |split_id: u32| -> bool {
                cmd_sender.send(PluginCommand::FocusSplit {
                    split_id: SplitId(split_id as usize),
                }).is_ok()
            })?)?;

            let cmd_sender = command_sender.clone();
            editor.set("setBufferCursor", Function::new(ctx.clone(), move |buffer_id: u32, position: u32| -> bool {
                cmd_sender.send(PluginCommand::SetBufferCursor {
                    buffer_id: BufferId(buffer_id as usize),
                    position: position as usize,
                }).is_ok()
            })?)?;

            // openFileInSplit - open file in a specific split
            let cmd_sender = command_sender.clone();
            editor.set("openFileInSplit", Function::new(ctx.clone(), move |split_id: u32, path: String, line: Option<u32>, column: Option<u32>| -> bool {
                cmd_sender.send(PluginCommand::OpenFileInSplit {
                    split_id: split_id as usize,
                    path: PathBuf::from(path),
                    line: line.map(|l| l as usize),
                    column: column.map(|c| c as usize),
                }).is_ok()
            })?)?;

            // setSplitScroll - set split scroll position
            let cmd_sender = command_sender.clone();
            editor.set("setSplitScroll", Function::new(ctx.clone(), move |split_id: u32, top_byte: u32| -> bool {
                cmd_sender.send(PluginCommand::SetSplitScroll {
                    split_id: SplitId(split_id as usize),
                    top_byte: top_byte as usize,
                }).is_ok()
            })?)?;

            // setSplitRatio - set split ratio
            let cmd_sender = command_sender.clone();
            editor.set("setSplitRatio", Function::new(ctx.clone(), move |split_id: u32, ratio: f32| -> bool {
                cmd_sender.send(PluginCommand::SetSplitRatio {
                    split_id: SplitId(split_id as usize),
                    ratio,
                }).is_ok()
            })?)?;

            // distributeSplitsEvenly - distribute splits evenly
            let cmd_sender = command_sender.clone();
            editor.set("distributeSplitsEvenly", Function::new(ctx.clone(), move |split_ids: Vec<u32>| -> bool {
                let ids: Vec<SplitId> = split_ids.into_iter()
                    .map(|id| SplitId(id as usize))
                    .collect();
                cmd_sender.send(PluginCommand::DistributeSplitsEvenly {
                    split_ids: ids,
                }).is_ok()
            })?)?;

            // === Line indicators ===
            // setLineIndicator(opts: {bufferId, line, namespace, symbol, color: [r,g,b], priority?})
            let cmd_sender = command_sender.clone();
            editor.set("setLineIndicator", Function::new(ctx.clone(), move |opts: Object| -> rquickjs::Result<bool> {
                let buffer_id: u32 = opts.get("bufferId")?;
                let line: u32 = opts.get("line")?;
                let namespace: String = opts.get("namespace")?;
                let symbol: String = opts.get("symbol")?;
                let color: Vec<u8> = opts.get("color")?;
                let priority: i32 = opts.get("priority").unwrap_or(0);

                let (r, g, b) = if color.len() >= 3 { (color[0], color[1], color[2]) } else { (255, 255, 255) };

                Ok(cmd_sender.send(PluginCommand::SetLineIndicator {
                    buffer_id: BufferId(buffer_id as usize),
                    line: line as usize,
                    namespace,
                    symbol,
                    color: (r, g, b),
                    priority,
                }).is_ok())
            })?)?;

            // clearLineIndicators(bufferId, namespace)
            let cmd_sender = command_sender.clone();
            editor.set("clearLineIndicators", Function::new(ctx.clone(), move |buffer_id: u32, namespace: String| -> bool {
                cmd_sender.send(PluginCommand::ClearLineIndicators {
                    buffer_id: BufferId(buffer_id as usize),
                    namespace,
                }).is_ok()
            })?)?;

            // === Async operations - internal callback-based implementations ===

            // Process spawning
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_spawnProcessStart", Function::new(ctx.clone(), move |command: String, args: Vec<String>, cwd: Option<String>| -> u64 {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };
                let _ = cmd_sender.send(PluginCommand::SpawnProcess {
                    callback_id: id,
                    command,
                    args,
                    cwd,
                });
                id
            })?)?;

            // getBufferText - async function to read buffer text range
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_getBufferTextStart", Function::new(ctx.clone(), move |buffer_id: u32, start: u32, end: u32| -> u64 {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };
                let _ = cmd_sender.send(PluginCommand::GetBufferText {
                    buffer_id: BufferId(buffer_id as usize),
                    start: start as usize,
                    end: end as usize,
                    request_id: id,
                });
                id
            })?)?;

            // Delay/sleep
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_delayStart", Function::new(ctx.clone(), move |duration_ms: u64| -> u64 {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };
                let _ = cmd_sender.send(PluginCommand::Delay {
                    callback_id: id,
                    duration_ms,
                });
                id
            })?)?;

            // LSP request - sendLspRequest(language, method, params?) -> Promise<result>
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_sendLspRequestStart", Function::new(ctx.clone(), move |ctx: rquickjs::Ctx, language: String, method: String, params: Option<Object>| -> rquickjs::Result<u64> {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };
                // Convert params object to serde_json::Value
                let params_json: Option<serde_json::Value> = params.map(|obj| {
                    let val = obj.into_value();
                    js_to_json(&ctx, val)
                });
                let _ = cmd_sender.send(PluginCommand::SendLspRequest {
                    request_id: id,
                    language,
                    method,
                    params: params_json,
                });
                Ok(id)
            })?)?;

            // Background process - spawnBackgroundProcess(command, args, cwd?, opts?) -> Promise<{processId, exitCode}>
            // Returns immediately with processId, resolves when process exits
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_spawnBackgroundProcessStart", Function::new(ctx.clone(), move |command: String, args: Vec<String>, cwd: Option<String>| -> u64 {
                let callback_id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };
                // Use callback_id as process_id for simplicity
                let process_id = callback_id;
                let _ = cmd_sender.send(PluginCommand::SpawnBackgroundProcess {
                    process_id,
                    command,
                    args,
                    cwd,
                    callback_id,
                });
                callback_id
            })?)?;

            // Kill background process
            let cmd_sender = command_sender.clone();
            editor.set("killBackgroundProcess", Function::new(ctx.clone(), move |process_id: u64| -> bool {
                cmd_sender.send(PluginCommand::KillBackgroundProcess { process_id }).is_ok()
            })?)?;

            // getHighlights - async, get syntax highlights for a range
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_getHighlightsStart", Function::new(ctx.clone(), move |buffer_id: u32, start: u32, end: u32| -> u64 {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };
                let _ = cmd_sender.send(PluginCommand::RequestHighlights {
                    buffer_id: BufferId(buffer_id as usize),
                    range: (start as usize)..(end as usize),
                    request_id: id,
                });
                id
            })?)?;

            // killProcess - async, kill a background process and wait for completion
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_killProcessStart", Function::new(ctx.clone(), move |process_id: u64| -> u64 {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };
                // Kill the process and use callback_id to notify when done
                let _ = cmd_sender.send(PluginCommand::KillBackgroundProcess { process_id });
                // For now, resolve immediately - proper implementation would track completion
                id
            })?)?;

            // spawnProcessWait - async, wait for a spawned process to complete
            let request_id = Rc::clone(&next_request_id);
            editor.set("_spawnProcessWaitStart", Function::new(ctx.clone(), move |_process_id: u64| -> u64 {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };
                // TODO: Implement proper process wait tracking
                // For now, return callback ID that won't resolve
                id
            })?)?;

            // createVirtualBufferInExistingSplit - async, create virtual buffer in existing split
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_createVirtualBufferInExistingSplitStart", Function::new(ctx.clone(), move |ctx: rquickjs::Ctx, split_id: u32, opts: Object| -> rquickjs::Result<u64> {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
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
                let line_wrap: Option<bool> = opts.get("lineWrap").ok();

                let entries_arr: Vec<Object> = opts.get("entries").unwrap_or_default();
                let entries: Vec<TextPropertyEntry> = entries_arr.iter()
                    .filter_map(|obj| parse_text_property_entry(&ctx, obj))
                    .collect();

                let _ = cmd_sender.send(PluginCommand::CreateVirtualBufferInExistingSplit {
                    name,
                    mode,
                    read_only,
                    entries,
                    split_id: SplitId(split_id as usize),
                    show_line_numbers,
                    show_cursors,
                    editing_disabled,
                    line_wrap,
                    request_id: Some(id),
                });
                Ok(id)
            })?)?;

            // createCompositeBuffer - async, create composite buffer
            let request_id = Rc::clone(&next_request_id);
            let cmd_sender = command_sender.clone();
            editor.set("_createCompositeBufferStart", Function::new(ctx.clone(), move |options_json: String| -> u64 {
                let id = {
                    let mut id_ref = request_id.borrow_mut();
                    let id = *id_ref;
                    *id_ref += 1;
                    id
                };

                #[derive(serde::Deserialize)]
                struct CompositeOpts {
                    name: String,
                    #[serde(default)]
                    mode: String,
                    layout: crate::services::plugins::api::CompositeLayoutConfig,
                    sources: Vec<crate::services::plugins::api::CompositeSourceConfig>,
                    #[serde(default)]
                    hunks: Option<Vec<crate::services::plugins::api::CompositeHunk>>,
                }

                let opts: CompositeOpts = match serde_json::from_str(&options_json) {
                    Ok(o) => o,
                    Err(_) => return id, // Will fail to resolve
                };

                let _ = cmd_sender.send(PluginCommand::CreateCompositeBuffer {
                    name: opts.name,
                    mode: opts.mode,
                    layout: opts.layout,
                    sources: opts.sources,
                    hunks: opts.hunks,
                    request_id: Some(id),
                });
                id
            })?)?;

            // updateCompositeAlignment - sync, update composite buffer alignment
            let cmd_sender = command_sender.clone();
            editor.set("updateCompositeAlignment", Function::new(ctx.clone(), move |buffer_id: u32, hunks_json: String| -> bool {
                let hunks: Vec<crate::services::plugins::api::CompositeHunk> = match serde_json::from_str(&hunks_json) {
                    Ok(h) => h,
                    Err(_) => return false,
                };
                cmd_sender.send(PluginCommand::UpdateCompositeAlignment {
                    buffer_id: BufferId(buffer_id as usize),
                    hunks,
                }).is_ok()
            })?)?;

            // closeCompositeBuffer - sync, close composite buffer
            let cmd_sender = command_sender.clone();
            editor.set("closeCompositeBuffer", Function::new(ctx.clone(), move |buffer_id: u32| -> bool {
                cmd_sender.send(PluginCommand::CloseCompositeBuffer {
                    buffer_id: BufferId(buffer_id as usize),
                }).is_ok()
            })?)?;

            // === Refresh ===
            let cmd_sender = command_sender.clone();
            editor.set("refreshLines", Function::new(ctx.clone(), move |buffer_id: u32| -> bool {
                cmd_sender.send(PluginCommand::RefreshLines {
                    buffer_id: BufferId(buffer_id as usize),
                }).is_ok()
            })?)?;

            // === i18n ===
            editor.set("getCurrentLocale", Function::new(ctx.clone(), || -> String {
                crate::i18n::current_locale()
            })?)?;

            // === Editor mode ===
            let cmd_sender = command_sender.clone();
            editor.set("setEditorMode", Function::new(ctx.clone(), move |mode: Option<String>| -> bool {
                cmd_sender.send(PluginCommand::SetEditorMode { mode }).is_ok()
            })?)?;

            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getEditorMode", Function::new(ctx.clone(), move || -> Option<String> {
                snapshot.read().ok().and_then(|s| s.editor_mode.clone())
            })?)?;

            // === Display settings ===
            // setLineNumbers - enable/disable line numbers for a buffer
            let cmd_sender = command_sender.clone();
            editor.set("setLineNumbers", Function::new(ctx.clone(), move |buffer_id: u32, enabled: bool| -> bool {
                cmd_sender.send(PluginCommand::SetLineNumbers {
                    buffer_id: BufferId(buffer_id as usize),
                    enabled,
                }).is_ok()
            })?)?;

            // === File Explorer Decorations ===
            // setFileExplorerDecorations - set decorations for file explorer
            let cmd_sender = command_sender.clone();
            editor.set("setFileExplorerDecorations", Function::new(ctx.clone(), move |namespace: String, decorations_json: String| -> bool {
                #[derive(serde::Deserialize)]
                struct Decoration {
                    path: String,
                    symbol: String,
                    color: (u8, u8, u8),
                    #[serde(default)]
                    priority: i32,
                }
                let decorations: Vec<Decoration> = match serde_json::from_str(&decorations_json) {
                    Ok(d) => d,
                    Err(_) => return false,
                };
                let decs: Vec<crate::view::file_tree::FileExplorerDecoration> = decorations.into_iter()
                    .map(|d| crate::view::file_tree::FileExplorerDecoration {
                        path: PathBuf::from(d.path),
                        symbol: d.symbol,
                        color: d.color,
                        priority: d.priority,
                    })
                    .collect();
                cmd_sender.send(PluginCommand::SetFileExplorerDecorations {
                    namespace,
                    decorations: decs,
                }).is_ok()
            })?)?;

            // clearFileExplorerDecorations - clear decorations for a namespace
            let cmd_sender = command_sender.clone();
            editor.set("clearFileExplorerDecorations", Function::new(ctx.clone(), move |namespace: String| -> bool {
                cmd_sender.send(PluginCommand::ClearFileExplorerDecorations { namespace }).is_ok()
            })?)?;

            // === Scroll Sync ===
            // createScrollSyncGroup - create a scroll sync group
            let cmd_sender = command_sender.clone();
            editor.set("createScrollSyncGroup", Function::new(ctx.clone(), move |group_id: u32, left_split: u32, right_split: u32| -> bool {
                cmd_sender.send(PluginCommand::CreateScrollSyncGroup {
                    group_id,
                    left_split: SplitId(left_split as usize),
                    right_split: SplitId(right_split as usize),
                }).is_ok()
            })?)?;

            // setScrollSyncAnchors - set scroll sync anchors
            let cmd_sender = command_sender.clone();
            editor.set("setScrollSyncAnchors", Function::new(ctx.clone(), move |group_id: u32, anchors_json: String| -> bool {
                #[derive(serde::Deserialize)]
                struct Anchor {
                    left: u32,
                    right: u32,
                }
                let anchors: Vec<Anchor> = match serde_json::from_str(&anchors_json) {
                    Ok(a) => a,
                    Err(_) => return false,
                };
                let anchor_pairs: Vec<(usize, usize)> = anchors.into_iter()
                    .map(|a| (a.left as usize, a.right as usize))
                    .collect();
                cmd_sender.send(PluginCommand::SetScrollSyncAnchors {
                    group_id,
                    anchors: anchor_pairs,
                }).is_ok()
            })?)?;

            // removeScrollSyncGroup - remove a scroll sync group
            let cmd_sender = command_sender.clone();
            editor.set("removeScrollSyncGroup", Function::new(ctx.clone(), move |group_id: u32| -> bool {
                cmd_sender.send(PluginCommand::RemoveScrollSyncGroup {
                    group_id,
                }).is_ok()
            })?)?;

            // === Action Popup ===
            // showActionPopup - show an action popup
            let cmd_sender = command_sender.clone();
            editor.set("showActionPopup", Function::new(ctx.clone(), move |options_json: String| -> bool {
                #[derive(serde::Deserialize)]
                struct PopupOptions {
                    popup_id: String,
                    title: String,
                    message: String,
                    actions: Vec<PopupAction>,
                }
                #[derive(serde::Deserialize)]
                struct PopupAction {
                    id: String,
                    label: String,
                }
                let opts: PopupOptions = match serde_json::from_str(&options_json) {
                    Ok(o) => o,
                    Err(_) => return false,
                };
                let actions: Vec<crate::services::plugins::api::ActionPopupAction> = opts.actions.into_iter()
                    .map(|a| crate::services::plugins::api::ActionPopupAction {
                        id: a.id,
                        label: a.label,
                    })
                    .collect();
                cmd_sender.send(PluginCommand::ShowActionPopup {
                    popup_id: opts.popup_id,
                    title: opts.title,
                    message: opts.message,
                    actions,
                }).is_ok()
            })?)?;

            // === LSP ===
            // disableLspForLanguage - disable LSP for a language
            let cmd_sender = command_sender.clone();
            editor.set("disableLspForLanguage", Function::new(ctx.clone(), move |language: String| -> bool {
                cmd_sender.send(PluginCommand::DisableLspForLanguage { language }).is_ok()
            })?)?;

            // === View Transform ===
            // submitViewTransform - submit a view transform
            let cmd_sender = command_sender.clone();
            editor.set("submitViewTransform", Function::new(ctx.clone(), move |buffer_id: u32, split_id: Option<u32>, payload_json: String| -> bool {
                let payload: crate::services::plugins::api::ViewTransformPayload = match serde_json::from_str(&payload_json) {
                    Ok(p) => p,
                    Err(_) => return false,
                };
                cmd_sender.send(PluginCommand::SubmitViewTransform {
                    buffer_id: BufferId(buffer_id as usize),
                    split_id: split_id.map(|s| SplitId(s as usize)),
                    payload,
                }).is_ok()
            })?)?;

            // clearViewTransform - clear view transform
            let cmd_sender = command_sender.clone();
            editor.set("clearViewTransform", Function::new(ctx.clone(), move |buffer_id: u32, split_id: Option<u32>| -> bool {
                cmd_sender.send(PluginCommand::ClearViewTransform {
                    buffer_id: BufferId(buffer_id as usize),
                    split_id: split_id.map(|s| SplitId(s as usize)),
                }).is_ok()
            })?)?;

            // === Process status ===
            // isProcessRunning - check if a background process is running
            // Note: This requires tracking in the backend; returning false as placeholder
            editor.set("isProcessRunning", Function::new(ctx.clone(), |_process_id: u64| -> bool {
                // TODO: Implement proper process tracking
                // For now, always return false as we don't track running processes
                false
            })?)?;

            // Store editor as internal _editorCore (not meant for direct plugin access)
            globals.set("_editorCore", editor)?;

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

                // Apply wrappers to async functions on _editorCore
                _editorCore.spawnProcess = _wrapAsyncThenable(_editorCore._spawnProcessStart, "spawnProcess");
                _editorCore.delay = _wrapAsync(_editorCore._delayStart, "delay");
                _editorCore.createVirtualBuffer = _wrapAsync(_editorCore._createVirtualBufferStart, "createVirtualBuffer");
                _editorCore.createVirtualBufferInSplit = _wrapAsyncThenable(_editorCore._createVirtualBufferInSplitStart, "createVirtualBufferInSplit");
                _editorCore.sendLspRequest = _wrapAsync(_editorCore._sendLspRequestStart, "sendLspRequest");
                _editorCore.spawnBackgroundProcess = _wrapAsyncThenable(_editorCore._spawnBackgroundProcessStart, "spawnBackgroundProcess");
                _editorCore.getBufferText = _wrapAsync(_editorCore._getBufferTextStart, "getBufferText");
                _editorCore.getHighlights = _wrapAsync(_editorCore._getHighlightsStart, "getHighlights");
                _editorCore.killProcess = _wrapAsync(_editorCore._killProcessStart, "killProcess");
                _editorCore.spawnProcessWait = _wrapAsync(_editorCore._spawnProcessWaitStart, "spawnProcessWait");
                _editorCore.createVirtualBufferInExistingSplit = _wrapAsync(_editorCore._createVirtualBufferInExistingSplitStart, "createVirtualBufferInExistingSplit");
                _editorCore.createCompositeBuffer = _wrapAsync(_editorCore._createCompositeBufferStart, "createCompositeBuffer");

                // Wrapper for getTextPropertiesAtCursor - parses JSON from Rust
                _editorCore.getTextPropertiesAtCursor = function(bufferId) {
                    return JSON.parse(_editorCore._getTextPropertiesAtCursorJson(bufferId));
                };

                // Wrapper for addOverlay - accepts positional args, converts to JSON
                _editorCore.addOverlay = function(bufferId, namespace, start, end, r, g, b, underline, bold, italic, bg_r, bg_g, bg_b, extend_to_line_end) {
                    return _editorCore._addOverlayInternal(JSON.stringify({
                        buffer_id: bufferId,
                        namespace: namespace,
                        start: start,
                        end: end,
                        r: r, g: g, b: b,
                        underline: underline,
                        bold: bold,
                        italic: italic,
                        bg_r: bg_r, bg_g: bg_g, bg_b: bg_b,
                        extend_to_line_end: extend_to_line_end
                    }));
                };

                // Wrapper for listBuffers - parses JSON from Rust
                _editorCore.listBuffers = function() {
                    return JSON.parse(_editorCore._listBuffersJson());
                };

                // Wrapper for readDir - parses JSON from Rust
                _editorCore.readDir = function(path) {
                    return JSON.parse(_editorCore._readDirJson(path));
                };

                // Wrapper for deleteTheme - wraps sync function in Promise
                _editorCore.deleteTheme = function(name) {
                    return new Promise(function(resolve, reject) {
                        const success = _editorCore._deleteThemeSync(name);
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
    pub async fn load_module_with_source(&mut self, path: &str, _plugin_source: &str) -> Result<()> {
        let path_buf = PathBuf::from(path);
        let source = std::fs::read_to_string(&path_buf)
            .map_err(|e| anyhow!("Failed to read plugin {}: {}", path, e))?;

        let filename = path_buf.file_name()
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
                        path, e
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

        tracing::debug!("execute_js: starting for plugin '{}' from '{}'", plugin_name, source_name);

        // Define getEditor() for this plugin - returns an editor object with plugin name in closures
        let escaped_name = plugin_name.replace('\\', "\\\\").replace('"', "\\\"");
        let define_get_editor = format!(r#"
            globalThis.getEditor = function() {{
                const core = globalThis._editorCore;
                const pluginName = "{}";
                return {{
                    // All core methods
                    ...core,

                    // Plugin name for reference
                    _pluginName: pluginName,

                    // Plugin-prefixed logging
                    error(msg) {{ core.error("[" + pluginName + "] " + msg); }},
                    warn(msg) {{ core.warn("[" + pluginName + "] " + msg); }},
                    info(msg) {{ core.info("[" + pluginName + "] " + msg); }},
                    debug(msg) {{ core.debug("[" + pluginName + "] " + msg); }},

                    // Plugin-specific translation
                    t(key, args) {{ return core._pluginTranslate(pluginName, key, args || {{}}); }},

                    // Command registration with plugin name for i18n
                    registerCommand(name, description, handler, context) {{
                        return core._registerCommandInternal(pluginName, name, description, handler, context);
                    }},

                    // For compatibility
                    getL10n() {{ return {{ t: (k, a) => this.t(k, a) }}; }},
                }};
            }};
        "#, escaped_name);

        // Wrap plugin code in IIFE for scope isolation
        let wrapped = format!(
            "(function() {{\n{}\n}})();",
            code
        );

        self.context.with(|ctx| {
            // Define getEditor for this plugin
            ctx.eval::<(), _>(define_get_editor.as_bytes())
                .map_err(|e| format_js_error(&ctx, e, source_name))?;

            tracing::debug!("execute_js: getEditor defined, now executing plugin code for '{}'", plugin_name);

            // Execute the plugin code
            let result = ctx.eval::<(), _>(wrapped.as_bytes())
                .map_err(|e| format_js_error(&ctx, e, source_name));

            tracing::debug!("execute_js: plugin code execution finished for '{}', result: {:?}", plugin_name, result.is_ok());

            result
        })
    }

    /// Emit an event to all registered handlers
    pub async fn emit(&mut self, event_name: &str, event_data: &str) -> Result<bool> {
        tracing::debug!("emit: event '{}' with {} bytes of data", event_name, event_data.len());

        // Track execution state for signal handler debugging
        crate::services::signal_handler::set_js_execution_state(
            format!("hook '{}'", event_name)
        );

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
        crate::services::signal_handler::set_js_execution_state(
            format!("action '{}' (fn: {})", action_name, function_name)
        );

        tracing::info!("start_action: BEGIN '{}' -> function '{}'", action_name, function_name);

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

        tracing::debug!("execute_action: '{}' -> function '{}'", action_name, function_name);

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
                                run_pending_jobs_checked(&ctx, &format!("execute_action {} promise", action_name));
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
        let _ = self.command_sender.send(PluginCommand::SetStatus { message });
    }

    /// Resolve a pending async callback with a result (called from Rust when async op completes)
    pub fn resolve_callback(&mut self, callback_id: u64, result_json: &str) {
        tracing::debug!("resolve_callback: starting for callback_id={}", callback_id);
        let code = format!(
            "globalThis._resolveCallback({}, {});",
            callback_id,
            result_json
        );
        self.context.with(|ctx| {
            tracing::debug!("resolve_callback: evaluating JS code: {}", code);
            if let Err(e) = ctx.eval::<(), _>(code.as_bytes()) {
                log_js_error(&ctx, e, &format!("resolving callback {}", callback_id));
            }
            // IMPORTANT: Run pending jobs to process Promise continuations
            let job_count = run_pending_jobs_checked(&ctx, &format!("resolve_callback {}", callback_id));
            tracing::info!("resolve_callback: executed {} pending jobs for callback_id={}", job_count, callback_id);
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
        backend.event_handlers.borrow_mut()
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

        backend.execute_js(r#"
            const editor = getEditor();
            editor.setStatus("Hello from test");
        "#, "test.js").unwrap();

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

        backend.execute_js(r#"
            const editor = getEditor();
            globalThis.myTestHandler = function() { };
            editor.registerCommand("Test Command", "A test command", "myTestHandler", null);
        "#, "test_plugin.js").unwrap();

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

        backend.execute_js(r#"
            const editor = getEditor();
            editor.defineMode("test-mode", null, [
                ["a", "action_a"],
                ["b", "action_b"]
            ]);
        "#, "test.js").unwrap();

        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::DefineMode { name, parent, bindings, read_only } => {
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

        backend.execute_js(r#"
            const editor = getEditor();
            editor.setEditorMode("vi-normal");
        "#, "test.js").unwrap();

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

        backend.execute_js(r#"
            const editor = getEditor();
            editor.setEditorMode(null);
        "#, "test.js").unwrap();

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

        backend.execute_js(r#"
            const editor = getEditor();
            editor.insertAtCursor("Hello, World!");
        "#, "test.js").unwrap();

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

        backend.execute_js(r#"
            const editor = getEditor();
            editor.setContext("myContext", true);
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            globalThis.my_sync_action = function() {
                editor.setStatus("sync action executed");
            };
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            globalThis.my_async_action = async function() {
                await Promise.resolve();
                editor.setStatus("async action executed");
            };
        "#, "test.js").unwrap();

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
            "actual_handler_function".to_string()
        );

        backend.execute_js(r#"
            const editor = getEditor();
            globalThis.actual_handler_function = function() {
                editor.setStatus("handler executed");
            };
        "#, "test.js").unwrap();

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

        backend.execute_js(r#"
            const editor = getEditor();
            globalThis.myEventHandler = function() { };
            editor.on("bufferSave", "myEventHandler");
        "#, "test.js").unwrap();

        assert!(backend.has_handlers("bufferSave"));
    }

    #[test]
    fn test_api_off_event_unregistration() {
        let (mut backend, _rx) = create_test_backend();

        backend.execute_js(r#"
            const editor = getEditor();
            globalThis.myEventHandler = function() { };
            editor.on("bufferSave", "myEventHandler");
            editor.off("bufferSave", "myEventHandler");
        "#, "test.js").unwrap();

        // Handler should be removed
        assert!(!backend.has_handlers("bufferSave"));
    }

    #[tokio::test]
    async fn test_emit_event() {
        let (mut backend, rx) = create_test_backend();

        backend.execute_js(r#"
            const editor = getEditor();
            globalThis.onSaveHandler = function(data) {
                editor.setStatus("saved: " + JSON.stringify(data));
            };
            editor.on("bufferSave", "onSaveHandler");
        "#, "test.js").unwrap();

        // Drain setup commands
        while rx.try_recv().is_ok() {}

        // Emit the event
        backend.emit("bufferSave", r#"{"path": "/test.txt"}"#).await.unwrap();

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

        backend.execute_js(r#"
            const editor = getEditor();
            editor.copyToClipboard("clipboard text");
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            editor.openFile("/path/to/file.txt", null, null);
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            editor.deleteRange(0, 10, 20);
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            editor.insertText(0, 5, "inserted");
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            editor.setBufferCursor(0, 100);
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            const pos = editor.getCursorPosition();
            globalThis._testResult = pos;
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            globalThis._dirname = editor.pathDirname("/foo/bar/baz.txt");
            globalThis._basename = editor.pathBasename("/foo/bar/baz.txt");
            globalThis._extname = editor.pathExtname("/foo/bar/baz.txt");
            globalThis._isAbsolute = editor.pathIsAbsolute("/foo/bar");
            globalThis._isRelative = editor.pathIsAbsolute("foo/bar");
            globalThis._joined = editor.pathJoin(["/foo", "bar", "baz"]);
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            // Store the promise for later
            globalThis._textPromise = editor.getBufferText(0, 10, 20);
        "#, "test.js").unwrap();

        // Verify the GetBufferText command was sent
        let cmd = rx.try_recv().unwrap();
        match cmd {
            PluginCommand::GetBufferText { buffer_id, start, end, request_id } => {
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
        backend.execute_js(r#"
            const editor = getEditor();
            globalThis._resolvedText = null;
            editor.getBufferText(0, 0, 100).then(text => {
                globalThis._resolvedText = text;
            });
        "#, "test.js").unwrap();

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
        backend.execute_js(r#"
            const editor = getEditor();
            globalThis._translated = editor.t("test.key");
        "#, "test.js").unwrap();

        backend.context.with(|ctx| {
            let global = ctx.globals();
            // Without actual translations, it returns the key
            let result: String = global.get("_translated").unwrap();
            assert_eq!(result, "test.key");
        });
    }
}
