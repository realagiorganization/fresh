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
use crate::services::plugins::transpile::{bundle_module, has_es_imports, transpile_typescript};
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
fn log_js_error(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error, context: &str) {
    let error = format_js_error(ctx, err, context);
    tracing::error!("{}", error);
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
                tracing::debug!("Plugin: {}", msg);
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
            // registerCommand(name, description, handler_name, context)
            let cmd_sender = command_sender.clone();
            let actions = Rc::clone(&registered_actions);
            editor.set("registerCommand", Function::new(ctx.clone(), move |name: String, description: String, handler_name: String, context: Option<String>| -> bool {
                tracing::debug!("registerCommand: name='{}', handler='{}', context={:?}", name, handler_name, context);

                // Store action handler mapping (handler_name -> handler_name for direct lookup)
                actions.borrow_mut().insert(handler_name.clone(), handler_name.clone());

                // Register with editor - action uses handler_name so execute_action can find it
                let command = Command {
                    name: name.clone(),
                    description,
                    action: Action::PluginAction(handler_name.clone()),
                    contexts: vec![],
                    custom_contexts: context.into_iter().collect(),
                    source: CommandSource::Plugin(handler_name),
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

            // === Overlays ===
            // addOverlay(opts: {bufferId, namespace?, start, end, color: [r,g,b], bgColor?: [r,g,b], underline?, bold?, italic?, extendToLineEnd?})
            let cmd_sender = command_sender.clone();
            editor.set("addOverlay", Function::new(ctx.clone(), move |opts: Object| -> rquickjs::Result<bool> {
                let buffer_id: u32 = opts.get("bufferId")?;
                let namespace: Option<String> = opts.get("namespace").ok();
                let start: u32 = opts.get("start")?;
                let end: u32 = opts.get("end")?;

                // color is [r, g, b] array
                let color: Vec<u8> = opts.get("color")?;
                let (r, g, b) = if color.len() >= 3 { (color[0], color[1], color[2]) } else { (255, 255, 255) };

                // bgColor is optional [r, g, b] array
                let bg_color: Option<(u8, u8, u8)> = opts.get::<_, Vec<u8>>("bgColor").ok()
                    .filter(|c| c.len() >= 3)
                    .map(|c| (c[0], c[1], c[2]));

                let underline: bool = opts.get("underline").unwrap_or(false);
                let bold: bool = opts.get("bold").unwrap_or(false);
                let italic: bool = opts.get("italic").unwrap_or(false);
                let extend_to_line_end: bool = opts.get("extendToLineEnd").unwrap_or(false);

                Ok(cmd_sender.send(PluginCommand::AddOverlay {
                    buffer_id: BufferId(buffer_id as usize),
                    namespace: namespace.map(OverlayNamespace::from_string),
                    range: (start as usize)..(end as usize),
                    color: (r, g, b),
                    bg_color,
                    underline,
                    bold,
                    italic,
                    extend_to_line_end,
                }).is_ok())
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

            // getTextPropertiesAtCursor(bufferId) - reads from state snapshot
            let snapshot = Arc::clone(&state_snapshot);
            editor.set("getTextPropertiesAtCursor", Function::new(ctx.clone(), move |buffer_id: u32| -> Option<String> {
                let snap = snapshot.read().ok()?;
                let buffer_id = BufferId(buffer_id as usize);
                let cursor_pos = snap.buffer_cursor_positions.get(&buffer_id).copied()
                    .or_else(|| {
                        if snap.active_buffer_id == buffer_id {
                            snap.primary_cursor.as_ref().map(|c| c.position)
                        } else {
                            None
                        }
                    })?;

                let properties = snap.buffer_text_properties.get(&buffer_id)?;
                // Find property at cursor position
                for prop in properties {
                    if prop.start <= cursor_pos && cursor_pos < prop.end {
                        return serde_json::to_string(&prop.properties).ok();
                    }
                }
                None
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

            // Store editor as internal _editorCore (not meant for direct plugin access)
            globals.set("_editorCore", editor)?;

            // Provide console.log for debugging
            let console = Object::new(ctx.clone())?;
            console.set("log", Function::new(ctx.clone(), |args: Vec<String>| {
                tracing::info!("console.log: {}", args.join(" "));
            })?)?;
            console.set("warn", Function::new(ctx.clone(), |args: Vec<String>| {
                tracing::warn!("console.warn: {}", args.join(" "));
            })?)?;
            console.set("error", Function::new(ctx.clone(), |args: Vec<String>| {
                tracing::error!("console.error: {}", args.join(" "));
            })?)?;
            globals.set("console", console)?;

            // Bootstrap: Promise infrastructure (getEditor is defined per-plugin in execute_js)
            ctx.eval::<(), _>(r#"
                // Pending promise callbacks: callbackId -> { resolve, reject }
                globalThis._pendingCallbacks = new Map();

                // Resolve a pending callback (called from Rust)
                globalThis._resolveCallback = function(callbackId, result) {
                    const cb = globalThis._pendingCallbacks.get(callbackId);
                    if (cb) {
                        globalThis._pendingCallbacks.delete(callbackId);
                        cb.resolve(result);
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
                // Usage: editor.foo = _wrapAsync(editor._fooStart);
                globalThis._wrapAsync = function(startFn) {
                    return function(...args) {
                        const callbackId = startFn.apply(this, args);
                        return new Promise((resolve, reject) => {
                            globalThis._pendingCallbacks.set(callbackId, { resolve, reject });
                        });
                    };
                };

                // Async wrapper that returns a thenable object (for APIs like spawnProcess)
                // The returned object has .result promise and is itself thenable
                globalThis._wrapAsyncThenable = function(startFn) {
                    return function(...args) {
                        const callbackId = startFn.apply(this, args);
                        const resultPromise = new Promise((resolve, reject) => {
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
                _editorCore.spawnProcess = _wrapAsyncThenable(_editorCore._spawnProcessStart);
                _editorCore.delay = _wrapAsync(_editorCore._delayStart);
                _editorCore.createVirtualBufferInSplit = _wrapAsyncThenable(_editorCore._createVirtualBufferInSplitStart);
                _editorCore.sendLspRequest = _wrapAsync(_editorCore._sendLspRequestStart);
                _editorCore.spawnBackgroundProcess = _wrapAsyncThenable(_editorCore._spawnBackgroundProcessStart);
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

        // Check for ES imports
        if has_es_imports(&source) {
            // Try to bundle
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
        } else {
            // Transpile and execute
            let filename = path_buf.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("plugin.ts");

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
        let handlers = self.event_handlers.borrow().get(event_name).cloned();

        if let Some(handler_names) = handlers {
            if handler_names.is_empty() {
                return Ok(true);
            }

            for handler_name in &handler_names {
                let code = format!(
                    r#"
                    (function() {{
                        try {{
                            const data = JSON.parse({});
                            if (typeof globalThis.{} === 'function') {{
                                globalThis.{}(data);
                            }}
                        }} catch (e) {{
                            console.error('Handler {} error:', e);
                        }}
                    }})();
                    "#,
                    serde_json::to_string(event_data)?,
                    handler_name,
                    handler_name,
                    handler_name
                );

                self.context.with(|ctx| {
                    if let Err(e) = ctx.eval::<(), _>(code.as_bytes()) {
                        log_js_error(&ctx, e, &format!("handler {}", handler_name));
                    }
                });
            }
        }

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

    /// Execute a registered action by name
    pub async fn execute_action(&mut self, action_name: &str) -> Result<()> {
        // First check if there's a registered command mapping
        let handler_name = self.registered_actions.borrow().get(action_name).cloned();
        // Use the registered handler name if found, otherwise try the action name directly
        // (defineMode bindings use global function names directly)
        let function_name = handler_name.unwrap_or_else(|| action_name.to_string());

        tracing::debug!("execute_action: '{}' -> function '{}'", action_name, function_name);

        let code = format!(
            r#"
            (function() {{
                try {{
                    if (typeof globalThis.{} === 'function') {{
                        globalThis.{}();
                    }} else {{
                        console.error('Action {} is not defined as a global function');
                    }}
                }} catch (e) {{
                    console.error('Action {} error:', e);
                }}
            }})();
            "#,
            function_name, function_name, action_name, action_name
        );

        self.context.with(|ctx| {
            if let Err(e) = ctx.eval::<(), _>(code.as_bytes()) {
                log_js_error(&ctx, e, &format!("action {}", action_name));
            }
        });

        Ok(())
    }

    /// Poll the event loop once (QuickJS is synchronous, so this is a no-op)
    pub fn poll_event_loop_once(&mut self) -> bool {
        // QuickJS doesn't have an async event loop like V8
        // Return false to indicate no pending work
        false
    }

    /// Send a status message to the editor
    pub fn send_status(&self, message: String) {
        let _ = self.command_sender.send(PluginCommand::SetStatus { message });
    }

    /// Resolve a pending async callback with a result (called from Rust when async op completes)
    pub fn resolve_callback(&mut self, callback_id: u64, result_json: &str) {
        let code = format!(
            "globalThis._resolveCallback({}, {});",
            callback_id,
            result_json
        );
        self.context.with(|ctx| {
            if let Err(e) = ctx.eval::<(), _>(code.as_bytes()) {
                log_js_error(&ctx, e, &format!("resolving callback {}", callback_id));
            }
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
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
