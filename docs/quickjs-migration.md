# QuickJS Backend Migration

## Overview

This document tracks the migration from deno_core (V8) to QuickJS for the Fresh editor's JavaScript plugin runtime.

## Goals

1. **Reduce dependencies** - From ~315 crates with deno_core/V8 to ~183 with QuickJS
2. **Simplify build** - No more V8 snapshot generation, faster compilation
3. **Lighter runtime** - QuickJS is ~700KB vs V8's multi-MB footprint
4. **Single backend** - No feature flags, just QuickJS + oxc

## Current Status

| Component | Status | Notes |
|-----------|--------|-------|
| QuickJS runtime | **Working** | ES2023 support via rquickjs 0.9 |
| TypeScript transpilation | **Working** | Via oxc 0.102 (parse + transform + codegen) |
| Plugin loading | **Working** | 18/19 plugins load successfully |
| ES module imports | **Skipped** | Plugins with `import` are skipped with warning |
| Core editor.* API | **Working** | ~30 methods fully implemented |
| Advanced API | **Stubs** | ~15 methods log warnings but don't crash |

### Plugin Compatibility

- **18 plugins load and run** - Full functionality for plugins without ES imports
- **1 plugin skipped** - `clangd_support.ts` (uses ES module imports from `./lib/`)

## Technology Stack

- **QuickJS**: Embedded JavaScript engine supporting ES2023 via `rquickjs` crate (v0.9)
- **oxc**: Fast TypeScript transpilation via `oxc_transformer` (v0.102)
- **oxc_semantic**: Scoping analysis for the transformer

## Next Steps

### Phase 1: ES Module Support (enables clangd_support.ts)

To support plugins with ES module imports, we need module bundling:

1. **Add `oxc_resolver`** - Resolve import paths to actual files
2. **Implement simple bundler** - Concatenate imported modules before transpilation
3. **Handle circular imports** - Track visited modules to avoid infinite loops

```rust
// Pseudo-code for module bundling
fn bundle_module(path: &str, visited: &mut HashSet<String>) -> Result<String> {
    if visited.contains(path) { return Ok(String::new()); }
    visited.insert(path.to_string());

    let source = fs::read_to_string(path)?;
    let mut bundled = String::new();

    for import in extract_imports(&source) {
        let resolved = oxc_resolver::resolve(&import, path)?;
        bundled.push_str(&bundle_module(&resolved, visited)?);
    }

    bundled.push_str(&strip_imports(&source));
    Ok(bundled)
}
```

### Phase 2: Implement Critical Stub Methods

Priority order based on plugin usage:

1. **`spawnProcess`** - Used by git plugins (git_blame, git_grep, git_log, live_grep)
2. **`addOverlay` / `clearNamespace`** - Used by syntax highlighting plugins
3. **`defineMode`** - Used by modal keybinding plugins
4. **`startPrompt` / `setPromptSuggestions`** - Used by interactive plugins

### Phase 3: Complete API Implementation

- Virtual buffer support (`createVirtualBufferInSplit`, `setVirtualBufferContent`)
- Split management (`closeSplit`, `setSplitBuffer`)
- Line indicators (`setLineIndicator`, `clearLineIndicators`)
- Buffer info (`getBufferInfo`, `getBufferSavedDiff`)

### Phase 4: Testing & Optimization

- Performance comparison vs deno_core
- Memory usage profiling
- Plugin execution benchmarks

## Completed Tasks

1. **Remove deno_core dependencies from Cargo.toml**
   - Removed: `deno_core`, `deno_ast`, `deno_error`, `v8`
   - Added: `rquickjs`, `oxc_*` crates

2. **Remove deno_core backend files**
   - Deleted: `src/services/plugins/backend/deno_core_backend.rs`
   - Deleted: `src/services/plugins/runtime.rs`
   - Deleted: `src/v8_init.rs`

3. **Simplify backend/mod.rs**
   - Removed conditional compilation feature flags
   - Only exports QuickJS backend

4. **Update thread.rs**
   - Uses `QuickJsBackend::new()` instead of `TypeScriptRuntime`

5. **Update test harness**
   - Removed V8 initialization from `tests/common/harness.rs`

6. **Implement TypeScript transpilation**
   - Parse -> Semantic analysis -> Transform -> Codegen

7. **Implement QuickJS backend**
   - IIFE wrapping for scope isolation
   - ES module import detection and skip

## Editor API Implementation

### Fully Implemented (~30 methods)

| Category | Methods |
|----------|---------|
| Status/Logging | `setStatus`, `debug`, `copyToClipboard` |
| Buffer Info | `getActiveBufferId`, `getBufferPath`, `getBufferLength`, `isBufferModified` |
| Cursor | `getCursorPosition`, `setBufferCursor` |
| Text Editing | `insertText`, `deleteRange`, `insertAtCursor` |
| Commands | `registerCommand`, `setContext` |
| Files | `openFile`, `showBuffer`, `closeBuffer` |
| Splits | `getActiveSplitId` |
| Events | `on`, `off` |
| Environment | `getEnv`, `getCwd` |
| Paths | `pathDirname`, `pathBasename`, `pathExtname`, `pathIsAbsolute`, `pathJoin` |
| File System | `fileExists`, `readFile`, `writeFile` |

### Stub Implementations (~15 methods)

These log warnings but allow plugins to load:

- `defineMode` - Modal keybindings
- `addOverlay`, `clearNamespace` - Syntax highlighting
- `spawnProcess` - External processes
- `setPromptSuggestions`, `startPrompt` - Interactive prompts
- `refreshLines` - Force refresh
- `getTextPropertiesAtCursor`, `getBufferInfo` - Buffer metadata
- `createVirtualBufferInSplit`, `setVirtualBufferContent` - Virtual buffers
- `closeSplit`, `setSplitBuffer` - Split management
- `clearLineIndicators`, `setLineIndicator` - Gutter indicators
- `getBufferSavedDiff` - Diff from saved

## File Structure

```
src/services/plugins/
├── backend/
│   ├── mod.rs              # Exports QuickJsBackend as SelectedBackend
│   └── quickjs_backend.rs  # QuickJS implementation (~1000 lines)
├── api.rs                  # EditorStateSnapshot, PluginCommand, etc.
├── thread.rs               # Plugin thread runner
├── hooks.rs                # Hook definitions
├── event_hooks.rs          # Event hook system
└── process.rs              # Process spawning (not yet integrated)
```

## Dependencies

```toml
# QuickJS JavaScript runtime with oxc for TypeScript transpilation
rquickjs = { version = "0.9", features = ["bindgen", "futures", "macro"] }
oxc_transformer = "0.102"
oxc_allocator = "0.102"
oxc_parser = "0.102"
oxc_span = "0.102"
oxc_codegen = "0.102"
oxc_semantic = "0.102"
# Future: oxc_resolver = "0.102"  # For ES module bundling
```

## Known Limitations

1. **No ES module imports** - Plugins with `import ... from` are skipped
   - Affected: `clangd_support.ts`
   - Workaround: Inline dependencies or use global state
   - Fix: Implement module bundling (see Phase 1)

2. **IIFE scope isolation** - Each plugin runs in an IIFE, not true ES modules
   - Plugins share `globalThis` for event handlers
   - Local `const`/`let` are properly isolated

3. **Stub implementations** - ~15 APIs log warnings but don't function
   - Plugins using these features will not work correctly
   - See Phase 2-3 for implementation plan

4. **Synchronous API** - Plugin API is synchronous even though QuickJS supports async
   - Future consideration for async plugin APIs

## Future: Type-Safe API Generation

Currently the plugin API is manually defined in multiple places (Rust bindings, TypeScript .d.ts, tests), leading to inconsistencies. A future improvement would be a single authoritative schema that auto-generates all artifacts.

### Option 1: Rust Proc Macro

```rust
#[plugin_api]
pub trait EditorApi {
    #[api(sync)]
    fn getCursorPosition(&self) -> CursorPosition;

    #[api(async)]
    fn readFile(&self, path: String) -> String;
}
```

**Pros**: Compile-time type safety, Rust-native
**Cons**: Complex macro implementation, TS generation needs extra tooling (ts-rs)

### Option 2: Schema File (TOML) + build.rs

```toml
[[api]]
name = "getCursorPosition"
returns = "CursorPosition"
sync = true

[[api]]
name = "insertAtCursor"
params = [{ name = "text", type = "String" }]
sync = true

[[types]]
name = "CursorPosition"
fields = [
  { name = "line", type = "usize", ts = "number" },
  { name = "column", type = "usize", ts = "number" }
]
```

**Generates**:
- `generated_bindings.rs` - QuickJS registration code
- `fresh.d.ts` - TypeScript definitions
- `api_tests.rs` - Test for each API call

**Pros**: Simple, language-agnostic, fast iteration
**Cons**: Not compile-time checked, schema can drift from implementation

### Option 3: TypeScript as Source of Truth

```typescript
interface EditorApi {
  getCursorPosition(): CursorPosition;
  readFile(path: string): Promise<string>;
}
```

**Pros**: Natural for plugin authors, familiar syntax
**Cons**: Need TS parser in build, Rust type mapping complex

### Recommendation

**Option 1 (proc macro)** is preferred for long-term maintainability and compile-time type safety. The Rust type system will catch API mismatches at compile time, and ts-rs can generate TypeScript definitions from the same types. While more complex to implement initially, it provides stronger guarantees and better IDE support.

## References

- [rquickjs crate](https://docs.rs/rquickjs/)
- [QuickJS engine](https://bellard.org/quickjs/)
- [oxc project](https://oxc-project.github.io/)
- [oxc_resolver](https://docs.rs/oxc_resolver/) - For future ES module support
