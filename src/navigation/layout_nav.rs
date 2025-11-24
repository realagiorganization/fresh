use crate::cursor::ViewPosition;
use crate::ui::view_pipeline::Layout;
use crate::viewport::Viewport;

/// Move vertically by view lines within a layout, preserving preferred column when provided.
pub fn move_vertical(
    layout: &Layout,
    cursor: &ViewPosition,
    preferred_col: Option<usize>,
    direction: isize,
) -> ViewPosition {
    let current_line = cursor.view_line;
    let target_line = ((current_line as isize) + direction)
        .max(0)
        .min((layout.lines.len().saturating_sub(1)) as isize) as usize;
    let target_col = preferred_col.unwrap_or(cursor.column);
    ViewPosition {
        view_line: target_line,
        column: target_col,
        source_byte: layout.view_position_to_source_byte(target_line, target_col),
    }
}

/// Move horizontally within a view line; clamps to line length.
pub fn move_horizontal(
    layout: &Layout,
    cursor: &ViewPosition,
    direction: isize,
) -> ViewPosition {
    let line_idx = cursor.view_line.min(layout.lines.len().saturating_sub(1));
    let line_len = layout.lines.get(line_idx).map(|l| l.char_mappings.len()).unwrap_or(0);
    let raw_col = (cursor.column as isize + direction).max(0) as usize;
    let target_col = raw_col.min(line_len.saturating_sub(1));
    ViewPosition {
        view_line: line_idx,
        column: target_col,
        source_byte: layout.view_position_to_source_byte(line_idx, target_col),
    }
}

/// Move to the start of the current view line.
pub fn move_line_start(layout: &Layout, cursor: &ViewPosition) -> ViewPosition {
    let line_idx = cursor.view_line.min(layout.lines.len().saturating_sub(1));
    ViewPosition {
        view_line: line_idx,
        column: 0,
        source_byte: layout.view_position_to_source_byte(line_idx, 0),
    }
}

/// Move to the end of the current view line.
pub fn move_line_end(layout: &Layout, cursor: &ViewPosition) -> ViewPosition {
    let line_idx = cursor.view_line.min(layout.lines.len().saturating_sub(1));
    let line_len = layout.lines.get(line_idx).map(|l| l.char_mappings.len()).unwrap_or(0);
    let col = line_len.saturating_sub(1);
    ViewPosition {
        view_line: line_idx,
        column: col,
        source_byte: layout.view_position_to_source_byte(line_idx, col),
    }
}

/// Move by a page (viewport height) in view lines.
pub fn move_page(
    layout: &Layout,
    cursor: &ViewPosition,
    viewport: &Viewport,
    direction: isize,
) -> ViewPosition {
    let page = viewport.visible_line_count().saturating_sub(1);
    let delta = (page as isize) * direction;
    move_vertical(layout, cursor, Some(cursor.column), delta)
}

/// Scroll the viewport by view lines.
pub fn scroll_view(layout: &Layout, viewport: &mut Viewport, line_offset: isize) {
    let max_top = layout.max_top_line(viewport.visible_line_count());
    let target = (viewport.top_view_line as isize + line_offset).max(0) as usize;
    viewport.top_view_line = target.min(max_top);
    if let Some(byte) = layout.get_source_byte_for_line(viewport.top_view_line) {
        viewport.top_byte = byte;
        viewport.anchor_byte = byte;
    }
}

/// Move to the start of the previous word in view coordinates.
/// Note: Requires access to buffer context; will be called from action_convert with buffer access.
pub fn move_word_left(layout: &Layout, cursor: &ViewPosition, buffer: &crate::text_buffer::Buffer) -> ViewPosition {
    crate::word_navigation::find_word_start_left_view(layout, cursor, buffer)
}

/// Move to the start of the next word in view coordinates.
/// Note: Requires access to buffer context; will be called from action_convert with buffer access.
pub fn move_word_right(layout: &Layout, cursor: &ViewPosition, buffer: &crate::text_buffer::Buffer) -> ViewPosition {
    crate::word_navigation::find_word_start_right_view(layout, cursor, buffer)
}
