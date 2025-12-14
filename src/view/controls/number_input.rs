//! Number input control with increment/decrement
//!
//! Renders as: `Label: [  42  ] [-] [+]`

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::FocusState;

/// State for a number input control
#[derive(Debug, Clone)]
pub struct NumberInputState {
    /// Current value
    pub value: i64,
    /// Minimum allowed value
    pub min: Option<i64>,
    /// Maximum allowed value
    pub max: Option<i64>,
    /// Step amount for increment/decrement
    pub step: i64,
    /// Label displayed before the input
    pub label: String,
    /// Focus state
    pub focus: FocusState,
}

impl NumberInputState {
    /// Create a new number input state
    pub fn new(value: i64, label: impl Into<String>) -> Self {
        Self {
            value,
            min: None,
            max: None,
            step: 1,
            label: label.into(),
            focus: FocusState::Normal,
        }
    }

    /// Set the minimum value
    pub fn with_min(mut self, min: i64) -> Self {
        self.min = Some(min);
        self
    }

    /// Set the maximum value
    pub fn with_max(mut self, max: i64) -> Self {
        self.max = Some(max);
        self
    }

    /// Set the step amount
    pub fn with_step(mut self, step: i64) -> Self {
        self.step = step;
        self
    }

    /// Set the focus state
    pub fn with_focus(mut self, focus: FocusState) -> Self {
        self.focus = focus;
        self
    }

    /// Increment the value by step
    pub fn increment(&mut self) {
        if self.focus == FocusState::Disabled {
            return;
        }
        let new_value = self.value.saturating_add(self.step);
        self.value = match self.max {
            Some(max) => new_value.min(max),
            None => new_value,
        };
    }

    /// Decrement the value by step
    pub fn decrement(&mut self) {
        if self.focus == FocusState::Disabled {
            return;
        }
        let new_value = self.value.saturating_sub(self.step);
        self.value = match self.min {
            Some(min) => new_value.max(min),
            None => new_value,
        };
    }

    /// Set the value directly, respecting min/max
    pub fn set_value(&mut self, value: i64) {
        if self.focus == FocusState::Disabled {
            return;
        }
        let mut v = value;
        if let Some(min) = self.min {
            v = v.max(min);
        }
        if let Some(max) = self.max {
            v = v.min(max);
        }
        self.value = v;
    }
}

/// Colors for the number input control
#[derive(Debug, Clone, Copy)]
pub struct NumberInputColors {
    /// Label color
    pub label: Color,
    /// Value text color
    pub value: Color,
    /// Border/bracket color
    pub border: Color,
    /// Button color (increment/decrement)
    pub button: Color,
    /// Focused highlight color
    pub focused: Color,
    /// Disabled color
    pub disabled: Color,
}

impl Default for NumberInputColors {
    fn default() -> Self {
        Self {
            label: Color::White,
            value: Color::Yellow,
            border: Color::Gray,
            button: Color::Cyan,
            focused: Color::Cyan,
            disabled: Color::DarkGray,
        }
    }
}

impl NumberInputColors {
    /// Create colors from theme
    pub fn from_theme(theme: &crate::view::theme::Theme) -> Self {
        Self {
            label: theme.editor_fg,
            value: theme.help_key_fg, // Highlighted value color
            border: theme.line_number_fg,
            button: theme.menu_active_fg,
            focused: theme.selection_bg,
            disabled: theme.line_number_fg,
        }
    }
}

/// Layout information returned after rendering for hit testing
#[derive(Debug, Clone, Copy)]
pub struct NumberInputLayout {
    /// The value display area
    pub value_area: Rect,
    /// The decrement button area
    pub decrement_area: Rect,
    /// The increment button area
    pub increment_area: Rect,
    /// The full control area
    pub full_area: Rect,
}

impl NumberInputLayout {
    /// Check if a point is on the decrement button
    pub fn is_decrement(&self, x: u16, y: u16) -> bool {
        x >= self.decrement_area.x
            && x < self.decrement_area.x + self.decrement_area.width
            && y >= self.decrement_area.y
            && y < self.decrement_area.y + self.decrement_area.height
    }

    /// Check if a point is on the increment button
    pub fn is_increment(&self, x: u16, y: u16) -> bool {
        x >= self.increment_area.x
            && x < self.increment_area.x + self.increment_area.width
            && y >= self.increment_area.y
            && y < self.increment_area.y + self.increment_area.height
    }
}

/// Render a number input control
///
/// # Arguments
/// * `frame` - The ratatui frame to render to
/// * `area` - Rectangle where the control should be rendered
/// * `state` - The number input state
/// * `colors` - Colors for rendering
///
/// # Returns
/// Layout information for hit testing
pub fn render_number_input(
    frame: &mut Frame,
    area: Rect,
    state: &NumberInputState,
    colors: &NumberInputColors,
) -> NumberInputLayout {
    let empty_layout = NumberInputLayout {
        value_area: Rect::default(),
        decrement_area: Rect::default(),
        increment_area: Rect::default(),
        full_area: area,
    };

    if area.height == 0 || area.width < 10 {
        return empty_layout;
    }

    let (label_color, value_color, border_color, button_color) = match state.focus {
        FocusState::Normal => (colors.label, colors.value, colors.border, colors.button),
        FocusState::Focused => (colors.focused, colors.value, colors.focused, colors.focused),
        FocusState::Hovered => (colors.focused, colors.value, colors.focused, colors.focused),
        FocusState::Disabled => (
            colors.disabled,
            colors.disabled,
            colors.disabled,
            colors.disabled,
        ),
    };

    // Format: "Label: [ value ] [-] [+]"
    let value_str = state.value.to_string();
    let value_padded = format!("{:^5}", value_str); // Center in 5 chars

    let line = Line::from(vec![
        Span::styled(&state.label, Style::default().fg(label_color)),
        Span::styled(": ", Style::default().fg(label_color)),
        Span::styled("[", Style::default().fg(border_color)),
        Span::styled(value_padded, Style::default().fg(value_color)),
        Span::styled("]", Style::default().fg(border_color)),
        Span::raw(" "),
        Span::styled("[-]", Style::default().fg(button_color)),
        Span::raw(" "),
        Span::styled("[+]", Style::default().fg(button_color)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);

    // Calculate layout positions
    let label_width = state.label.len() as u16 + 2; // ": "
    let value_start = area.x + label_width;
    let value_width = 7; // "[" + 5 chars + "]"

    let dec_start = value_start + value_width + 1;
    let dec_width = 3;

    let inc_start = dec_start + dec_width + 1;
    let inc_width = 3;

    NumberInputLayout {
        value_area: Rect::new(value_start, area.y, value_width, 1),
        decrement_area: Rect::new(dec_start, area.y, dec_width, 1),
        increment_area: Rect::new(inc_start, area.y, inc_width, 1),
        full_area: Rect::new(area.x, area.y, inc_start - area.x + inc_width, 1),
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
    fn test_number_input_renders() {
        test_frame(40, 1, |frame, area| {
            let state = NumberInputState::new(42, "Count");
            let colors = NumberInputColors::default();
            let layout = render_number_input(frame, area, &state, &colors);

            assert!(layout.value_area.width > 0);
            assert!(layout.decrement_area.width > 0);
            assert!(layout.increment_area.width > 0);
        });
    }

    #[test]
    fn test_number_input_increment() {
        let mut state = NumberInputState::new(5, "Value");
        state.increment();
        assert_eq!(state.value, 6);
    }

    #[test]
    fn test_number_input_decrement() {
        let mut state = NumberInputState::new(5, "Value");
        state.decrement();
        assert_eq!(state.value, 4);
    }

    #[test]
    fn test_number_input_min_max() {
        let mut state = NumberInputState::new(5, "Value").with_min(0).with_max(10);

        // Should not go below min
        state.set_value(-5);
        assert_eq!(state.value, 0);

        // Should not go above max
        state.set_value(20);
        assert_eq!(state.value, 10);
    }

    #[test]
    fn test_number_input_step() {
        let mut state = NumberInputState::new(0, "Value").with_step(5);
        state.increment();
        assert_eq!(state.value, 5);
        state.increment();
        assert_eq!(state.value, 10);
    }

    #[test]
    fn test_number_input_disabled() {
        let mut state = NumberInputState::new(5, "Value").with_focus(FocusState::Disabled);
        state.increment();
        assert_eq!(state.value, 5); // Should not change
    }

    #[test]
    fn test_number_input_hit_detection() {
        test_frame(40, 1, |frame, area| {
            let state = NumberInputState::new(42, "Count");
            let colors = NumberInputColors::default();
            let layout = render_number_input(frame, area, &state, &colors);

            // Decrement button should be detected
            let dec_x = layout.decrement_area.x;
            assert!(layout.is_decrement(dec_x, 0));
            assert!(!layout.is_increment(dec_x, 0));

            // Increment button should be detected
            let inc_x = layout.increment_area.x;
            assert!(layout.is_increment(inc_x, 0));
            assert!(!layout.is_decrement(inc_x, 0));
        });
    }
}
