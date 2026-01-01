# Plan: Reuse Normal Buffer Rendering for Composite Buffers

## Problem Statement

The normal buffer rendering (`render_view_lines()`) is a ~700 line function with sophisticated features:
- ViewLine-based rendering with per-character mappings
- Syntax highlighting via compute_char_style()
- Selection rendering (multiple cursors, block selection)
- ANSI sequence handling
- Virtual text injection
- Line wrapping with proper line number handling

The composite buffer rendering is currently simplified and missing these features.

## Current Architecture

```
Normal Buffer:
  EditorState
      ↓
  build_view_data() → Vec<ViewLine>
      ↓
  selection_context() → SelectionContext (byte-based)
      ↓
  decoration_context() → DecorationContext (byte-based)
      ↓
  render_view_lines(LineRenderInput) → rendered output
```

The problem: `render_view_lines()` is tightly coupled to:
1. **EditorState** - for buffer lookups, debug mode, etc.
2. **Byte-based contexts** - SelectionContext and DecorationContext use byte offsets

For composite buffers, we have:
- Multiple source buffers (one per pane)
- Alignment that maps display rows to source lines
- Per-pane viewports and cursors

## Proposed Architecture: Extract LineRenderer

### Step 1: Extract Per-Line Rendering

Create a new struct that encapsulates line rendering logic:

```rust
/// Renders a single line with full styling support
pub struct LineRenderer<'a> {
    theme: &'a Theme,
    left_column: usize,
    is_active: bool,
}

impl<'a> LineRenderer<'a> {
    /// Render a ViewLine with styling
    pub fn render_line(
        &self,
        view_line: &ViewLine,
        line_context: LineContext,
    ) -> RenderedLine {
        // Character-by-character rendering with:
        // - Horizontal scrolling (skip left_column chars)
        // - Cursor highlighting (if is_cursor)
        // - Selection highlighting (if in selection range)
        // - Syntax highlighting (from highlight_spans)
        // - ANSI handling
    }
}

/// Context for a single line (replaces byte-based contexts)
pub struct LineContext {
    /// Line number to display (None for continuation lines)
    pub line_number: Option<usize>,

    /// Cursor column position on this line (None if no cursor)
    pub cursor_column: Option<usize>,

    /// Selection ranges within this line (character-based, not byte-based)
    pub selection_ranges: Vec<Range<usize>>,

    /// Highlight spans relative to this line
    pub highlight_spans: Vec<HighlightSpan>,

    /// Background color override (for diff highlighting)
    pub background_override: Option<Color>,
}

pub struct RenderedLine {
    pub spans: Vec<Span<'static>>,
    pub cursor_x: Option<u16>,  // Screen X position of cursor, if on this line
}
```

### Step 2: Refactor render_view_lines to Use LineRenderer

```rust
fn render_view_lines(input: LineRenderInput<'_>) -> LineRenderOutput {
    let renderer = LineRenderer::new(input.theme, input.left_column, input.is_active);

    let mut lines = Vec::new();
    for (view_line, line_context) in view_lines_with_context(input) {
        let rendered = renderer.render_line(view_line, line_context);
        lines.push(Line::from(rendered.spans));
    }

    LineRenderOutput { lines, ... }
}

/// Builds LineContext for each ViewLine from the byte-based contexts
fn view_lines_with_context<'a>(
    input: &'a LineRenderInput<'a>,
) -> impl Iterator<Item = (&'a ViewLine, LineContext)> {
    // Convert byte-based SelectionContext to per-line selection ranges
    // Convert byte-based DecorationContext to per-line highlight spans
}
```

### Step 3: Use LineRenderer for Composite Buffers

```rust
fn render_composite_buffer(...) {
    let renderer = LineRenderer::new(theme, 0, is_active);

    for display_row in visible_rows {
        let aligned_row = &alignment.rows[display_row];

        for (pane_idx, source) in sources.iter().enumerate() {
            // Get ViewLine from source buffer
            let view_line = get_pane_view_line(source_state, aligned_row, pane_idx);

            // Build per-line context
            let line_context = LineContext {
                line_number: aligned_row.get_pane_line(pane_idx).map(|r| r.line + 1),
                cursor_column: if is_cursor_row && is_focused_pane {
                    Some(view_state.cursor_column)
                } else {
                    None
                },
                selection_ranges: vec![],  // TODO: from pane_cursors
                highlight_spans: get_highlight_spans_for_line(source_state, line_num),
                background_override: get_diff_background(aligned_row.row_type),
            };

            let rendered = renderer.render_line(&view_line, line_context);
            // ... render to frame
        }
    }
}
```

## Benefits

1. **Single source of truth** - All character styling logic in LineRenderer
2. **Testable** - LineRenderer can be unit tested
3. **Flexible contexts** - LineContext is line-relative, not byte-relative
4. **Incremental adoption** - Can refactor render_view_lines incrementally

## Implementation Order

1. Create `LineRenderer` struct with basic render_line() method
2. Extract character styling loop from render_view_lines() into LineRenderer
3. Add `LineContext` and context building for normal buffers
4. Verify normal buffer rendering still works
5. Add composite buffer support using LineRenderer
6. Add syntax highlighting for composite panes
7. Add selection rendering for composite panes

## Estimated Effort

- Phase 1 (Extract LineRenderer): ~2-3 hours of careful refactoring
- Phase 2 (Composite integration): ~1-2 hours
- Phase 3 (Full feature parity): ~2-3 hours

## Alternative: Quick Win

If full refactoring is too costly, we can:
1. Keep the current composite rendering (with scrollbar, cursor, horizontal scroll)
2. Just add syntax highlighting by querying the source buffer's highlighter
3. Accept some code similarity (not duplication) between render paths

This gets 80% of the benefit with 20% of the effort.
