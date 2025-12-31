# Multi-Buffer Single-Tab Architecture

**Status**: Implemented
**Date**: 2025-12-31
**Related Documents**:
- `COMPOSITE_BUFFER_ARCHITECTURE.md` - Original proposal
- `REVIEW_DIFF_FEATURE.md` - Feature documentation

---

## Implementation Summary

The multi-buffer single-tab architecture has been implemented with the following components:

### Core Infrastructure (Rust)
- `src/model/composite_buffer.rs` - Core data structures (CompositeBuffer, SourcePane, LineAlignment, DiffHunk)
- `src/view/composite_view.rs` - Per-split view state (CompositeViewState, PaneViewport)
- `src/view/composite_renderer.rs` - Rendering logic for side-by-side, stacked, and unified layouts
- `src/input/composite_router.rs` - Input routing to focused pane
- `src/app/composite_buffer_actions.rs` - Editor methods for composite buffer management

### Plugin API (TypeScript)
- `editor.createCompositeBuffer(options)` - Create a multi-buffer view
- `editor.updateCompositeAlignment(bufferId, hunks)` - Update diff alignment
- `editor.closeCompositeBuffer(bufferId)` - Close a composite buffer

### Example Plugin
- `plugins/diff_view.ts` - Demonstrates side-by-side diff comparing current file with git HEAD

---

## Overview

This document specifies the complete architecture for displaying multiple underlying buffers within a single visual pane (tab). This enables side-by-side diff, unified diff, 3-way merge, and code review views—all within a single tab.

---

## Core Data Structures

### 1. CompositeBuffer

A new buffer variant that synthesizes its view from multiple source buffers.

```rust
// src/model/composite_buffer.rs

use crate::model::event::BufferId;
use std::ops::Range;

/// A buffer that composes content from multiple source buffers
#[derive(Debug, Clone)]
pub struct CompositeBuffer {
    /// Unique ID for this composite buffer
    pub id: BufferId,

    /// Display name (shown in tab bar)
    pub name: String,

    /// Layout mode for this composite
    pub layout: CompositeLayout,

    /// Source buffer configurations
    pub sources: Vec<SourcePane>,

    /// Line alignment map (for side-by-side diff)
    /// Maps display_line -> (left_source_line, right_source_line)
    pub alignment: LineAlignment,

    /// Which pane currently has focus (for input routing)
    pub active_pane: usize,

    /// Mode for keybindings
    pub mode: String,
}

/// How the composite buffer arranges its source panes
#[derive(Debug, Clone)]
pub enum CompositeLayout {
    /// Side-by-side columns (for diff view)
    SideBySide {
        /// Width ratio for each pane (must sum to 1.0)
        ratios: Vec<f32>,
        /// Show separator between panes
        show_separator: bool,
    },
    /// Vertically stacked sections (for notebook cells)
    Stacked {
        /// Spacing between sections
        spacing: u16,
    },
    /// Interleaved lines (for unified diff)
    Unified,
}

/// Configuration for a single source pane within the composite
#[derive(Debug, Clone)]
pub struct SourcePane {
    /// ID of the source buffer
    pub buffer_id: BufferId,

    /// Human-readable label (e.g., "OLD", "NEW", "BASE")
    pub label: String,

    /// Whether this pane accepts edits
    pub editable: bool,

    /// Visual style for this pane
    pub style: PaneStyle,

    /// Byte range in source buffer to display (None = entire buffer)
    pub range: Option<Range<usize>>,
}

/// Visual styling for a pane
#[derive(Debug, Clone, Default)]
pub struct PaneStyle {
    /// Background color for added lines
    pub add_bg: Option<(u8, u8, u8)>,
    /// Background color for removed lines
    pub remove_bg: Option<(u8, u8, u8)>,
    /// Background color for modified lines
    pub modify_bg: Option<(u8, u8, u8)>,
    /// Gutter indicator style
    pub gutter_style: GutterStyle,
}

#[derive(Debug, Clone, Default)]
pub enum GutterStyle {
    #[default]
    LineNumbers,
    DiffMarkers,      // +/-/~
    Both,             // Line numbers + markers
    None,
}
```

### 2. LineAlignment

Critical for side-by-side diff—maps display lines to source lines with padding.

```rust
// src/model/line_alignment.rs

/// Alignment information for side-by-side views
#[derive(Debug, Clone)]
pub struct LineAlignment {
    /// Each entry maps a display row to source lines in each pane
    /// None means padding (blank line) for that pane
    pub rows: Vec<AlignedRow>,
}

#[derive(Debug, Clone)]
pub struct AlignedRow {
    /// Source line for each pane (None = padding)
    pub pane_lines: Vec<Option<SourceLineRef>>,
    /// Type of this row for styling
    pub row_type: RowType,
}

#[derive(Debug, Clone)]
pub struct SourceLineRef {
    /// Line number in source buffer (0-indexed)
    pub line: usize,
    /// Byte range in source buffer
    pub byte_range: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowType {
    /// Both sides have matching content
    Context,
    /// Line exists only in left/old
    Deletion,
    /// Line exists only in right/new
    Addition,
    /// Line differs between sides
    Modification,
    /// Hunk separator
    HunkHeader,
}

impl LineAlignment {
    /// Create alignment from a unified diff
    pub fn from_diff(old_lines: &[&str], new_lines: &[&str], hunks: &[DiffHunk]) -> Self {
        let mut rows = Vec::new();

        for hunk in hunks {
            // Add hunk header row
            rows.push(AlignedRow {
                pane_lines: vec![None, None],
                row_type: RowType::HunkHeader,
            });

            // Process hunk lines with LCS-based alignment
            let aligned = Self::align_hunk_lines(
                &old_lines[hunk.old_start..hunk.old_start + hunk.old_count],
                &new_lines[hunk.new_start..hunk.new_start + hunk.new_count],
            );
            rows.extend(aligned);
        }

        Self { rows }
    }

    /// Align lines within a hunk using Myers diff algorithm
    fn align_hunk_lines(old: &[&str], new: &[&str]) -> Vec<AlignedRow> {
        // Implementation uses patience diff or Myers algorithm
        // to pair corresponding lines and insert padding
        todo!()
    }
}
```

### 3. CompositeViewState

Per-split state for composite buffer rendering.

```rust
// src/view/composite_view.rs

use crate::model::event::{BufferId, SplitId};
use crate::view::viewport::Viewport;
use crate::model::cursor::Cursors;

/// View state for a composite buffer in a split
#[derive(Debug, Clone)]
pub struct CompositeViewState {
    /// The composite buffer being displayed
    pub composite_id: BufferId,

    /// Independent viewport per pane
    pub pane_viewports: Vec<PaneViewport>,

    /// Which pane has focus
    pub focused_pane: usize,

    /// Single scroll position (display row)
    /// All panes scroll together via alignment
    pub scroll_row: usize,

    /// Cursor positions per pane (for editing)
    pub pane_cursors: Vec<Cursors>,
}

#[derive(Debug, Clone)]
pub struct PaneViewport {
    /// Computed rect for this pane (set during render)
    pub rect: ratatui::layout::Rect,
    /// Horizontal scroll offset
    pub left_column: usize,
}

impl CompositeViewState {
    /// Scroll all panes together
    pub fn scroll(&mut self, delta: isize) {
        self.scroll_row = (self.scroll_row as isize + delta).max(0) as usize;
    }

    /// Switch focus to next pane
    pub fn focus_next_pane(&mut self, pane_count: usize) {
        self.focused_pane = (self.focused_pane + 1) % pane_count;
    }

    /// Switch focus to previous pane
    pub fn focus_prev_pane(&mut self, pane_count: usize) {
        self.focused_pane = (self.focused_pane + pane_count - 1) % pane_count;
    }
}
```

---

## Rendering Pipeline

### 4. CompositeRenderer

Renders a composite buffer as a unified view.

```rust
// src/view/composite_renderer.rs

use crate::model::composite_buffer::{CompositeBuffer, CompositeLayout};
use crate::view::composite_view::CompositeViewState;
use ratatui::prelude::*;
use ratatui::widgets::*;

pub struct CompositeRenderer<'a> {
    composite: &'a CompositeBuffer,
    view_state: &'a CompositeViewState,
    buffers: &'a BufferMap,  // Access to source buffers
}

impl<'a> CompositeRenderer<'a> {
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        match &self.composite.layout {
            CompositeLayout::SideBySide { ratios, show_separator } => {
                self.render_side_by_side(frame, area, ratios, *show_separator);
            }
            CompositeLayout::Stacked { spacing } => {
                self.render_stacked(frame, area, *spacing);
            }
            CompositeLayout::Unified => {
                self.render_unified(frame, area);
            }
        }
    }

    fn render_side_by_side(
        &self,
        frame: &mut Frame,
        area: Rect,
        ratios: &[f32],
        show_separator: bool,
    ) {
        // Calculate pane widths
        let separator_width = if show_separator { 1 } else { 0 };
        let total_separators = (self.composite.sources.len() - 1) * separator_width;
        let available_width = area.width.saturating_sub(total_separators as u16);

        let mut pane_rects = Vec::new();
        let mut x = area.x;

        for (i, ratio) in ratios.iter().enumerate() {
            let width = (available_width as f32 * ratio).round() as u16;
            pane_rects.push(Rect {
                x,
                y: area.y,
                width,
                height: area.height,
            });
            x += width;

            // Render separator
            if show_separator && i < ratios.len() - 1 {
                self.render_separator(frame, x, area.y, area.height);
                x += 1;
            }
        }

        // Render each pane
        for (i, (pane, rect)) in self.composite.sources.iter().zip(&pane_rects).enumerate() {
            let is_focused = i == self.view_state.focused_pane;
            self.render_pane(frame, pane, *rect, i, is_focused);
        }
    }

    fn render_pane(
        &self,
        frame: &mut Frame,
        pane: &SourcePane,
        rect: Rect,
        pane_index: usize,
        is_focused: bool,
    ) {
        let source_buffer = self.buffers.get(pane.buffer_id);
        let alignment = &self.composite.alignment;

        // Render visible rows based on alignment
        for display_row in self.view_state.scroll_row..self.view_state.scroll_row + rect.height as usize {
            if display_row >= alignment.rows.len() {
                break;
            }

            let aligned_row = &alignment.rows[display_row];
            let y = rect.y + (display_row - self.view_state.scroll_row) as u16;

            match &aligned_row.pane_lines[pane_index] {
                Some(source_ref) => {
                    // Render actual content from source buffer
                    let line_text = source_buffer.get_line(source_ref.line);
                    let style = self.style_for_row_type(aligned_row.row_type, pane);

                    self.render_line_with_gutter(
                        frame,
                        rect.x,
                        y,
                        rect.width,
                        &line_text,
                        source_ref.line,
                        style,
                        pane,
                    );
                }
                None => {
                    // Render padding (empty line)
                    let style = Style::default().bg(Color::Rgb(40, 40, 40));
                    frame.render_widget(
                        Paragraph::new("").style(style),
                        Rect { x: rect.x, y, width: rect.width, height: 1 },
                    );
                }
            }
        }

        // Render focus indicator
        if is_focused {
            self.render_focus_indicator(frame, rect);
        }
    }

    fn render_separator(&self, frame: &mut Frame, x: u16, y: u16, height: u16) {
        for row in 0..height {
            frame.render_widget(
                Paragraph::new("│").style(Style::default().fg(Color::DarkGray)),
                Rect { x, y: y + row, width: 1, height: 1 },
            );
        }
    }

    fn style_for_row_type(&self, row_type: RowType, pane: &SourcePane) -> Style {
        match row_type {
            RowType::Addition => Style::default()
                .bg(Color::Rgb(pane.style.add_bg.unwrap_or((0, 60, 0)).0,
                              pane.style.add_bg.unwrap_or((0, 60, 0)).1,
                              pane.style.add_bg.unwrap_or((0, 60, 0)).2)),
            RowType::Deletion => Style::default()
                .bg(Color::Rgb(pane.style.remove_bg.unwrap_or((60, 0, 0)).0,
                              pane.style.remove_bg.unwrap_or((60, 0, 0)).1,
                              pane.style.remove_bg.unwrap_or((60, 0, 0)).2)),
            RowType::Modification => Style::default()
                .bg(Color::Rgb(pane.style.modify_bg.unwrap_or((60, 60, 0)).0,
                              pane.style.modify_bg.unwrap_or((60, 60, 0)).1,
                              pane.style.modify_bg.unwrap_or((60, 60, 0)).2)),
            RowType::Context => Style::default(),
            RowType::HunkHeader => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        }
    }
}
```

---

## Input Routing

### 5. CompositeInputRouter

Routes keyboard/mouse input to the correct source buffer.

```rust
// src/input/composite_router.rs

use crate::model::composite_buffer::CompositeBuffer;
use crate::view::composite_view::CompositeViewState;
use crate::model::event::{BufferId, EditorEvent};

pub struct CompositeInputRouter;

impl CompositeInputRouter {
    /// Route an input event to the appropriate target
    pub fn route_event(
        composite: &CompositeBuffer,
        view_state: &CompositeViewState,
        event: EditorEvent,
    ) -> RoutedEvent {
        let focused_pane = &composite.sources[view_state.focused_pane];

        match event {
            // Navigation events affect the composite view
            EditorEvent::ScrollUp(n) |
            EditorEvent::ScrollDown(n) |
            EditorEvent::PageUp |
            EditorEvent::PageDown |
            EditorEvent::GotoTop |
            EditorEvent::GotoBottom => {
                RoutedEvent::CompositeScroll(event)
            }

            // Pane switching
            EditorEvent::FocusNextPane => {
                RoutedEvent::SwitchPane(Direction::Next)
            }
            EditorEvent::FocusPrevPane => {
                RoutedEvent::SwitchPane(Direction::Prev)
            }

            // Editing events route to focused pane's source buffer
            EditorEvent::Insert(ch) |
            EditorEvent::Delete |
            EditorEvent::Backspace |
            EditorEvent::NewLine => {
                if focused_pane.editable {
                    RoutedEvent::ToSourceBuffer {
                        buffer_id: focused_pane.buffer_id,
                        event,
                        cursor: view_state.pane_cursors[view_state.focused_pane].primary().clone(),
                    }
                } else {
                    RoutedEvent::Blocked("Pane is read-only")
                }
            }

            // Cursor movement within pane
            EditorEvent::CursorUp |
            EditorEvent::CursorDown |
            EditorEvent::CursorLeft |
            EditorEvent::CursorRight => {
                RoutedEvent::PaneCursor {
                    pane_index: view_state.focused_pane,
                    event,
                }
            }

            _ => RoutedEvent::Unhandled(event),
        }
    }

    /// Convert display coordinates to source buffer coordinates
    pub fn display_to_source(
        composite: &CompositeBuffer,
        view_state: &CompositeViewState,
        display_row: usize,
        display_col: usize,
        pane_index: usize,
    ) -> Option<SourceCoordinate> {
        let aligned_row = composite.alignment.rows.get(display_row)?;
        let source_ref = aligned_row.pane_lines.get(pane_index)?.as_ref()?;

        Some(SourceCoordinate {
            buffer_id: composite.sources[pane_index].buffer_id,
            byte_offset: source_ref.byte_range.start + display_col,
            line: source_ref.line,
            column: display_col,
        })
    }
}

#[derive(Debug)]
pub enum RoutedEvent {
    /// Event affects composite view scrolling
    CompositeScroll(EditorEvent),
    /// Switch focus to another pane
    SwitchPane(Direction),
    /// Route to a source buffer
    ToSourceBuffer {
        buffer_id: BufferId,
        event: EditorEvent,
        cursor: Cursor,
    },
    /// Cursor movement within a pane
    PaneCursor {
        pane_index: usize,
        event: EditorEvent,
    },
    /// Event was blocked (e.g., editing read-only pane)
    Blocked(&'static str),
    /// Event not handled by composite
    Unhandled(EditorEvent),
}

#[derive(Debug)]
pub struct SourceCoordinate {
    pub buffer_id: BufferId,
    pub byte_offset: usize,
    pub line: usize,
    pub column: usize,
}
```

---

## Plugin API

### 6. Plugin Commands

TypeScript API for plugins to create composite buffers.

```typescript
// plugins/lib/fresh.d.ts additions

interface CompositeBufferOptions {
    name: string;
    mode: string;
    layout: CompositeLayout;
    sources: SourcePaneConfig[];
    alignment?: AlignmentConfig;
}

interface CompositeLayout {
    type: 'side-by-side' | 'stacked' | 'unified';
    ratios?: number[];           // For side-by-side
    showSeparator?: boolean;     // For side-by-side
    spacing?: number;            // For stacked
}

interface SourcePaneConfig {
    bufferId: number;
    label: string;
    editable: boolean;
    style?: PaneStyleConfig;
    range?: { start: number; end: number };
}

interface PaneStyleConfig {
    addBg?: [number, number, number];
    removeBg?: [number, number, number];
    modifyBg?: [number, number, number];
    gutterStyle?: 'line-numbers' | 'diff-markers' | 'both' | 'none';
}

interface AlignmentConfig {
    type: 'diff';
    hunks: DiffHunk[];
}

interface DiffHunk {
    oldStart: number;
    oldCount: number;
    newStart: number;
    newCount: number;
}

interface Editor {
    // ... existing methods ...

    /**
     * Create a composite buffer that displays multiple sources
     */
    createCompositeBuffer(options: CompositeBufferOptions): Promise<{
        bufferId: number;
        splitId: number;
    }>;

    /**
     * Update the alignment for a composite buffer (e.g., after source edit)
     */
    updateCompositeAlignment(bufferId: number, alignment: AlignmentConfig): void;

    /**
     * Set which pane has focus in a composite buffer
     */
    setCompositeFocus(bufferId: number, paneIndex: number): void;

    /**
     * Get current composite buffer state
     */
    getCompositeState(bufferId: number): CompositeState | null;
}

interface CompositeState {
    focusedPane: number;
    scrollRow: number;
    paneCount: number;
}
```

---

## Diff View Implementation

### 7. Side-by-Side Diff Plugin

Complete implementation for the side-by-side diff feature.

```typescript
// plugins/diff_view.ts

/// <reference path="./lib/fresh.d.ts" />

interface DiffViewState {
    compositeBufferId: number | null;
    oldBufferId: number | null;
    newBufferId: number | null;
    hunks: DiffHunk[];
}

const state: DiffViewState = {
    compositeBufferId: null,
    oldBufferId: null,
    newBufferId: null,
    hunks: [],
};

/**
 * Open side-by-side diff view for a file against HEAD
 */
globalThis.open_side_by_side_diff = async (filePath?: string) => {
    const path = filePath || editor.getActiveBufferPath();
    if (!path) {
        editor.setStatus("No file to diff");
        return;
    }

    // Get HEAD version
    const gitShow = await editor.spawnProcess("git", ["show", `HEAD:${path}`]);
    if (gitShow.exit_code !== 0) {
        editor.setStatus("Failed to get HEAD version");
        return;
    }
    const oldContent = gitShow.stdout;

    // Get working copy content
    const newContent = editor.getBufferText(editor.getActiveBufferId());

    // Parse diff hunks
    const diffResult = await editor.spawnProcess("git", ["diff", "HEAD", "--", path]);
    state.hunks = parseDiffHunks(diffResult.stdout);

    // Create virtual buffer for OLD version
    const oldBufferResult = await editor.createVirtualBuffer({
        name: `[OLD] ${path}`,
        mode: "readonly",
        entries: [{ text: oldContent, properties: {} }],
        read_only: true,
        editing_disabled: true,
    });
    state.oldBufferId = oldBufferResult;

    // Get NEW buffer (the actual file)
    const newBufferId = editor.findBufferByPath(path) || editor.getActiveBufferId();
    state.newBufferId = newBufferId;

    // Create composite buffer
    const result = await editor.createCompositeBuffer({
        name: `Diff: ${path}`,
        mode: "diff-view",
        layout: {
            type: 'side-by-side',
            ratios: [0.5, 0.5],
            showSeparator: true,
        },
        sources: [
            {
                bufferId: state.oldBufferId,
                label: "OLD (HEAD)",
                editable: false,
                style: {
                    removeBg: [80, 30, 30],
                    gutterStyle: 'both',
                },
            },
            {
                bufferId: newBufferId,
                label: "NEW (Working)",
                editable: true,
                style: {
                    addBg: [30, 80, 30],
                    modifyBg: [80, 80, 30],
                    gutterStyle: 'both',
                },
            },
        ],
        alignment: {
            type: 'diff',
            hunks: state.hunks,
        },
    });

    state.compositeBufferId = result.bufferId;
    editor.setStatus(`Diff view opened: ${state.hunks.length} hunks`);
};

/**
 * Parse git diff output into hunks
 */
function parseDiffHunks(diffOutput: string): DiffHunk[] {
    const hunks: DiffHunk[] = [];
    const lines = diffOutput.split('\n');

    for (const line of lines) {
        const match = line.match(/@@ -(\d+),?(\d*) \+(\d+),?(\d*) @@/);
        if (match) {
            hunks.push({
                oldStart: parseInt(match[1]) - 1,  // 0-indexed
                oldCount: parseInt(match[2] || '1'),
                newStart: parseInt(match[3]) - 1,  // 0-indexed
                newCount: parseInt(match[4] || '1'),
            });
        }
    }

    return hunks;
}

/**
 * Navigate to next hunk
 */
globalThis.diff_next_hunk = () => {
    if (!state.compositeBufferId) return;

    const compState = editor.getCompositeState(state.compositeBufferId);
    if (!compState) return;

    // Find next hunk after current scroll position
    // (implementation depends on alignment row types)
    editor.executeAction("composite_scroll_to_next_hunk");
};

/**
 * Navigate to previous hunk
 */
globalThis.diff_prev_hunk = () => {
    if (!state.compositeBufferId) return;
    editor.executeAction("composite_scroll_to_prev_hunk");
};

/**
 * Switch focus between OLD and NEW panes
 */
globalThis.diff_switch_pane = () => {
    if (!state.compositeBufferId) return;

    const compState = editor.getCompositeState(state.compositeBufferId);
    if (!compState) return;

    const nextPane = (compState.focusedPane + 1) % compState.paneCount;
    editor.setCompositeFocus(state.compositeBufferId, nextPane);
};

/**
 * Accept change from OLD to NEW at current position
 */
globalThis.diff_accept_change = async () => {
    // Copy content from OLD pane to NEW pane at current hunk
    // Implementation uses coordinate mapping
};

/**
 * Reject change (revert NEW to OLD) at current position
 */
globalThis.diff_reject_change = async () => {
    // Copy content from NEW pane back to OLD version
};

// Register keybindings
editor.registerMode("diff-view", {
    keybindings: {
        "n": "diff_next_hunk",
        "p": "diff_prev_hunk",
        "Tab": "diff_switch_pane",
        "a": "diff_accept_change",
        "r": "diff_reject_change",
        "q": "close_buffer",
        "j": "scroll_down",
        "k": "scroll_up",
        "G": "goto_bottom",
        "g": "goto_top",
    },
});

// Register command
editor.registerCommand({
    name: "Side-by-Side Diff",
    action: "open_side_by_side_diff",
    category: "Git",
});
```

---

## Integration Points

### 8. Editor State Updates

Changes to the main Editor struct.

```rust
// src/app/mod.rs additions

use crate::model::composite_buffer::CompositeBuffer;
use crate::view::composite_view::CompositeViewState;

pub struct Editor {
    // ... existing fields ...

    /// Composite buffers (separate from regular buffers)
    pub composite_buffers: HashMap<BufferId, CompositeBuffer>,

    /// View state for composite buffers (per split)
    pub composite_view_states: HashMap<(SplitId, BufferId), CompositeViewState>,
}

impl Editor {
    /// Check if a buffer is composite
    pub fn is_composite_buffer(&self, buffer_id: BufferId) -> bool {
        self.composite_buffers.contains_key(&buffer_id)
    }

    /// Get composite buffer
    pub fn get_composite(&self, buffer_id: BufferId) -> Option<&CompositeBuffer> {
        self.composite_buffers.get(&buffer_id)
    }

    /// Get or create composite view state
    pub fn get_composite_view_state(
        &mut self,
        split_id: SplitId,
        buffer_id: BufferId,
    ) -> &mut CompositeViewState {
        self.composite_view_states
            .entry((split_id, buffer_id))
            .or_insert_with(|| CompositeViewState::new(buffer_id))
    }
}
```

### 9. Render Loop Integration

```rust
// src/app/render.rs additions

impl Editor {
    fn render_split_content(&mut self, split_id: SplitId, buffer_id: BufferId, rect: Rect) {
        if self.is_composite_buffer(buffer_id) {
            self.render_composite_buffer(split_id, buffer_id, rect);
        } else {
            self.render_regular_buffer(split_id, buffer_id, rect);
        }
    }

    fn render_composite_buffer(&mut self, split_id: SplitId, buffer_id: BufferId, rect: Rect) {
        let composite = self.composite_buffers.get(&buffer_id).unwrap();
        let view_state = self.get_composite_view_state(split_id, buffer_id);

        let renderer = CompositeRenderer::new(composite, view_state, &self.buffers);
        renderer.render(&mut self.frame, rect);
    }
}
```

---

## Key Design Decisions

### Answers to Open Questions

1. **Line Numbers**: Show both left and right line numbers in a combined gutter (e.g., `42│38`)
2. **Cursor**: One cursor per pane; only focused pane shows active cursor
3. **Selection**: Selection cannot span across panes; each pane has independent selection
4. **Editing**: Focused pane can be edited if `editable: true`; edits route to source buffer
5. **Syntax Highlighting**: Each pane uses its source buffer's syntax highlighting
6. **Performance**: Alignment computed once per diff; only visible rows rendered

### Why This Design

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| Separate CompositeBuffer type | New variant, not extending TextBuffer | Clean separation; composites have fundamentally different behavior |
| LineAlignment struct | Pre-computed alignment | O(1) lookup during render; avoids per-frame diff computation |
| Input routing | Focus-based with coordinate mapping | Natural editing UX; clear which pane receives input |
| Single scroll position | Unified scroll with alignment | Both sides always show corresponding content |
| Source buffer references | BufferId only, not embedded content | Live updates when source changes; no duplication |

---

## Summary

This architecture provides:

1. **Single Tab Display**: CompositeBuffer appears as one tab in the tab bar
2. **Multi-Source Rendering**: Side-by-side, stacked, or unified layouts
3. **Line Alignment**: Pixel-perfect diff view with padding for mismatched line counts
4. **Editing Support**: Changes route to source buffers with proper coordinate mapping
5. **Scroll Synchronization**: Built into the renderer, no external sync needed
6. **Plugin API**: Full control for plugins to create diff/merge views
7. **Extensibility**: Same architecture supports 3-way merge, notebook cells, etc.
