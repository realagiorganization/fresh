# Multi-Buffer Single-Tab Research

**Status**: Research Phase
**Date**: 2025-12-31
**Related Documents**:
- `COMPOSITE_BUFFER_ARCHITECTURE.md` - Proposed architecture (2025-12-22)
- `DIFF_BRANCH_CONTINUATION.md` - Current implementation status
- `REVIEW_DIFF_FEATURE.md` - Feature documentation
- `docs/SCROLL_SYNC_DESIGN.md` - Scroll synchronization approach (on branch)

## Executive Summary

This document researches the feasibility of displaying multiple underlying buffers within a single visual pane (tab), enabling unified views like side-by-side diff, unified diff, and 3-way merge—all without opening multiple tabs/splits.

**Key Finding**: The proposed Composite Buffer Architecture in `COMPOSITE_BUFFER_ARCHITECTURE.md` provides a solid foundation. This research extends that proposal with specific implementation considerations for side-by-side diff within a single tab.

---

## Current Architecture

### Buffer Model (`src/model/buffer.rs`)

- **TextBuffer**: Core text storage using PieceTree with integrated line tracking
- Each buffer has a unique `BufferId` and optional file path
- Virtual buffers are "content-less"—they receive text via `TextPropertyEntry[]` and don't back to files

### Split/Tab Model (`src/view/split.rs`)

```
SplitNode (Tree structure)
├── Leaf { buffer_id, split_id }  // Displays ONE buffer
└── Split { direction, first, second, ratio, split_id }  // Container

SplitViewState (Per-split)
├── cursors: Cursors
├── viewport: Viewport
├── open_buffers: Vec<BufferId>  // Tab bar for this split
├── sync_group: Option<u32>      // For scroll sync
└── layout: Option<Layout>       // View rendering
```

**Key Constraint**: A `SplitNode::Leaf` displays exactly ONE `BufferId`. There's no concept of a single split displaying multiple buffers inline.

### Current Side-by-Side Diff Approach

From `plugins/audit_mode.ts` and `REVIEW_DIFF_FEATURE.md`:

```
┌─────────────────────┬─────────────────────┐
│   Split 1 (NEW)     │   Split 2 (OLD)     │
│   buffer_id=42      │   buffer_id=43      │
│   (editable file)   │   (virtual buffer)  │
└─────────────────────┴─────────────────────┘
          ↑                     ↑
          └──── Scroll Sync ────┘
```

- Opens **two separate splits** side by side
- LEFT: Current file (editable), RIGHT: HEAD version (read-only virtual buffer)
- Scroll sync via `on_viewport_changed` hook or newer `ScrollSyncGroup`
- **Issues**: Two tabs appear, pane order convention (OLD|NEW vs NEW|OLD), line alignment

---

## Proposed Feature: Multi-Buffer Single Tab

### Goal

Display multiple underlying buffers in a **single visual pane** without creating separate tabs. Examples:

1. **Side-by-side diff**: Two columns (OLD | NEW) within one tab
2. **Unified diff**: Interleaved old/new lines with different source buffers
3. **3-way merge**: Base | Ours | Theirs in one view
4. **Code review**: Inline comments/suggestions from different sources

### Conceptual Model

```
┌─────────────────────────────────────────────┐
│              Single Tab/Split               │
│  ┌─────────────────┬──────────────────┐    │
│  │  Buffer A View  │  Buffer B View   │    │
│  │  (OLD content)  │  (NEW content)   │    │
│  │                 │                  │    │
│  └─────────────────┴──────────────────┘    │
└─────────────────────────────────────────────┘
```

The tab shows **one composite view** that internally references multiple buffers.

---

## Design Options

### Option 1: Composite Buffer (Virtual Buffer + References)

Create a special buffer type that doesn't own text but references sections from other buffers.

```rust
enum BufferContent {
    /// Normal buffer with owned PieceTree
    Owned(PieceTree),
    /// Composite buffer referencing other buffers
    Composite(CompositeBufferLayout),
}

struct CompositeBufferLayout {
    /// Regions that make up this composite view
    regions: Vec<CompositeRegion>,
}

struct CompositeRegion {
    /// Source buffer ID
    source_buffer: BufferId,
    /// Range in source buffer
    source_range: Range<usize>,
    /// Display position in composite view
    display_column: usize,  // 0=left, 1=right for side-by-side
    display_row_offset: usize,
    /// Visual style
    style: RegionStyle,
}
```

**Pros**:
- Clean abstraction—single buffer ID, single tab
- Cursor/selection can span across regions
- Syntax highlighting per source buffer

**Cons**:
- Complex line number mapping (which buffer's lines?)
- Edit handling: which buffer receives edits?
- Significant core changes to rendering pipeline

### Option 2: View Transform with Multi-Source Tokens

Extend the existing `ViewTransformPayload` to support tokens from multiple sources.

```typescript
interface ViewTokenWire {
    source_offset: Option<usize>,
    source_buffer?: BufferId,  // NEW: which buffer this token comes from
    kind: ViewTokenWireKind,
    style?: ViewTokenStyle,
}
```

The plugin generates a token stream that interleaves content from multiple buffers:

```
[Token(buffer=A, "line 1 old\n"), Token(buffer=B, "line 1 new\n")]
[Token(buffer=A, "line 2 old\n"), Token(buffer=B, "line 2 new\n")]
```

**Pros**:
- Minimal core changes—plugins handle layout
- Flexible: works for side-by-side, unified, 3-way
- Leverages existing view transform infrastructure

**Cons**:
- Cursor byte positions become ambiguous (which buffer?)
- No direct editing—view-only (acceptable for diff view)
- Line numbers need special handling

### Option 3: Split-Within-Split (Nested Layouts)

Allow a single "tab" to internally render as side-by-side without creating separate SplitNode entries.

```rust
struct SplitViewState {
    // Existing fields...

    /// Optional inline layout for rendering multiple buffers
    inline_layout: Option<InlineLayout>,
}

enum InlineLayout {
    /// Side-by-side: two buffers rendered in columns
    SideBySide {
        left_buffer: BufferId,
        right_buffer: BufferId,
        ratio: f32,
        scroll_sync: bool,
    },
    /// Unified: interleaved lines from two buffers
    Unified {
        old_buffer: BufferId,
        new_buffer: BufferId,
        hunks: Vec<UnifiedHunk>,
    },
}
```

**Pros**:
- Keeps buffer model unchanged
- Single tab visually, but internally manages two buffers
- Could support editing in one "side"

**Cons**:
- Render pipeline complexity
- Input routing: which side receives keystrokes?
- Split separator handling within the tab

### Option 4: Virtual Buffer with Embedded Regions (Emacs-style)

Use text properties to mark regions that "belong to" different logical sources.

```typescript
entries.push({
    text: "- old line\n",
    properties: {
        source_buffer: oldBufferId,
        source_byte: 100,
        region_type: "deletion",
    }
});
entries.push({
    text: "+ new line\n",
    properties: {
        source_buffer: newBufferId,
        source_byte: 100,
        region_type: "addition",
    }
});
```

**Pros**:
- Already supported infrastructure
- Plugins have full control over layout
- Works today for read-only views

**Cons**:
- Content is copied into virtual buffer (not live references)
- Changes to source buffers require regenerating virtual buffer
- No editing support

---

## Recommended Approach: Phased Implementation

### Phase 1: Enhanced Virtual Buffer (Option 4 + Improvements)

Use the existing virtual buffer system with enhanced text properties:

1. **Source Tracking**: Add `source_buffer` and `source_byte` properties
2. **Live Refresh**: Watch source buffers for changes, regenerate view
3. **Click-to-Navigate**: Clicking a region opens the source buffer at that location
4. **Flexible Layout**: Plugin controls side-by-side vs unified via token generation

```typescript
// Plugin generates side-by-side by alternating columns
function generateSideBySideDiff(oldBuffer: BufferId, newBuffer: BufferId) {
    const entries: TextPropertyEntry[] = [];
    const leftWidth = 80;

    for (const [oldLine, newLine] of alignedLines) {
        entries.push({
            text: padRight(oldLine, leftWidth) + " │ " + newLine + "\n",
            properties: {
                leftSource: { buffer: oldBuffer, byte: oldByteOffset },
                rightSource: { buffer: newBuffer, byte: newByteOffset },
            }
        });
    }
    return entries;
}
```

### Phase 2: Inline Split Rendering (Option 3)

If editing support is needed, implement `InlineLayout`:

1. Render engine recognizes `inline_layout` and draws two buffer views side-by-side
2. Cursor focus determines which buffer receives input
3. Tab bar shows single entry with special indicator
4. Scroll sync is automatic (built into render)

### Phase 3: Composite Buffer (Option 1)

For full editing support with semantic understanding:

1. Core support for buffers that reference other buffers
2. Edits propagate to source buffers
3. Undo/redo tracks operations across referenced buffers

---

## Side-by-Side Diff Specific Considerations

### Line Alignment Challenge

Diff hunks have different line counts in old vs new:
```
OLD (3 lines)     NEW (5 lines)
line 1            line 1
line 2            line 2a
line 3            line 2b
                  line 2c
                  line 3
```

**Solutions**:
1. **Padding**: Insert blank lines in shorter side
2. **Anchor-based sync**: Sync at hunk boundaries, allow drift within hunks
3. **Semantic alignment**: Use diff algorithm to pair corresponding lines

### Rendering Architecture

```
┌─ Single Tab ─────────────────────────────────────┐
│ ┌─ Gutter ─┐┌─ Left View ─┐│┌─ Right View ─┐    │
│ │   1      ││ old line 1  ││ new line 1   │    │
│ │   2      ││ old line 2  ││ new line 2a  │    │
│ │   -      ││             ││ new line 2b  │    │
│ │   3      ││ old line 3  ││ new line 3   │    │
│ └──────────┘└─────────────┘│└─────────────┘    │
└─────────────────────────────────────────────────┘
```

**Gutter options**:
- Show left line numbers only
- Show both (e.g., "1|1", "2|2a")
- Show semantic "hunk" indicators

---

## Implementation Roadmap

### Milestone 1: Read-Only Side-by-Side (2-3 weeks of work)

1. Create `DiffViewFactory` plugin utility
2. Generate side-by-side layout with padding alignment
3. Add source buffer tracking in text properties
4. Implement click-to-navigate to source

### Milestone 2: Scroll Sync (1 week)

1. Leverage existing `ScrollSyncManager` for within-tab sync
2. Single scroll position, compute offsets per column

### Milestone 3: Editing Support (3-4 weeks)

1. Implement `InlineLayout` in `SplitViewState`
2. Render two buffer viewports within single split
3. Focus management between left/right
4. Cursor indicator for active side

### Milestone 4: 3-Way Merge (2-3 weeks)

1. Extend `InlineLayout` to support 3 buffers
2. Conflict detection and resolution UI
3. Accept/reject per region

---

## Related Work in Other Editors

### VS Code

- Uses "DiffEditor" component with two `CodeEditor` instances
- Renders in single DOM element but manages two models
- Line decorations show gutter indicators

### Zed

- `MultiBuffer` concept: single buffer aggregating excerpts from multiple files
- Each excerpt maintains reference to source buffer
- Edits propagate to source

### Vim/Neovim

- `:diffsplit` creates separate windows
- `vimdiff` synchronizes scroll via events
- No true "single buffer, multiple sources"

### Emacs

- `ediff` uses three separate windows + control panel
- `smerge-mode` renders conflict markers inline in single buffer

---

## Open Questions

1. **Line Numbers**: Show left/right/both/neither?
2. **Cursor**: Can cursor exist in both sides? Or focus-based?
3. **Selection**: Can selection span across left/right boundary?
4. **Editing**: Edit one side? Both? Neither?
5. **Syntax Highlighting**: Per-source-buffer or unified?
6. **Performance**: Large diffs with thousands of hunks?

---

## Existing Proposed Architecture (COMPOSITE_BUFFER_ARCHITECTURE.md)

The codebase already has a detailed proposal in `COMPOSITE_BUFFER_ARCHITECTURE.md`. Key concepts:

### SectionDescriptor

```rust
struct SectionDescriptor {
    id: String,               // Unique ID for the section
    source_buffer_id: BufferId,
    range: Range<usize>,      // Byte or line range in the source
    style: SectionStyle,      // Border type, markers (+/-), padding
    heading: Option<String>,  // Header text (e.g., filename or "In [5]:")
    is_editable: bool,        // Whether to allow input routing
    metadata: serde_json::Value,
}
```

### Synthesis Pipeline

1. **Token Fetching**: For each section, request already-computed tokens from Source Buffer's HighlightEngine
2. **Framing (Box Engine)**: Inject UI-only tokens for borders (`┌`, `│`, `└`), markers (`+`, `-`), and gutters
3. **Coordinate Mapping**: Bidirectional mapping between Composite Viewport and Source Buffer positions

### Live Editing & Input Routing

When a key is pressed:
1. Editor identifies the buffer/byte under cursor via Mapping Table
2. If mapping exists and `is_editable` is true: reroute `Insert`/`Delete` to Source Buffer
3. If on protected character (border/header): block input

---

## Mapping Existing Architecture to Side-by-Side Diff

### Proposed Implementation

For side-by-side diff within a single tab, the Composite Buffer approach maps as follows:

```
┌─────────────────────────────────────────────────────────┐
│                   Composite Buffer                       │
│  ┌─ Section 1 ──────────┐  ┌─ Section 2 ──────────┐    │
│  │ source: HEAD buffer  │  │ source: Working Copy │    │
│  │ range: 0..entire     │  │ range: 0..entire     │    │
│  │ display_column: 0    │  │ display_column: 1    │    │
│  │ is_editable: false   │  │ is_editable: true    │    │
│  └──────────────────────┘  └──────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

### Token Stream for Side-by-Side

The synthesis pipeline would generate tokens like:

```
Line 1: [Token(src=HEAD, "old line 1"), Token(UI, "│"), Token(src=WC, "new line 1"), Newline]
Line 2: [Token(src=HEAD, "old line 2"), Token(UI, "│"), Token(src=WC, "new line 2"), Newline]
Line 3: [Token(UI, "<padding>   "), Token(UI, "│"), Token(src=WC, "new line 3"), Newline]  // Added line
```

### Coordinate Mapping for Diff View

```
ViewLine:
  char[0..10]   → maps to HEAD buffer, byte offset 0..10
  char[11]      → maps to UI separator (protected)
  char[12..22]  → maps to Working Copy buffer, byte offset 0..10
```

---

## Comparison: Current Split-Based vs Proposed Single-Tab

| Aspect | Current (Two Splits) | Proposed (Single Tab) |
|--------|---------------------|----------------------|
| Tab count | 2 (clutter) | 1 (clean) |
| Scroll sync | Via hooks, can jitter | Built into render |
| Line alignment | None (lines drift) | Pixel-perfect via padding |
| Editing | Each side independent | Routed via coordinate map |
| Implementation | Working but fragile | Requires core changes |
| Performance | Overhead of sync | Single render pass |

---

## Recommended Implementation Strategy

Based on the existing Composite Buffer Architecture, here is the recommended approach:

### Phase 1: Plugin-Side Layout (Near-term, No Core Changes)

Use existing virtual buffers with enhanced text properties:

```typescript
// Side-by-side as single virtual buffer
const leftWidth = Math.floor(viewportWidth / 2) - 1;
for (let i = 0; i < alignedLines.length; i++) {
    const [oldLine, newLine] = alignedLines[i];
    entries.push({
        text: padRight(oldLine, leftWidth) + " │ " + padRight(newLine, leftWidth) + "\n",
        properties: {
            leftSource: { buffer: headBufferId, line: oldLineNum },
            rightSource: { buffer: workingBufferId, line: newLineNum },
        }
    });
}
```

**Limitations**: Read-only, content copied not referenced, no live updates.

### Phase 2: Core Multi-Source Token Support

Extend `ViewTokenWire` to track source buffer:

```rust
pub struct ViewTokenWire {
    pub source_offset: Option<usize>,
    pub source_buffer: Option<BufferId>,  // NEW
    pub kind: ViewTokenWireKind,
    pub style: Option<ViewTokenStyle>,
}
```

Update view pipeline to handle multi-source tokens.

### Phase 3: Full Composite Buffer with Editing

Implement `SectionDescriptor`, coordinate mapping, and input routing as described in `COMPOSITE_BUFFER_ARCHITECTURE.md`.

---

## Conclusion

The recommended approach is:

1. **Start with enhanced virtual buffers** (Phase 1) for read-only diff views
2. **Add multi-source token support** (Phase 2) for proper source tracking
3. **Implement full Composite Buffer Architecture** (Phase 3) for editable diffs

The existing `COMPOSITE_BUFFER_ARCHITECTURE.md` provides a solid foundation. The key extensions needed are:
- Column-based layout for side-by-side (not just vertical sections)
- Line alignment with padding for differing line counts
- Synchronized scrolling within the single view

This provides immediate value (better diff viewing) while establishing patterns for future merge/conflict resolution features.
