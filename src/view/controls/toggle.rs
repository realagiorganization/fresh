//! Toggle (checkbox) control for boolean values
//!
//! Renders as: `[x] Label` or `[ ] Label`

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::FocusState;

/// State for a toggle control
#[derive(Debug, Clone)]
pub struct ToggleState {
    /// Current value
    pub checked: bool,
    /// Label displayed next to the toggle
    pub label: String,
    /// Focus state
    pub focus: FocusState,
}

impl ToggleState {
    /// Create a new toggle state
    pub fn new(checked: bool, label: impl Into<String>) -> Self {
        Self {
            checked,
            label: label.into(),
            focus: FocusState::Normal,
        }
    }

    /// Set the focus state
    pub fn with_focus(mut self, focus: FocusState) -> Self {
        self.focus = focus;
        self
    }

    /// Toggle the value
    pub fn toggle(&mut self) {
        if self.focus != FocusState::Disabled {
            self.checked = !self.checked;
        }
    }
}

/// Colors for the toggle control
#[derive(Debug, Clone, Copy)]
pub struct ToggleColors {
    /// Checkbox bracket color
    pub bracket: Color,
    /// Checkmark color when checked
    pub checkmark: Color,
    /// Label text color
    pub label: Color,
    /// Focused highlight color
    pub focused: Color,
    /// Disabled color
    pub disabled: Color,
}

impl Default for ToggleColors {
    fn default() -> Self {
        Self {
            bracket: Color::Gray,
            checkmark: Color::Green,
            label: Color::White,
            focused: Color::Cyan,
            disabled: Color::DarkGray,
        }
    }
}

impl ToggleColors {
    /// Create colors from theme
    pub fn from_theme(theme: &crate::view::theme::Theme) -> Self {
        Self {
            bracket: theme.line_number_fg,
            checkmark: theme.diagnostic_info_fg, // Green for checkmark
            label: theme.editor_fg,
            focused: theme.selection_bg,
            disabled: theme.line_number_fg,
        }
    }
}

/// Layout information returned after rendering for hit testing
#[derive(Debug, Clone, Copy)]
pub struct ToggleLayout {
    /// The checkbox area (clickable)
    pub checkbox_area: Rect,
    /// The full control area including label
    pub full_area: Rect,
}

impl ToggleLayout {
    /// Check if a point is within the clickable area
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.full_area.x
            && x < self.full_area.x + self.full_area.width
            && y >= self.full_area.y
            && y < self.full_area.y + self.full_area.height
    }
}

/// Render a toggle control
///
/// # Arguments
/// * `frame` - The ratatui frame to render to
/// * `area` - Rectangle where the toggle should be rendered
/// * `state` - The toggle state
/// * `colors` - Colors for rendering
///
/// # Returns
/// Layout information for hit testing
pub fn render_toggle(
    frame: &mut Frame,
    area: Rect,
    state: &ToggleState,
    colors: &ToggleColors,
) -> ToggleLayout {
    render_toggle_aligned(frame, area, state, colors, None)
}

/// Render a toggle control with optional label width alignment
///
/// # Arguments
/// * `frame` - The ratatui frame to render to
/// * `area` - Rectangle where the toggle should be rendered
/// * `state` - The toggle state
/// * `colors` - Colors for rendering
/// * `label_width` - Optional minimum label width for alignment
///
/// # Returns
/// Layout information for hit testing
pub fn render_toggle_aligned(
    frame: &mut Frame,
    area: Rect,
    state: &ToggleState,
    colors: &ToggleColors,
    label_width: Option<u16>,
) -> ToggleLayout {
    if area.height == 0 || area.width < 4 {
        return ToggleLayout {
            checkbox_area: Rect::default(),
            full_area: area,
        };
    }

    let (bracket_color, _check_color, label_color) = match state.focus {
        FocusState::Normal => (colors.bracket, colors.checkmark, colors.label),
        FocusState::Focused => (colors.focused, colors.checkmark, colors.focused),
        FocusState::Hovered => (colors.focused, colors.checkmark, colors.focused),
        FocusState::Disabled => (colors.disabled, colors.disabled, colors.disabled),
    };

    let checkbox = if state.checked { "[x]" } else { "[ ]" };

    // Format: "Label: [x]" with optional padding
    let actual_label_width = label_width.unwrap_or(state.label.len() as u16);
    let padded_label = format!("{:width$}", state.label, width = actual_label_width as usize);

    let line = Line::from(vec![
        Span::styled(padded_label, Style::default().fg(label_color)),
        Span::styled(": ", Style::default().fg(label_color)),
        Span::styled(checkbox, Style::default().fg(bracket_color)),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);

    // Checkbox position after label
    let checkbox_start = area.x + actual_label_width + 2; // label + ": "
    let checkbox_area = Rect::new(checkbox_start, area.y, 3.min(area.width), 1);

    // Full area is label + ": " + checkbox
    let full_width = (actual_label_width + 2 + 3).min(area.width);
    let full_area = Rect::new(area.x, area.y, full_width, 1);

    ToggleLayout {
        checkbox_area,
        full_area,
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
    fn test_toggle_checked() {
        test_frame(20, 1, |frame, area| {
            let state = ToggleState::new(true, "Enable");
            let colors = ToggleColors::default();
            let layout = render_toggle(frame, area, &state, &colors);

            assert_eq!(layout.checkbox_area.width, 3);
            assert_eq!(layout.full_area.width, 11); // "Enable: [x]"
        });
    }

    #[test]
    fn test_toggle_unchecked() {
        test_frame(20, 1, |frame, area| {
            let state = ToggleState::new(false, "Enable");
            let colors = ToggleColors::default();
            let layout = render_toggle(frame, area, &state, &colors);

            assert_eq!(layout.checkbox_area.width, 3);
        });
    }

    #[test]
    fn test_toggle_click_detection() {
        test_frame(20, 1, |frame, area| {
            let state = ToggleState::new(true, "Enable");
            let colors = ToggleColors::default();
            let layout = render_toggle(frame, area, &state, &colors);

            // Click on checkbox
            assert!(layout.contains(0, 0));
            assert!(layout.contains(2, 0));

            // Click on label
            assert!(layout.contains(5, 0));

            // Click outside
            assert!(!layout.contains(15, 0));
        });
    }

    #[test]
    fn test_toggle_state_toggle() {
        let mut state = ToggleState::new(false, "Test");
        assert!(!state.checked);

        state.toggle();
        assert!(state.checked);

        state.toggle();
        assert!(!state.checked);
    }

    #[test]
    fn test_toggle_disabled_no_toggle() {
        let mut state = ToggleState::new(false, "Test").with_focus(FocusState::Disabled);
        state.toggle();
        assert!(!state.checked); // Should not change
    }

    #[test]
    fn test_toggle_narrow_area() {
        test_frame(2, 1, |frame, area| {
            let state = ToggleState::new(true, "Enable");
            let colors = ToggleColors::default();
            let layout = render_toggle(frame, area, &state, &colors);

            // Should still have some layout even if truncated
            assert!(layout.full_area.width <= area.width);
        });
    }
}
