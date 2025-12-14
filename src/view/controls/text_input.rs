//! Single-line text input control
//!
//! Renders as: `Label: [text content     ]`

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::FocusState;

/// State for a text input control
#[derive(Debug, Clone)]
pub struct TextInputState {
    /// Current text value
    pub value: String,
    /// Cursor position (character index)
    pub cursor: usize,
    /// Label displayed before the input
    pub label: String,
    /// Placeholder text when empty
    pub placeholder: String,
    /// Focus state
    pub focus: FocusState,
}

impl TextInputState {
    /// Create a new text input state
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            label: label.into(),
            placeholder: String::new(),
            focus: FocusState::Normal,
        }
    }

    /// Set the initial value
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self.cursor = self.value.len();
        self
    }

    /// Set the placeholder text
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set the focus state
    pub fn with_focus(mut self, focus: FocusState) -> Self {
        self.focus = focus;
        self
    }

    /// Insert a character at the cursor position
    pub fn insert(&mut self, c: char) {
        if self.focus == FocusState::Disabled {
            return;
        }
        self.value.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Delete the character before the cursor (backspace)
    pub fn backspace(&mut self) {
        if self.focus == FocusState::Disabled || self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.value.remove(self.cursor);
    }

    /// Delete the character at the cursor (delete)
    pub fn delete(&mut self) {
        if self.focus == FocusState::Disabled || self.cursor >= self.value.len() {
            return;
        }
        self.value.remove(self.cursor);
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self) {
        if self.cursor < self.value.len() {
            self.cursor += 1;
        }
    }

    /// Move cursor to start
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end
    pub fn move_end(&mut self) {
        self.cursor = self.value.len();
    }

    /// Clear the input
    pub fn clear(&mut self) {
        if self.focus != FocusState::Disabled {
            self.value.clear();
            self.cursor = 0;
        }
    }

    /// Set the value directly
    pub fn set_value(&mut self, value: impl Into<String>) {
        if self.focus != FocusState::Disabled {
            self.value = value.into();
            self.cursor = self.value.len();
        }
    }
}

/// Colors for the text input control
#[derive(Debug, Clone, Copy)]
pub struct TextInputColors {
    /// Label color
    pub label: Color,
    /// Input text color
    pub text: Color,
    /// Border/bracket color
    pub border: Color,
    /// Placeholder text color
    pub placeholder: Color,
    /// Cursor color
    pub cursor: Color,
    /// Focused highlight color
    pub focused: Color,
    /// Disabled color
    pub disabled: Color,
}

impl Default for TextInputColors {
    fn default() -> Self {
        Self {
            label: Color::White,
            text: Color::White,
            border: Color::Gray,
            placeholder: Color::DarkGray,
            cursor: Color::Yellow,
            focused: Color::Cyan,
            disabled: Color::DarkGray,
        }
    }
}

impl TextInputColors {
    /// Create colors from theme
    pub fn from_theme(theme: &crate::view::theme::Theme) -> Self {
        Self {
            label: theme.editor_fg,
            text: theme.editor_fg,
            border: theme.line_number_fg,
            placeholder: theme.line_number_fg,
            cursor: theme.cursor,
            focused: theme.selection_bg,
            disabled: theme.line_number_fg,
        }
    }
}

/// Layout information returned after rendering for hit testing
#[derive(Debug, Clone, Copy)]
pub struct TextInputLayout {
    /// The text input field area
    pub input_area: Rect,
    /// The full control area including label
    pub full_area: Rect,
    /// Cursor position in screen coordinates (if focused)
    pub cursor_pos: Option<(u16, u16)>,
}

impl TextInputLayout {
    /// Check if a point is within the input area
    pub fn is_input(&self, x: u16, y: u16) -> bool {
        x >= self.input_area.x
            && x < self.input_area.x + self.input_area.width
            && y >= self.input_area.y
            && y < self.input_area.y + self.input_area.height
    }
}

/// Render a text input control
///
/// # Arguments
/// * `frame` - The ratatui frame to render to
/// * `area` - Rectangle where the control should be rendered
/// * `state` - The text input state
/// * `colors` - Colors for rendering
/// * `field_width` - Width of the input field (not including label)
///
/// # Returns
/// Layout information for hit testing
pub fn render_text_input(
    frame: &mut Frame,
    area: Rect,
    state: &TextInputState,
    colors: &TextInputColors,
    field_width: u16,
) -> TextInputLayout {
    let empty_layout = TextInputLayout {
        input_area: Rect::default(),
        full_area: area,
        cursor_pos: None,
    };

    if area.height == 0 || area.width < 5 {
        return empty_layout;
    }

    let (label_color, text_color, border_color, placeholder_color) = match state.focus {
        FocusState::Normal => (colors.label, colors.text, colors.border, colors.placeholder),
        FocusState::Focused => (
            colors.focused,
            colors.text,
            colors.focused,
            colors.placeholder,
        ),
        FocusState::Hovered => (
            colors.focused,
            colors.text,
            colors.focused,
            colors.placeholder,
        ),
        FocusState::Disabled => (
            colors.disabled,
            colors.disabled,
            colors.disabled,
            colors.disabled,
        ),
    };

    let label_width = state.label.len() as u16 + 2; // ": "
    let actual_field_width = field_width.min(area.width.saturating_sub(label_width + 2)); // "[" + "]"

    // Determine what text to display
    let (display_text, is_placeholder) = if state.value.is_empty() && !state.placeholder.is_empty()
    {
        (&state.placeholder, true)
    } else {
        (&state.value, false)
    };

    // Calculate visible portion of text
    let inner_width = actual_field_width.saturating_sub(2) as usize; // Inside brackets
    let scroll_offset = if state.cursor > inner_width {
        state.cursor - inner_width
    } else {
        0
    };

    let visible_text: String = display_text
        .chars()
        .skip(scroll_offset)
        .take(inner_width)
        .collect();

    let padded = format!("{:width$}", visible_text, width = inner_width);

    let text_style = if is_placeholder {
        Style::default().fg(placeholder_color)
    } else {
        Style::default().fg(text_color)
    };

    let line = Line::from(vec![
        Span::styled(&state.label, Style::default().fg(label_color)),
        Span::styled(": ", Style::default().fg(label_color)),
        Span::styled("[", Style::default().fg(border_color)),
        Span::styled(padded, text_style),
        Span::styled("]", Style::default().fg(border_color)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);

    let input_start = area.x + label_width;
    let input_area = Rect::new(input_start, area.y, actual_field_width + 2, 1);

    // Calculate cursor position if focused
    let cursor_pos = if state.focus == FocusState::Focused && !is_placeholder {
        let cursor_x = input_start + 1 + (state.cursor - scroll_offset) as u16;
        if cursor_x < input_start + actual_field_width + 1 {
            // Render cursor by overwriting the character at cursor position
            let cursor_area = Rect::new(cursor_x, area.y, 1, 1);
            let cursor_char = state.value.chars().nth(state.cursor).unwrap_or(' ');
            let cursor_span = Span::styled(
                cursor_char.to_string(),
                Style::default()
                    .fg(colors.cursor)
                    .add_modifier(Modifier::REVERSED),
            );
            frame.render_widget(Paragraph::new(Line::from(vec![cursor_span])), cursor_area);
            Some((cursor_x, area.y))
        } else {
            None
        }
    } else {
        None
    };

    TextInputLayout {
        input_area,
        full_area: Rect::new(
            area.x,
            area.y,
            input_start - area.x + actual_field_width + 2,
            1,
        ),
        cursor_pos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn test_frame<F>(width: u16, height: u16, f: F)
    where
        F: FnOnce(&mut Frame, Rect),
    {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, width, height);
                f(frame, area);
            })
            .unwrap();
    }

    #[test]
    fn test_text_input_renders() {
        test_frame(40, 1, |frame, area| {
            let state = TextInputState::new("Name").with_value("John");
            let colors = TextInputColors::default();
            let layout = render_text_input(frame, area, &state, &colors, 20);

            assert!(layout.input_area.width > 0);
        });
    }

    #[test]
    fn test_text_input_insert() {
        let mut state = TextInputState::new("Test");
        state.insert('a');
        state.insert('b');
        state.insert('c');
        assert_eq!(state.value, "abc");
        assert_eq!(state.cursor, 3);
    }

    #[test]
    fn test_text_input_backspace() {
        let mut state = TextInputState::new("Test").with_value("abc");
        state.backspace();
        assert_eq!(state.value, "ab");
        assert_eq!(state.cursor, 2);
    }

    #[test]
    fn test_text_input_cursor_movement() {
        let mut state = TextInputState::new("Test").with_value("hello");
        assert_eq!(state.cursor, 5);

        state.move_left();
        assert_eq!(state.cursor, 4);

        state.move_home();
        assert_eq!(state.cursor, 0);

        state.move_right();
        assert_eq!(state.cursor, 1);

        state.move_end();
        assert_eq!(state.cursor, 5);
    }

    #[test]
    fn test_text_input_delete() {
        let mut state = TextInputState::new("Test").with_value("abc");
        state.move_home();
        state.delete();
        assert_eq!(state.value, "bc");
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_text_input_disabled() {
        let mut state = TextInputState::new("Test").with_focus(FocusState::Disabled);
        state.insert('a');
        assert_eq!(state.value, "");
    }

    #[test]
    fn test_text_input_clear() {
        let mut state = TextInputState::new("Test").with_value("hello");
        state.clear();
        assert_eq!(state.value, "");
        assert_eq!(state.cursor, 0);
    }
}
