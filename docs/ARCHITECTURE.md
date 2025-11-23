# Fresh Architecture

Fresh is a high-performance, terminal-based text editor built in Rust. It's designed to be fast, responsive, and extensible, with a modern architecture that draws inspiration from the best ideas in the world of text editors.

## Core Design Principles

*   **Performance First:** Every architectural decision is made with performance in mind. This includes the choice of data structures, the design of the rendering pipeline, and the implementation of core features.
*   **Event-Driven:** All state changes are represented as events, which are processed by a central event loop. This makes the editor's state predictable and enables features like unlimited undo/redo.
*   **Asynchronous I/O:** All file and process I/O is handled asynchronously on a separate thread pool. This ensures that the editor's UI is never blocked by slow I/O operations.
*   **Extensible:** A powerful TypeScript-based plugin system allows for deep customization and extension of the editor's functionality.

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────┐
│                   MAIN THREAD (Sync)                    │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────┐    │
│  │ Event Loop   │→ │  Editor      │→ │  Renderer   │    │
│  │ (crossterm)  │  │  (state)     │  │  (ratatui)  │    │
│  └──────────────┘  └──────────────┘  └─────────────┘    │
│         ↓                 ↑                             │
│    Input Queue      EventQueue (mpsc)                   │
└─────────────────────────────────────────────────────────┘
         ↑                      ↑
         │ send events          │ send messages
         │                      │
    ┌────┴──────────┐  ┌────────┴──────────┐
    │ LSP Tasks     │  │ File I/O Tasks    │
    │ (Tokio)       │  │ (Tokio)           │
    └───────────────┘  └───────────────────┘
```

## The Document Model

To provide a clean separation between the editor's UI and the underlying text buffer, Fresh uses a `DocumentModel` trait. This abstraction layer is responsible for all interactions with the text buffer and provides a consistent API for both small and large files.

### Dual Position System

To support multi-gigabyte files where line indexing may be unavailable, the `DocumentModel` uses a dual position system:

*   **`DocumentPosition::LineColumn`:** For small files, this provides precise line and column-based positioning.
*   **`DocumentPosition::ByteOffset`:** For large files, this provides byte-offset-based positioning, which is always available and precise.

### The `DocumentModel` Trait

The `DocumentModel` trait defines a set of methods for interacting with the document, including:

*   **`get_viewport_content`:** The core rendering primitive, which returns the content for the visible portion of the screen.
*   **`position_to_offset` and `offset_to_position`:** For converting between the two position types.
*   **`insert`, `delete`, and `replace`:** For modifying the document's content.

This abstraction allows the rest of the editor to be blissfully unaware of the details of the underlying text buffer, such as whether it's a small file with a full line index or a large file with lazy loading.

## The Buffer

The core of the editor is the text buffer, which is implemented as a **`PieceTree`**. A `PieceTree` is a balanced binary tree that represents the text as a sequence of "pieces," which are references to either the original, immutable file buffer or an in-memory buffer of user additions.

This data structure provides several key advantages:

*   **O(log n) Edits:** Inserts and deletes are O(log n), where n is the number of pieces. This makes text editing extremely fast, even in large files.
*   **Efficient Memory Usage:** The `PieceTree` only stores the changes to the file, not the entire file content. This makes it very memory-efficient, especially for large files.
*   **Lazy Loading:** For multi-gigabyte files, Fresh uses a lazy loading strategy. The file is not loaded into memory all at once. Instead, chunks of the file are loaded on demand as the user scrolls through the file.

## The Rendering Pipeline

The rendering pipeline is designed to be as efficient as possible, especially when it comes to overlays (visual decorations like highlights and squiggly lines).

### The Overlay Problem

A naive implementation of overlays, where each character on the screen is checked against a list of all active overlays, can lead to significant performance problems. This is an O(N*M) problem, where N is the number of characters on the screen and M is the number of overlays.

### The Solution: A High-Performance Overlay System

Fresh uses a multi-pronged approach to solve the overlay problem:

1.  **Line-Indexed Overlay Storage:** Instead of storing overlays in a flat list, they are stored in a `BTreeMap<usize, Vec<Overlay>>`, where the key is the starting line number of the overlay. This allows for a very fast lookup of all overlays on a given line.
2.  **Render-Time Overlay Cache:** During each frame, the editor creates a cache of all overlays that are visible in the current viewport. This cache is then used to apply the overlays to the text, avoiding the need to query the overlay manager for each character.
3.  **Diagnostic Hash Check:** For LSP diagnostics, which are a major source of overlays, Fresh uses a hash check to avoid redundant updates. If the set of diagnostics from the language server hasn't changed, no work is done.

This new architecture, which is currently being implemented, will provide a massive performance improvement over the old system and will allow Fresh to handle thousands of overlays without breaking a sweat.

## LSP Integration

Fresh has a deep and robust integration with the Language Server Protocol (LSP), providing features like code completion, diagnostics, and go-to-definition.

The LSP integration is built on a multi-threaded architecture that ensures the editor's UI is never blocked by the language server.

*   **`LspManager`:** A central coordinator that manages multiple language servers (one for each language).
*   **`LspHandle`:** A handle to a specific language server, providing a non-blocking API for sending commands and notifications.
*   **`AsyncBridge`:** An `mpsc` channel that bridges the asynchronous world of the LSP tasks with the synchronous world of the editor's main event loop.

This architecture allows Fresh to communicate with language servers in a highly efficient and non-blocking way, providing a smooth and responsive user experience.

## Annotated Views

Fresh supports **Annotated Views**, a powerful pattern for displaying file content with injected annotations (such as headers, metadata lines, or visual separators) while preserving core editor features like line numbers and syntax highlighting. This architecture enables features like git blame, code coverage overlays, and inline documentation.

### The Problem

Consider displaying git blame information: you want to show the file content with header lines above each block indicating the commit, author, and date. A naive approach creates several problems:

1. **Line Number Mismatch:** If headers are part of the buffer content, line numbers include them, making it impossible to show "line 42" next to the actual line 42 of the source file.

2. **Syntax Highlighting Loss:** If the buffer contains mixed content (headers + code), tree-sitter cannot parse it correctly, and syntax highlighting breaks.

3. **Historical Content:** For features like "blame at parent commit," you need to display historical file versions that differ from the current file.

### The Solution: View Transforms + Virtual Buffers

Fresh solves this with a hybrid architecture combining two mechanisms:

```
┌─────────────────────────────────────────────────────────────────┐
│                    ANNOTATED VIEW ARCHITECTURE                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │              Virtual Buffer (Content Layer)              │  │
│   │  • Contains actual file content (current or historical) │  │
│   │  • Language detected from buffer name extension         │  │
│   │  • Syntax highlighting via tree-sitter                  │  │
│   │  • Text properties store annotation metadata            │  │
│   └─────────────────────────────────────────────────────────┘  │
│                              │                                  │
│                              ▼                                  │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │             View Transform (Presentation Layer)          │  │
│   │  • Injects annotation headers between content blocks    │  │
│   │  • Headers: source_offset = None (no line number)       │  │
│   │  • Content: source_offset = Some(byte) (has line num)   │  │
│   │  • Styling via ViewTokenWire.style field                │  │
│   └─────────────────────────────────────────────────────────┘  │
│                              │                                  │
│                              ▼                                  │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │                   Rendered Output                        │  │
│   │     ── abc123 (Alice, 2 days ago) "Fix bug" ──          │  │
│   │  42 │ fn main() {                                       │  │
│   │  43 │     println!("Hello");                            │  │
│   │     ── def456 (Bob, 1 week ago) "Add feature" ──        │  │
│   │  44 │     do_something();                               │  │
│   │  45 │ }                                                 │  │
│   └─────────────────────────────────────────────────────────┘  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### Key Components

#### 1. Virtual Buffer with Content

The first layer is a virtual buffer containing the actual file content:

```typescript
// Create buffer with historical or current content
const content = await editor.spawnProcess("git", ["show", `${commit}:${path}`]);
const bufferId = await editor.createVirtualBuffer({
  name: `*annotated:${basename(path)}*`,  // Extension enables syntax highlighting
  mode: "annotated-view",
  read_only: true,
  entries: [{ text: content.stdout, properties: { commit, path } }],
});
```

**Key properties:**
- Buffer name includes file extension (e.g., `*blame:main.rs*`) for automatic language detection
- Content is pure source code, enabling correct tree-sitter parsing
- Text properties can store metadata without affecting content

#### 2. ViewTokenWire with Source Mapping

The `ViewTokenWire` structure enables precise control over line numbering:

```rust
pub struct ViewTokenWire {
    /// Source byte offset - None for injected annotations
    pub source_offset: Option<usize>,
    pub kind: ViewTokenWireKind,
    /// Optional styling for injected content
    pub style: Option<ViewTokenStyle>,
}

pub struct ViewTokenStyle {
    pub fg: Option<(u8, u8, u8)>,
    pub bg: Option<(u8, u8, u8)>,
    pub bold: bool,
    pub italic: bool,
}
```

The `source_offset` field is the key mechanism:
- **`Some(byte_position)`:** Token maps to source content. The renderer:
  - Includes it in line number calculation
  - Applies syntax highlighting from that byte position
  - Enables cursor positioning and selection
- **`None`:** Token is injected annotation. The renderer:
  - Skips line number increment (shows blank in gutter)
  - Applies `style` field if present
  - Does not participate in source-based features

#### 3. View Transform Hook

Plugins register for the `view_transform_request` hook, called each render frame:

```typescript
editor.on("view_transform_request", "onViewTransform");

globalThis.onViewTransform = function(args: {
  buffer_id: number;
  split_id: number;
  viewport_start: number;
  viewport_end: number;
  tokens: ViewTokenWire[];
}) {
  // Only transform our annotated buffers
  if (!isAnnotatedBuffer(args.buffer_id)) return;

  const transformed: ViewTokenWire[] = [];

  for (const block of getAnnotationBlocks(args.buffer_id)) {
    // Inject header (no source mapping = no line number)
    injectHeader(transformed, block, {
      source_offset: null,
      style: { bg: [50, 50, 55], fg: [220, 220, 220], bold: true }
    });

    // Pass through content tokens (preserve source mapping)
    for (const token of args.tokens) {
      if (tokenInBlock(token, block)) {
        transformed.push(token);  // Unchanged - keeps line numbers & highlighting
      }
    }
  }

  editor.submitViewTransform(
    args.buffer_id,
    args.split_id,
    args.viewport_start,
    args.viewport_end,
    transformed,
    null
  );
};
```

### How Line Numbers Work

The renderer in `split_rendering.rs` handles line numbers via the `is_continuation` check:

```rust
// Check if previous character had no source mapping
let is_continuation = if line_view_offset > 0 {
    view_mapping.get(line_view_offset - 1) == Some(&None)
} else {
    false
};

// Only increment line number for content lines
if !is_continuation && lines_rendered > 0 {
    current_source_line_num += 1;
}
```

This means:
- Lines starting after a `source_offset: None` newline show blank in the line number gutter
- Lines starting after a `source_offset: Some(_)` newline increment and display the line number
- The result: annotation headers have no line numbers, content lines have correct source line numbers

### How Syntax Highlighting Works

Syntax highlighting is applied based on `source_offset`:

```rust
let highlight_color = byte_pos.and_then(|bp| {
    highlight_spans
        .iter()
        .find(|span| span.range.contains(&bp))
        .map(|span| span.color)
});
```

- Content tokens have `source_offset: Some(byte)` → looked up in highlight spans → colored
- Annotation tokens have `source_offset: None` → no highlight lookup → uses `style` field instead

### Use Cases

#### Git Blame

```
Buffer: Historical file content from `git show commit:path`
Annotations: Commit headers above each blame block
Line numbers: Match historical file lines
Highlighting: Based on file language
```

#### Code Coverage

```
Buffer: Current file content
Annotations: Coverage percentage headers above functions
Line numbers: Match current file
Highlighting: Normal syntax + coverage overlays
```

#### Inline Documentation

```
Buffer: Source code
Annotations: Doc comments rendered as styled blocks
Line numbers: Only for code lines
Highlighting: Code highlighted, docs styled differently
```

### Performance Considerations

1. **Viewport-Only Processing:** View transforms only process the visible viewport, not the entire file. For a 100K line file, only ~50 lines are transformed per frame.

2. **Efficient Block Lookup:** Annotation metadata should be stored in a sorted structure enabling O(log n) lookup of blocks overlapping the viewport.

3. **Frame-Rate Transform:** The `view_transform_request` hook is called every frame. Plugins must respond quickly. Pre-compute annotation positions; don't run git commands during the hook.

4. **Caching:** View transforms are cached per-split and reused until explicitly cleared or the viewport changes significantly.

### Implementation Checklist

To implement an annotated view feature:

1. **Define annotation structure:** What metadata accompanies each block? (commit info, coverage data, etc.)

2. **Create content buffer:** Use `createVirtualBuffer` with appropriate name for language detection

3. **Store block positions:** Track byte ranges for each annotation block in the content

4. **Implement view transform hook:** Inject headers with `source_offset: None` and `style`, pass through content tokens unchanged

5. **Handle navigation:** Map cursor positions in the view back to logical positions in your annotation model

6. **Support refresh:** When annotations change (e.g., blame at different commit), update buffer content and block positions; view transform auto-updates on next render