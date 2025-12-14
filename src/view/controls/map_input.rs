//! Map/Dictionary control for editing key-value pairs
//!
//! Renders as an expandable list of entries:
//! ```text
//! Languages:
//!   ▶ rust
//!   ▶ python
//!   ▼ javascript (expanded)
//!       tab_size: 2
//!       auto_indent: [x]
//!   [+ Add entry...]
//! ```

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::FocusState;

/// State for a map/dictionary control
#[derive(Debug, Clone)]
pub struct MapState {
    /// Map entries as (key, value) pairs where value is JSON
    pub entries: Vec<(String, serde_json::Value)>,
    /// Currently focused entry index (None = add-new field)
    pub focused_entry: Option<usize>,
    /// Text in the "add new" key field
    pub new_key_text: String,
    /// Cursor position in the new key field
    pub cursor: usize,
    /// Label for this map
    pub label: String,
    /// Focus state
    pub focus: FocusState,
    /// Expanded entry indices (for viewing/editing nested values)
    pub expanded: Vec<usize>,
    /// Schema for value type (for creating new entries)
    pub value_schema: Option<Box<crate::view::settings::schema::SettingSchema>>,
}

impl MapState {
    /// Create a new map state
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            entries: Vec::new(),
            focused_entry: None,
            new_key_text: String::new(),
            cursor: 0,
            label: label.into(),
            focus: FocusState::Normal,
            expanded: Vec::new(),
            value_schema: None,
        }
    }

    /// Set the entries from JSON value
    pub fn with_entries(mut self, value: &serde_json::Value) -> Self {
        if let Some(obj) = value.as_object() {
            self.entries = obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            // Sort by key for consistent ordering
            self.entries.sort_by(|a, b| a.0.cmp(&b.0));
        }
        self
    }

    /// Set the value schema for creating new entries
    pub fn with_value_schema(
        mut self,
        schema: crate::view::settings::schema::SettingSchema,
    ) -> Self {
        self.value_schema = Some(Box::new(schema));
        self
    }

    /// Set the focus state
    pub fn with_focus(mut self, focus: FocusState) -> Self {
        self.focus = focus;
        self
    }

    /// Add a new entry with the given key and default value
    pub fn add_entry(&mut self, key: String, value: serde_json::Value) {
        if self.focus == FocusState::Disabled || key.is_empty() {
            return;
        }
        // Check for duplicate key
        if self.entries.iter().any(|(k, _)| k == &key) {
            return;
        }
        self.entries.push((key, value));
        self.entries.sort_by(|a, b| a.0.cmp(&b.0));
    }

    /// Add entry from the new_key_text field with default value
    pub fn add_entry_from_input(&mut self) {
        if self.new_key_text.is_empty() {
            return;
        }
        let key = std::mem::take(&mut self.new_key_text);
        self.cursor = 0;
        // Use an empty object as default value
        self.add_entry(key, serde_json::json!({}));
    }

    /// Remove an entry by index
    pub fn remove_entry(&mut self, index: usize) {
        if self.focus == FocusState::Disabled || index >= self.entries.len() {
            return;
        }
        self.entries.remove(index);
        // Adjust focused_entry if needed
        if let Some(focused) = self.focused_entry {
            if focused >= self.entries.len() {
                self.focused_entry = if self.entries.is_empty() {
                    None
                } else {
                    Some(self.entries.len() - 1)
                };
            }
        }
        // Remove from expanded list
        self.expanded.retain(|&idx| idx != index);
        // Adjust expanded indices
        self.expanded = self
            .expanded
            .iter()
            .map(|&idx| if idx > index { idx - 1 } else { idx })
            .collect();
    }

    /// Focus on an entry
    pub fn focus_entry(&mut self, index: usize) {
        if index < self.entries.len() {
            self.focused_entry = Some(index);
        }
    }

    /// Focus on the new entry field
    pub fn focus_new_entry(&mut self) {
        self.focused_entry = None;
        self.cursor = self.new_key_text.len();
    }

    /// Toggle expansion of an entry
    pub fn toggle_expand(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        if let Some(pos) = self.expanded.iter().position(|&i| i == index) {
            self.expanded.remove(pos);
        } else {
            self.expanded.push(index);
        }
    }

    /// Check if an entry is expanded
    pub fn is_expanded(&self, index: usize) -> bool {
        self.expanded.contains(&index)
    }

    /// Insert a character in the new key field
    pub fn insert(&mut self, c: char) {
        if self.focus == FocusState::Disabled || self.focused_entry.is_some() {
            return;
        }
        self.new_key_text.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Backspace in the new key field
    pub fn backspace(&mut self) {
        if self.focus == FocusState::Disabled || self.focused_entry.is_some() || self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.new_key_text.remove(self.cursor);
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self) {
        if self.cursor < self.new_key_text.len() {
            self.cursor += 1;
        }
    }

    /// Move focus to previous entry
    pub fn focus_prev(&mut self) {
        match self.focused_entry {
            Some(0) => {} // Stay at first entry
            Some(idx) => self.focused_entry = Some(idx - 1),
            None if !self.entries.is_empty() => {
                self.focused_entry = Some(self.entries.len() - 1);
            }
            None => {}
        }
    }

    /// Move focus to next entry
    pub fn focus_next(&mut self) {
        match self.focused_entry {
            Some(idx) if idx + 1 < self.entries.len() => {
                self.focused_entry = Some(idx + 1);
            }
            Some(_) => {
                self.focused_entry = None;
                self.cursor = self.new_key_text.len();
            }
            None => {}
        }
    }

    /// Get the number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the map is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Convert entries to JSON value
    pub fn to_value(&self) -> serde_json::Value {
        let map: serde_json::Map<String, serde_json::Value> = self
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        serde_json::Value::Object(map)
    }
}

/// Colors for the map control
#[derive(Debug, Clone, Copy)]
pub struct MapColors {
    pub label: Color,
    pub key: Color,
    pub value_preview: Color,
    pub border: Color,
    pub remove_button: Color,
    pub add_button: Color,
    pub focused: Color,
    pub cursor: Color,
    pub disabled: Color,
    pub expand_arrow: Color,
}

impl Default for MapColors {
    fn default() -> Self {
        Self {
            label: Color::White,
            key: Color::Cyan,
            value_preview: Color::Gray,
            border: Color::Gray,
            remove_button: Color::Red,
            add_button: Color::Green,
            focused: Color::Yellow,
            cursor: Color::Yellow,
            disabled: Color::DarkGray,
            expand_arrow: Color::White,
        }
    }
}

impl MapColors {
    pub fn from_theme(theme: &crate::view::theme::Theme) -> Self {
        Self {
            label: theme.editor_fg,
            key: theme.menu_highlight_fg,
            value_preview: theme.line_number_fg,
            border: theme.line_number_fg,
            remove_button: theme.diagnostic_error_fg,
            add_button: theme.diagnostic_info_fg,
            focused: theme.selection_bg,
            cursor: theme.cursor,
            disabled: theme.line_number_fg,
            expand_arrow: theme.editor_fg,
        }
    }
}

/// Layout information for hit testing
#[derive(Debug, Clone)]
pub struct MapLayout {
    pub full_area: Rect,
    pub entry_areas: Vec<MapEntryLayout>,
    pub add_row_area: Option<Rect>,
}

#[derive(Debug, Clone)]
pub struct MapEntryLayout {
    pub index: usize,
    pub row_area: Rect,
    pub expand_area: Rect,
    pub key_area: Rect,
    pub remove_area: Rect,
}

/// Render a map control
pub fn render_map(
    frame: &mut Frame,
    area: Rect,
    state: &MapState,
    colors: &MapColors,
    key_width: u16,
) -> MapLayout {
    let empty_layout = MapLayout {
        full_area: area,
        entry_areas: Vec::new(),
        add_row_area: None,
    };

    if area.height == 0 || area.width < 15 {
        return empty_layout;
    }

    let label_color = match state.focus {
        FocusState::Focused => colors.focused,
        FocusState::Hovered => colors.focused,
        FocusState::Disabled => colors.disabled,
        FocusState::Normal => colors.label,
    };

    // Render label
    let label_line = Line::from(vec![
        Span::styled(&state.label, Style::default().fg(label_color)),
        Span::raw(":"),
    ]);
    frame.render_widget(
        Paragraph::new(label_line),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let mut entry_areas = Vec::new();
    let mut y = area.y + 1;
    let indent = 2u16;
    let actual_key_width = key_width.min(area.width.saturating_sub(indent + 8));

    // Render entries
    for (idx, (key, value)) in state.entries.iter().enumerate() {
        if y >= area.y + area.height {
            break;
        }

        let is_focused = state.focused_entry == Some(idx) && state.focus == FocusState::Focused;
        let is_expanded = state.is_expanded(idx);

        let arrow = if is_expanded { "▼" } else { "▶" };
        let arrow_color = if is_focused {
            colors.focused
        } else {
            colors.expand_arrow
        };
        let key_color = if is_focused {
            colors.focused
        } else {
            colors.key
        };

        // Value preview (truncated)
        let value_preview = format_value_preview(value, 20);

        let line = Line::from(vec![
            Span::raw(" ".repeat(indent as usize)),
            Span::styled(arrow, Style::default().fg(arrow_color)),
            Span::raw(" "),
            Span::styled(
                format!("{:width$}", key, width = actual_key_width as usize),
                Style::default().fg(key_color),
            ),
            Span::raw(" "),
            Span::styled(value_preview, Style::default().fg(colors.value_preview)),
            Span::raw(" "),
            Span::styled("[x]", Style::default().fg(colors.remove_button)),
        ]);

        let row_area = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(Paragraph::new(line), row_area);

        entry_areas.push(MapEntryLayout {
            index: idx,
            row_area,
            expand_area: Rect::new(area.x + indent, y, 1, 1),
            key_area: Rect::new(area.x + indent + 2, y, actual_key_width, 1),
            remove_area: Rect::new(area.x + indent + 2 + actual_key_width + 22, y, 3, 1),
        });

        y += 1;

        // If expanded, show nested values (simplified view)
        if is_expanded && y < area.y + area.height {
            if let Some(obj) = value.as_object() {
                for (nested_key, nested_value) in obj.iter().take(5) {
                    if y >= area.y + area.height {
                        break;
                    }
                    let nested_preview = format_value_preview(nested_value, 15);
                    let nested_line = Line::from(vec![
                        Span::raw(" ".repeat((indent + 4) as usize)),
                        Span::styled(
                            format!("{}: ", nested_key),
                            Style::default().fg(colors.label),
                        ),
                        Span::styled(nested_preview, Style::default().fg(colors.value_preview)),
                    ]);
                    frame.render_widget(
                        Paragraph::new(nested_line),
                        Rect::new(area.x, y, area.width, 1),
                    );
                    y += 1;
                }
                if obj.len() > 5 && y < area.y + area.height {
                    let more_line = Line::from(Span::styled(
                        format!(
                            "{}... and {} more",
                            " ".repeat((indent + 4) as usize),
                            obj.len() - 5
                        ),
                        Style::default()
                            .fg(colors.value_preview)
                            .add_modifier(Modifier::ITALIC),
                    ));
                    frame.render_widget(
                        Paragraph::new(more_line),
                        Rect::new(area.x, y, area.width, 1),
                    );
                    y += 1;
                }
            }
        }
    }

    // Render "add new" row
    let add_row_area = if y < area.y + area.height {
        let is_focused = state.focused_entry.is_none() && state.focus == FocusState::Focused;
        let (border_color, text_color) = if is_focused {
            (colors.focused, colors.label)
        } else if state.focus == FocusState::Disabled {
            (colors.disabled, colors.disabled)
        } else {
            (colors.border, colors.label)
        };

        let inner_width = actual_key_width.saturating_sub(2) as usize;
        let visible: String = state.new_key_text.chars().take(inner_width).collect();
        let padded = format!("{:width$}", visible, width = inner_width);

        let line = Line::from(vec![
            Span::raw(" ".repeat(indent as usize)),
            Span::styled("[", Style::default().fg(border_color)),
            Span::styled(padded, Style::default().fg(text_color)),
            Span::styled("]", Style::default().fg(border_color)),
            Span::raw(" "),
            Span::styled("[+]", Style::default().fg(colors.add_button)),
            Span::raw(" Add entry..."),
        ]);

        let row_area = Rect::new(area.x, y, area.width, 1);
        frame.render_widget(Paragraph::new(line), row_area);

        // Render cursor if focused
        if is_focused && state.cursor <= inner_width {
            let cursor_x = area.x + indent + 1 + state.cursor as u16;
            let cursor_char = state.new_key_text.chars().nth(state.cursor).unwrap_or(' ');
            let cursor_area = Rect::new(cursor_x, y, 1, 1);
            let cursor_span = Span::styled(
                cursor_char.to_string(),
                Style::default()
                    .fg(colors.cursor)
                    .add_modifier(Modifier::REVERSED),
            );
            frame.render_widget(Paragraph::new(Line::from(vec![cursor_span])), cursor_area);
        }

        Some(row_area)
    } else {
        None
    };

    MapLayout {
        full_area: area,
        entry_areas,
        add_row_area,
    }
}

/// Format a JSON value as a short preview string
fn format_value_preview(value: &serde_json::Value, max_len: usize) -> String {
    let s = match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => format!("\"{}\"", s),
        serde_json::Value::Array(arr) => format!("[{} items]", arr.len()),
        serde_json::Value::Object(obj) => format!("{{{} fields}}", obj.len()),
    };
    if s.len() > max_len {
        format!("{}...", &s[..max_len - 3])
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_state_new() {
        let state = MapState::new("Test");
        assert_eq!(state.label, "Test");
        assert!(state.entries.is_empty());
        assert!(state.focused_entry.is_none());
    }

    #[test]
    fn test_map_state_add_entry() {
        let mut state = MapState::new("Test");
        state.add_entry("key1".to_string(), serde_json::json!({"foo": "bar"}));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].0, "key1");
    }

    #[test]
    fn test_map_state_remove_entry() {
        let mut state = MapState::new("Test");
        state.add_entry("a".to_string(), serde_json::json!({}));
        state.add_entry("b".to_string(), serde_json::json!({}));
        state.remove_entry(0);
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].0, "b");
    }

    #[test]
    fn test_map_state_navigation() {
        let mut state = MapState::new("Test").with_focus(FocusState::Focused);
        state.add_entry("a".to_string(), serde_json::json!({}));
        state.add_entry("b".to_string(), serde_json::json!({}));

        // Start at add-new
        assert!(state.focused_entry.is_none());

        // Go to last entry
        state.focus_prev();
        assert_eq!(state.focused_entry, Some(1));

        // Go to first entry
        state.focus_prev();
        assert_eq!(state.focused_entry, Some(0));

        // Go forward
        state.focus_next();
        assert_eq!(state.focused_entry, Some(1));

        // Go to add-new
        state.focus_next();
        assert!(state.focused_entry.is_none());
    }

    #[test]
    fn test_map_state_expand() {
        let mut state = MapState::new("Test");
        state.add_entry("key1".to_string(), serde_json::json!({}));

        assert!(!state.is_expanded(0));
        state.toggle_expand(0);
        assert!(state.is_expanded(0));
        state.toggle_expand(0);
        assert!(!state.is_expanded(0));
    }
}
