//! Multi-cursor operations for adding cursors at various positions

use crate::model::cursor::Cursor;
use crate::state::EditorState;

/// Result of attempting to add a cursor
pub enum AddCursorResult {
    /// Cursor was added successfully
    Success {
        cursor: Cursor,
        total_cursors: usize,
    },
    /// Operation failed with a message
    Failed { message: String },
}

/// Information about a cursor's position within its line
struct CursorLineInfo {
    /// Byte offset of the line start
    line_start: usize,
    /// Column offset from line start
    col_offset: usize,
}

/// Get line info for a cursor position
fn get_cursor_line_info(state: &mut EditorState, position: usize) -> Option<CursorLineInfo> {
    let mut iter = state.buffer.line_iterator(position, 80);
    let (line_start, _) = iter.next()?;
    Some(CursorLineInfo {
        line_start,
        col_offset: position.saturating_sub(line_start),
    })
}

/// Calculate cursor position on a line, clamping to line length (excluding newline)
fn cursor_position_on_line(line_start: usize, line_content: &str, target_col: usize) -> usize {
    let line_len = line_content.trim_end_matches('\n').len();
    line_start + target_col.min(line_len)
}

/// Create a successful AddCursorResult
fn success_result(cursor: Cursor, state: &EditorState) -> AddCursorResult {
    AddCursorResult::Success {
        cursor,
        total_cursors: state.cursors.iter().count() + 1,
    }
}

/// Adjust cursor position if it's on a newline character
/// Returns position + 1 if cursor is at a newline, otherwise returns position unchanged
fn adjust_position_for_newline(state: &mut EditorState, position: usize) -> usize {
    if position < state.buffer.len() {
        if let Ok(byte_at_cursor) = state.buffer.get_text_range_mut(position, 1) {
            if byte_at_cursor.first() == Some(&b'\n') {
                return position + 1;
            }
        }
    }
    position
}

/// Add a cursor at the next occurrence of the selected text
/// If no selection, returns Failed
pub fn add_cursor_at_next_match(state: &mut EditorState) -> AddCursorResult {
    // Get the selected text from the primary cursor
    let primary = state.cursors.primary();
    let selection_range = match primary.selection_range() {
        Some(range) => range,
        None => {
            return AddCursorResult::Failed {
                message: "No selection to match".to_string(),
            }
        }
    };

    // Extract the selected text
    let pattern = state.get_text_range(selection_range.start, selection_range.end);

    // Find the next occurrence after the current selection
    let search_start = selection_range.end;
    let match_pos = match state.buffer.find_next(&pattern, search_start) {
        Some(pos) => pos,
        None => {
            return AddCursorResult::Failed {
                message: "No more matches".to_string(),
            }
        }
    };

    // Create a new cursor at the match position with selection
    let new_cursor = Cursor::with_selection(match_pos, match_pos + pattern.len());
    success_result(new_cursor, state)
}

/// Add a cursor above the primary cursor at the same column
pub fn add_cursor_above(state: &mut EditorState) -> AddCursorResult {
    let position = state.cursors.primary().position;

    // Adjust position if cursor is at a newline character
    // This handles cases where add_cursor_above/below places cursor at same column
    let adjusted_position = adjust_position_for_newline(state, position);

    // Get current line info
    let Some(info) = get_cursor_line_info(state, adjusted_position) else {
        return AddCursorResult::Failed {
            message: "Unable to find current line".to_string(),
        };
    };

    // Check if we're on the first line
    if info.line_start == 0 {
        return AddCursorResult::Failed {
            message: "Already at first line".to_string(),
        };
    }

    // Navigate to previous line using iterator
    let mut iter = state.buffer.line_iterator(adjusted_position, 80);
    iter.next(); // Consume current line
    iter.prev(); // Move back to current line

    // Get the previous line
    if let Some((prev_line_start, prev_line_content)) = iter.prev() {
        let new_pos = cursor_position_on_line(prev_line_start, &prev_line_content, info.col_offset);
        success_result(Cursor::new(new_pos), state)
    } else {
        AddCursorResult::Failed {
            message: "Already at first line".to_string(),
        }
    }
}

/// Add a cursor below the primary cursor at the same column
pub fn add_cursor_below(state: &mut EditorState) -> AddCursorResult {
    let position = state.cursors.primary().position;

    // Get current line info
    let Some(info) = get_cursor_line_info(state, position) else {
        return AddCursorResult::Failed {
            message: "Unable to find current line".to_string(),
        };
    };

    // Navigate to next line using iterator
    let mut iter = state.buffer.line_iterator(position, 80);
    iter.next(); // Consume current line

    // Get next line
    if let Some((next_line_start, next_line_content)) = iter.next() {
        let new_pos = cursor_position_on_line(next_line_start, &next_line_content, info.col_offset);
        success_result(Cursor::new(new_pos), state)
    } else {
        AddCursorResult::Failed {
            message: "Already at last line".to_string(),
        }
    }
}
