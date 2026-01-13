# QuickJS Migration Plan

Replace deno_core (V8) + deno_ast with QuickJS + oxc.

## Status: API Implementation Complete ✓

The migration from deno to QuickJS + oxc is complete with full API implementation. The plugin system now uses:
- **QuickJS** (rquickjs 0.9) for JavaScript runtime
- **oxc** (0.108) for TypeScript transpilation
- **Promise wrapper pattern** for async API compatibility
- **Native JS object parameters** instead of JSON strings for complex types

---

## Completed Work

### Phase 1: Dependencies ✓
- Removed: `deno_core`, `deno_ast`, `deno_error`
- Added: `rquickjs`, `oxc_allocator`, `oxc_parser`, `oxc_transformer`, `oxc_codegen`, `oxc_span`, `oxc_semantic`

### Phase 2: Transpilation ✓
- Created `src/services/plugins/transpile.rs`
- TypeScript → JavaScript via oxc (parse → semantic → transform → codegen)
- ES module bundling for plugins with local imports

### Phase 3: QuickJS Backend ✓
- Created `src/services/plugins/backend/quickjs_backend.rs`
- Implemented ~40 editor API methods
- Promise infrastructure with callback-based async pattern

### Phase 4: Async Operations ✓
- `_wrapAsync()` / `_wrapAsyncThenable()` decorators in JS bootstrap
- `editor.spawnProcess()` - returns thenable, executes process, resolves with {stdout, stderr, exitCode}
- `editor.delay(ms)` - returns promise, sleeps for duration
- Callback resolution flow: JS → Rust command → App handles → resolve_callback → JS promise resolves

### Phase 5: Cleanup ✓
- Deleted `src/services/plugins/runtime.rs` (265KB)
- Deleted `src/v8_init.rs`
- Removed V8 initialization from test harness
- Updated all imports and module declarations

### Phase 6: API Methods ✓
- Implemented all previously stubbed methods with native JS object parameters:
  - `addOverlay(opts)` - accepts `{bufferId, namespace?, start, end, color: [r,g,b], bgColor?, underline?, bold?, italic?, extendToLineEnd?}`
  - `setPromptSuggestions(suggestions)` - accepts array of `{text, description?, value?, disabled?, keybinding?}`
  - `defineMode(opts)` - accepts `{name, parent?, bindings: [{key, command}], readOnly?}`
  - `createVirtualBufferInSplit(opts)` - async, returns `{bufferId, splitId}`
  - `setVirtualBufferContent(bufferId, entries)` - entries are `{text, properties?}`
  - `getTextPropertiesAtCursor(bufferId)` - reads from state snapshot
  - `setLineIndicator(opts)` - accepts `{bufferId, line, namespace, symbol, color: [r,g,b], priority?}`
  - `clearLineIndicators(bufferId, namespace)`
- Added helper functions for JS ↔ JSON conversion (`js_to_json`, `parse_text_property_entry`)

---

## Architecture: Async Pattern

```
┌─────────────────────────────────────────────────────────────────┐
│ Plugin JS Code                                                   │
│   const result = await editor.spawnProcess("git", ["status"]);  │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ JS Bootstrap (_wrapAsyncThenable)                               │
│   1. Call editor._spawnProcessStart() → returns callbackId      │
│   2. Create Promise, store {resolve, reject} in _pendingCallbacks│
│   3. Return thenable object                                     │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ Rust QuickJsBackend (_spawnProcessStart)                        │
│   1. Generate unique callbackId                                 │
│   2. Send PluginCommand::SpawnProcess to app                    │
│   3. Return callbackId to JS                                    │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ App (handle_plugin_command)                                     │
│   1. Execute std::process::Command                              │
│   2. Call plugin_manager.resolve_callback(id, result_json)      │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ PluginThreadHandle.resolve_callback()                           │
│   Send PluginRequest::ResolveCallback to plugin thread          │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ Plugin Thread (handle_plugin_request)                           │
│   Call runtime.borrow_mut().resolve_callback(id, json)          │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ QuickJsBackend.resolve_callback()                               │
│   Execute: globalThis._resolveCallback(callbackId, result)      │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│ JS Bootstrap (_resolveCallback)                                 │
│   1. Find pending callback by ID                                │
│   2. Call resolve(result)                                       │
│   3. Promise resolves, plugin code continues                    │
└─────────────────────────────────────────────────────────────────┘
```

---

## Remaining Work

### High Priority (Plugin Compatibility)

1. **Test with actual plugins** - Load bundled plugins and verify they work
2. **LSP integration** - `sendLspRequest()` needs async implementation
3. **Background process spawning** - `spawnBackgroundProcess()` stub

### Low Priority (Nice to Have)

4. **Non-blocking delay** - Current delay blocks UI thread
5. **Non-blocking process spawn** - Current spawn blocks UI thread
6. **Process streaming** - Stream stdout/stderr instead of waiting for completion

### TypeScript Definition Generation

For better developer experience, consider auto-generating `.d.ts` files:
- **Option 1**: `ts-rs` crate - derive macros for Rust types
- **Option 2**: Spec-based generation - define API in YAML, generate both Rust and TypeScript
- **Option 3**: Custom proc-macros with metadata extraction

---

## File Summary

| File | Status |
|------|--------|
| `Cargo.toml` | ✓ Updated |
| `src/services/plugins/mod.rs` | ✓ Updated |
| `src/services/plugins/transpile.rs` | ✓ Created |
| `src/services/plugins/backend/mod.rs` | ✓ Created |
| `src/services/plugins/backend/quickjs_backend.rs` | ✓ Created (~900 lines) |
| `src/services/plugins/thread.rs` | ✓ Updated |
| `src/services/plugins/manager.rs` | ✓ Updated |
| `src/services/plugins/api.rs` | ✓ Updated |
| `src/app/mod.rs` | ✓ Updated (handlers use resolve_callback) |
| `src/lib.rs` | ✓ Updated (removed v8_init) |
| `tests/common/harness.rs` | ✓ Updated (removed V8 init) |
| `src/services/plugins/runtime.rs` | ✓ Deleted |
| `src/v8_init.rs` | ✓ Deleted |

---

## Adding New Async APIs

To add a new async API (e.g., `editor.foo()`):

**1. Rust side (`quickjs_backend.rs`):**
```rust
let request_id = Rc::clone(&next_request_id);
let cmd_sender = command_sender.clone();
editor.set("_fooStart", Function::new(ctx.clone(), move |arg: String| -> u64 {
    let id = { /* increment and get id */ };
    let _ = cmd_sender.send(PluginCommand::Foo { callback_id: id, arg });
    id
})?)?;
```

**2. JS bootstrap:**
```javascript
editor.foo = _wrapAsync(editor._fooStart);
// or for thenable:
editor.foo = _wrapAsyncThenable(editor._fooStart);
```

**3. Add command variant (`api.rs`):**
```rust
Foo { callback_id: u64, arg: String },
```

**4. Handle in app (`app/mod.rs`):**
```rust
PluginCommand::Foo { callback_id, arg } => {
    let result = do_foo(arg);
    self.plugin_manager.resolve_callback(callback_id, serde_json::to_string(&result).unwrap());
}
```
