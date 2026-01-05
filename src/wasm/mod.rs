//! WASM browser build module for Fresh editor
//!
//! This module provides the entry point for running Fresh in a web browser
//! using WebAssembly. It uses Ratzilla for browser-based terminal rendering.
//!
//! Uses the native PieceTree-based Buffer from model::buffer for maximum
//! code sharing with the native editor.

pub mod event_adapter;
pub mod fs_backend;

use ratzilla::ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[allow(unused_imports)]
use crate::config::Config;
use crate::model::buffer::TextBuffer as NativeBuffer;
use crate::primitives::syntect_highlighter::SyntectHighlighter;
use crate::view::text_content::TextContentProvider;
use crate::view::theme::Theme;
use event_adapter::{WasmKeyCode, WasmKeyEvent, WasmKeyModifiers, WasmMouseButton, WasmMouseEvent, WasmMouseEventKind};
#[allow(unused_imports)]
use fs_backend::WasmFsBackend;

/// Editor buffer that wraps the native PieceTree-based Buffer with cursor tracking
///
/// This provides a high-level editing API (insert_char, delete_char, cursor movement)
/// on top of the native Buffer's byte-offset operations.
struct EditorBuffer {
    /// The native PieceTree-based buffer (shared with native editor)
    buffer: NativeBuffer,
    /// Cursor position as (line, column) - column is character-based, not byte-based
    cursor: (usize, usize),
    /// Viewport top line for scrolling
    viewport_top: usize,
    /// Whether the buffer has been modified
    modified: bool,
    /// File name (if any)
    filename: Option<String>,
}

impl EditorBuffer {
    fn new() -> Self {
        Self {
            buffer: NativeBuffer::empty(),
            cursor: (0, 0),
            viewport_top: 0,
            modified: false,
            filename: None,
        }
    }

    fn from_content(content: &str, filename: Option<String>) -> Self {
        let buffer = NativeBuffer::from_bytes(content.as_bytes().to_vec());
        Self {
            buffer,
            cursor: (0, 0),
            viewport_top: 0,
            modified: false,
            filename,
        }
    }

    /// Convert cursor (line, column) to byte offset in the buffer
    fn cursor_to_offset(&self) -> usize {
        let (line, col) = self.cursor;

        // Get the byte offset of the start of the current line
        let line_start = self.buffer.line_start_offset(line).unwrap_or(0);

        // Get the line content to convert column to byte offset
        if let Some(text_line) = TextContentProvider::get_line(&self.buffer, line) {
            // Convert character column to byte offset within line
            let byte_col = text_line
                .content
                .char_indices()
                .nth(col)
                .map(|(i, _)| i)
                .unwrap_or(text_line.content.len());
            line_start + byte_col
        } else {
            line_start
        }
    }

    fn insert_char(&mut self, ch: char) {
        let offset = self.cursor_to_offset();
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        self.buffer.insert(offset, s);
        self.cursor.1 += 1;
        self.modified = true;
    }

    fn insert_newline(&mut self) {
        let offset = self.cursor_to_offset();
        self.buffer.insert(offset, "\n");
        self.cursor = (self.cursor.0 + 1, 0);
        self.modified = true;
    }

    fn delete_char(&mut self) {
        let (line, col) = self.cursor;
        if col > 0 {
            // Delete character before cursor on current line
            let offset = self.cursor_to_offset();
            // Need to find the start of the previous character
            if let Some(text_line) = TextContentProvider::get_line(&self.buffer, line) {
                let byte_col_before = text_line
                    .content
                    .char_indices()
                    .nth(col - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let line_start = self.buffer.line_start_offset(line).unwrap_or(0);
                let delete_start = line_start + byte_col_before;
                self.buffer.delete(delete_start..offset);
                self.cursor.1 -= 1;
                self.modified = true;
            }
        } else if line > 0 {
            // Join with previous line (delete the newline at end of previous line)
            if let Some(prev_line) = TextContentProvider::get_line(&self.buffer, line - 1) {
                let prev_line_len = prev_line.content.chars().count();
                let line_start = self.buffer.line_start_offset(line).unwrap_or(0);
                // Delete the newline character before this line
                if line_start > 0 {
                    self.buffer.delete(line_start - 1..line_start);
                    self.cursor = (line - 1, prev_line_len);
                    self.modified = true;
                }
            }
        }
    }

    fn move_cursor(&mut self, dx: i32, dy: i32) {
        let (line, col) = self.cursor;
        let line_count = TextContentProvider::line_count(&self.buffer);

        let new_line = (line as i32 + dy).max(0) as usize;
        let new_line = new_line.min(line_count.saturating_sub(1));

        let line_len = TextContentProvider::get_line(&self.buffer, new_line)
            .map(|l| l.content.chars().count())
            .unwrap_or(0);

        let new_col = if dy != 0 {
            col.min(line_len)
        } else {
            (col as i32 + dx).max(0) as usize
        };
        let new_col = new_col.min(line_len);

        self.cursor = (new_line, new_col);
    }

    fn move_to_line_start(&mut self) {
        self.cursor.1 = 0;
    }

    fn move_to_line_end(&mut self) {
        let (line, _) = self.cursor;
        if let Some(text_line) = TextContentProvider::get_line(&self.buffer, line) {
            self.cursor.1 = text_line.content.chars().count();
        }
    }

    fn ensure_cursor_visible(&mut self, height: usize) {
        let (line, _) = self.cursor;
        if line < self.viewport_top {
            self.viewport_top = line;
        } else if line >= self.viewport_top + height {
            self.viewport_top = line - height + 1;
        }
    }

    /// Get line count (delegate to buffer via TextContentProvider)
    fn line_count(&self) -> usize {
        TextContentProvider::line_count(&self.buffer)
    }

    /// Get content as string (delegate to buffer via TextContentProvider)
    fn content(&self) -> String {
        TextContentProvider::content(&self.buffer)
    }
}

/// WASM Editor state
struct WasmEditorState {
    buffer: EditorBuffer,
    width: u16,
    height: u16,
    status_message: Option<String>,
    /// Shared theme from view module
    theme: Theme,
    /// Syntax highlighter (optional, based on file extension)
    highlighter: Option<SyntectHighlighter>,
}

impl WasmEditorState {
    fn new(width: u16, height: u16) -> Self {
        // Use shared theme from view module - load from config theme name or default to dark
        let theme = Theme::default();
        Self {
            buffer: EditorBuffer::new(),
            width,
            height,
            status_message: Some("Fresh Editor (WASM) - Press Ctrl+Q to quit".to_string()),
            theme,
            highlighter: None,
        }
    }

    fn handle_key(&mut self, event: event_adapter::WasmKeyEvent) -> bool {
        use event_adapter::WasmKeyModifiers;

        match event.code {
            WasmKeyCode::Char('q') if event.modifiers.contains(WasmKeyModifiers::CONTROL) => {
                // Quit - signal to stop
                return false;
            }
            WasmKeyCode::Char('s') if event.modifiers.contains(WasmKeyModifiers::CONTROL) => {
                self.status_message = Some("Save not implemented in WASM demo".to_string());
            }
            WasmKeyCode::Char(c) => {
                self.buffer.insert_char(c);
                self.status_message = None;
            }
            WasmKeyCode::Enter => {
                self.buffer.insert_newline();
                self.status_message = None;
            }
            WasmKeyCode::Backspace => {
                self.buffer.delete_char();
                self.status_message = None;
            }
            WasmKeyCode::Left => {
                self.buffer.move_cursor(-1, 0);
            }
            WasmKeyCode::Right => {
                self.buffer.move_cursor(1, 0);
            }
            WasmKeyCode::Up => {
                self.buffer.move_cursor(0, -1);
            }
            WasmKeyCode::Down => {
                self.buffer.move_cursor(0, 1);
            }
            WasmKeyCode::Home => {
                self.buffer.move_to_line_start();
            }
            WasmKeyCode::End => {
                self.buffer.move_to_line_end();
            }
            WasmKeyCode::PageUp => {
                let height = self.height.saturating_sub(2) as i32;
                self.buffer.move_cursor(0, -height);
            }
            WasmKeyCode::PageDown => {
                let height = self.height.saturating_sub(2) as i32;
                self.buffer.move_cursor(0, height);
            }
            _ => {}
        }
        true
    }

    #[allow(dead_code)]
    fn handle_mouse(&mut self, event: event_adapter::WasmMouseEvent) {
        match event.kind {
            WasmMouseEventKind::Down(_) => {
                // Click to position cursor
                let line = self.buffer.viewport_top + event.row as usize;
                let col = event.column as usize;

                // Clamp to valid range
                let line_count = self.buffer.line_count();
                let line = line.min(line_count.saturating_sub(1));

                let line_len = TextContentProvider::get_line(&self.buffer.buffer, line)
                    .map(|l| l.content.chars().count())
                    .unwrap_or(0);
                let col = col.min(line_len);

                self.buffer.cursor = (line, col);
            }
            WasmMouseEventKind::ScrollDown => {
                self.buffer.move_cursor(0, 3);
            }
            WasmMouseEventKind::ScrollUp => {
                self.buffer.move_cursor(0, -3);
            }
            _ => {}
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        let size = frame.area();
        let height = size.height.saturating_sub(2) as usize; // Leave room for status bar

        // Ensure cursor is visible
        self.buffer.ensure_cursor_visible(height);

        // Create layout: main area + status bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(size);

        // Render the text area with syntax highlighting
        self.render_text_area(frame, chunks[0]);

        // Render status bar
        self.render_status_bar(frame, chunks[1]);
    }

    fn render_text_area(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let height = area.height as usize;
        let line_count = self.buffer.line_count();

        // Render visible lines
        let mut lines: Vec<Line<'_>> = Vec::new();
        for i in 0..height {
            let line_idx = self.buffer.viewport_top + i;
            if line_idx >= line_count {
                // Empty line beyond content
                lines.push(Line::from(Span::styled("~", Style::default().fg(self.theme.syntax_comment))));
            } else if let Some(text_line) = TextContentProvider::get_line(&self.buffer.buffer, line_idx) {
                // Simple rendering without syntax highlighting for now
                // TODO: Add syntax highlighting using SyntectHighlighter::highlight_viewport
                lines.push(Line::from(text_line.content.clone()));
            } else {
                lines.push(Line::from(""));
            }
        }

        let paragraph = Paragraph::new(lines)
            .style(Style::default().fg(self.theme.editor_fg).bg(self.theme.editor_bg));
        frame.render_widget(paragraph, area);

        // Render cursor (simple block cursor)
        let (cursor_line, cursor_col) = self.buffer.cursor;
        if cursor_line >= self.buffer.viewport_top
            && cursor_line < self.buffer.viewport_top + height
        {
            let cursor_y = area.y + (cursor_line - self.buffer.viewport_top) as u16;
            let cursor_x = area.x + cursor_col as u16;
            if cursor_x < area.x + area.width && cursor_y < area.y + area.height {
                frame.set_cursor_position((cursor_x, cursor_y));
            }
        }
    }

    fn render_status_bar(&self, frame: &mut Frame<'_>, area: Rect) {
        let (line, col) = self.buffer.cursor;
        let modified = if self.buffer.modified { "[+]" } else { "" };
        let filename = self
            .buffer
            .filename
            .as_deref()
            .unwrap_or("[No Name]");

        let status_left = format!(" {} {} ", filename, modified);
        let status_right = format!(" {}:{} ", line + 1, col + 1);

        // Message or position
        let message = self
            .status_message
            .as_deref()
            .unwrap_or("");

        let status = format!(
            "{}{}{}{}",
            status_left,
            message,
            " ".repeat(
                area.width
                    .saturating_sub(status_left.len() as u16)
                    .saturating_sub(status_right.len() as u16)
                    .saturating_sub(message.len() as u16) as usize
            ),
            status_right
        );

        // Use inverted colors for status bar
        let status_style = Style::default()
            .fg(self.theme.editor_bg)
            .bg(self.theme.editor_fg);

        let paragraph = Paragraph::new(status).style(status_style);
        frame.render_widget(paragraph, area);
    }
}

/// WASM-exported editor handle
#[wasm_bindgen]
pub struct WasmEditor {
    state: Rc<RefCell<Option<WasmEditorState>>>,
}

#[wasm_bindgen]
impl WasmEditor {
    /// Create a new WASM editor instance
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        // Set up panic hook for better error messages
        console_error_panic_hook::set_once();

        Self {
            state: Rc::new(RefCell::new(None)),
        }
    }

    /// Initialize the editor with the given terminal size
    pub fn init(&self, width: u16, height: u16) {
        let mut state = self.state.borrow_mut();
        *state = Some(WasmEditorState::new(width, height));
    }

    /// Load content into the editor
    pub fn load_content(&self, content: &str, filename: Option<String>) {
        let mut state = self.state.borrow_mut();
        if let Some(ref mut s) = *state {
            s.buffer = EditorBuffer::from_content(content, filename.clone());
            // Try to set up syntax highlighter based on filename extension
            if let Some(ref name) = filename {
                let path = std::path::Path::new(name);
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    s.highlighter = SyntectHighlighter::for_extension(ext);
                }
            }
            s.status_message = Some(format!(
                "Loaded: {}",
                filename.as_deref().unwrap_or("[No Name]")
            ));
        }
    }

    /// Get the current content
    pub fn get_content(&self) -> String {
        let state = self.state.borrow();
        if let Some(ref s) = *state {
            s.buffer.content()
        } else {
            String::new()
        }
    }

    /// Handle a key event (returns false if editor should quit)
    pub fn handle_key(
        &self,
        key: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> bool {
        let mut state = self.state.borrow_mut();
        if let Some(ref mut s) = *state {
            // Convert string key to WasmKeyCode
            let code = match key {
                "Enter" => WasmKeyCode::Enter,
                "Tab" => WasmKeyCode::Tab,
                "Backspace" => WasmKeyCode::Backspace,
                "Delete" => WasmKeyCode::Delete,
                "Escape" => WasmKeyCode::Esc,
                "ArrowUp" => WasmKeyCode::Up,
                "ArrowDown" => WasmKeyCode::Down,
                "ArrowLeft" => WasmKeyCode::Left,
                "ArrowRight" => WasmKeyCode::Right,
                "Home" => WasmKeyCode::Home,
                "End" => WasmKeyCode::End,
                "PageUp" => WasmKeyCode::PageUp,
                "PageDown" => WasmKeyCode::PageDown,
                s if s.len() == 1 => WasmKeyCode::Char(s.chars().next().unwrap()),
                _ => return true, // Unknown key, ignore
            };

            let mut modifiers = WasmKeyModifiers::NONE;
            if ctrl {
                modifiers |= WasmKeyModifiers::CONTROL;
            }
            if alt {
                modifiers |= WasmKeyModifiers::ALT;
            }
            if shift {
                modifiers |= WasmKeyModifiers::SHIFT;
            }

            let event = WasmKeyEvent::new(code, modifiers);
            s.handle_key(event)
        } else {
            true
        }
    }

    /// Handle a mouse event
    pub fn handle_mouse(&self, kind: &str, row: u16, column: u16) {
        let mut state = self.state.borrow_mut();
        if let Some(ref mut s) = *state {
            let mouse_kind = match kind {
                "mousedown" => WasmMouseEventKind::Down(WasmMouseButton::Left),
                "mouseup" => WasmMouseEventKind::Up(WasmMouseButton::Left),
                "mousemove" => WasmMouseEventKind::Moved,
                "wheel_up" | "scrollup" => WasmMouseEventKind::ScrollUp,
                "wheel_down" | "scrolldown" => WasmMouseEventKind::ScrollDown,
                _ => return, // Unknown event type
            };

            let event = WasmMouseEvent {
                kind: mouse_kind,
                column,
                row,
                modifiers: WasmKeyModifiers::NONE,
            };
            s.handle_mouse(event);
        }
    }

    /// Resize the editor
    pub fn resize(&self, width: u16, height: u16) {
        let mut state = self.state.borrow_mut();
        if let Some(ref mut s) = *state {
            s.width = width;
            s.height = height;
        }
    }

    /// Check if the buffer has been modified
    pub fn is_modified(&self) -> bool {
        let state = self.state.borrow();
        if let Some(ref s) = *state {
            s.buffer.modified
        } else {
            false
        }
    }
}

impl Default for WasmEditor {
    fn default() -> Self {
        Self::new()
    }
}

/// Main entry point for WASM - starts the editor
#[wasm_bindgen(start)]
pub fn wasm_main() {
    // Set up panic hook for better error messages
    console_error_panic_hook::set_once();

    // Log startup
    web_sys::console::log_1(&"Fresh Editor WASM module loaded".into());
}

// Note: run_editor using Ratzilla's WebRenderer is not currently working
// Use the WasmEditor struct directly from JavaScript instead
