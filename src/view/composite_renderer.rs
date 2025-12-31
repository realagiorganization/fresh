//! Composite buffer renderer
//!
//! Renders multiple source buffers side-by-side or in other layouts
//! within a single tab/split.

use crate::model::composite_buffer::{CompositeBuffer, CompositeLayout, RowType};
use crate::model::event::BufferId;
use crate::view::composite_view::CompositeViewState;
use crate::view::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::collections::HashMap;

/// A simple buffer interface for the renderer
pub trait BufferSource {
    /// Get the text content of a specific line
    fn get_line(&self, line: usize) -> Option<String>;
    /// Get the total number of lines
    fn line_count(&self) -> usize;
}

/// Renders a composite buffer into a frame
pub struct CompositeRenderer<'a> {
    composite: &'a CompositeBuffer,
    view_state: &'a CompositeViewState,
    theme: &'a Theme,
}

impl<'a> CompositeRenderer<'a> {
    /// Create a new composite renderer
    pub fn new(
        composite: &'a CompositeBuffer,
        view_state: &'a CompositeViewState,
        theme: &'a Theme,
    ) -> Self {
        Self {
            composite,
            view_state,
            theme,
        }
    }

    /// Render the composite buffer
    pub fn render<S: BufferSource>(
        &self,
        frame: &mut Frame,
        area: Rect,
        sources: &HashMap<BufferId, S>,
    ) {
        match &self.composite.layout {
            CompositeLayout::SideBySide {
                ratios,
                show_separator,
            } => {
                self.render_side_by_side(frame, area, ratios, *show_separator, sources);
            }
            CompositeLayout::Stacked { spacing } => {
                self.render_stacked(frame, area, *spacing, sources);
            }
            CompositeLayout::Unified => {
                self.render_unified(frame, area, sources);
            }
        }
    }

    fn render_side_by_side<S: BufferSource>(
        &self,
        frame: &mut Frame,
        area: Rect,
        ratios: &[f32],
        show_separator: bool,
        sources: &HashMap<BufferId, S>,
    ) {
        // Calculate pane widths
        let separator_width: u16 = if show_separator { 1 } else { 0 };
        let separator_count = self.composite.sources.len().saturating_sub(1);
        let total_separators = separator_count as u16 * separator_width;
        let available_width = area.width.saturating_sub(total_separators);

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
                x += separator_width;
            }
        }

        // Render each pane
        for (i, (pane, rect)) in self
            .composite
            .sources
            .iter()
            .zip(&pane_rects)
            .enumerate()
        {
            let is_focused = i == self.view_state.focused_pane;
            if let Some(source) = sources.get(&pane.buffer_id) {
                self.render_pane(frame, source, *rect, i, is_focused);
            }
        }
    }

    fn render_pane<S: BufferSource>(
        &self,
        frame: &mut Frame,
        source: &S,
        rect: Rect,
        pane_index: usize,
        is_focused: bool,
    ) {
        let alignment = &self.composite.alignment;
        let pane_style = &self.composite.sources[pane_index].style;
        let gutter_width = 5; // Line number gutter

        // Render visible rows based on alignment
        for display_row in
            self.view_state.scroll_row..self.view_state.scroll_row + rect.height as usize
        {
            if display_row >= alignment.rows.len() {
                break;
            }

            let aligned_row = &alignment.rows[display_row];
            let y = rect.y + (display_row - self.view_state.scroll_row) as u16;

            match aligned_row.get_pane_line(pane_index) {
                Some(source_ref) => {
                    // Render actual content from source buffer
                    let line_text = source.get_line(source_ref.line).unwrap_or_default();
                    let row_style = self.style_for_row_type(aligned_row.row_type, pane_index);

                    self.render_line_with_gutter(
                        frame,
                        rect.x,
                        y,
                        rect.width,
                        &line_text,
                        source_ref.line + 1, // 1-indexed for display
                        row_style,
                        gutter_width,
                        pane_style.gutter_style,
                        aligned_row.row_type,
                    );
                }
                None => {
                    // Render padding (empty line)
                    let style = Style::default().bg(Color::Rgb(30, 30, 30));
                    let padding_line = Line::from(vec![Span::styled(
                        " ".repeat(rect.width as usize),
                        style,
                    )]);
                    frame.render_widget(
                        Paragraph::new(vec![padding_line]),
                        Rect {
                            x: rect.x,
                            y,
                            width: rect.width,
                            height: 1,
                        },
                    );
                }
            }
        }

        // Render focus indicator (subtle border effect)
        if is_focused {
            self.render_focus_indicator(frame, rect);
        }
    }

    fn render_line_with_gutter(
        &self,
        frame: &mut Frame,
        x: u16,
        y: u16,
        width: u16,
        text: &str,
        line_num: usize,
        row_style: Style,
        gutter_width: usize,
        gutter_style: crate::model::composite_buffer::GutterStyle,
        row_type: RowType,
    ) {
        let mut spans = Vec::new();

        // Render gutter
        let gutter_text = match gutter_style {
            crate::model::composite_buffer::GutterStyle::LineNumbers => {
                format!("{:>width$} ", line_num, width = gutter_width - 1)
            }
            crate::model::composite_buffer::GutterStyle::DiffMarkers => {
                let marker = match row_type {
                    RowType::Addition => "+",
                    RowType::Deletion => "-",
                    RowType::Modification => "~",
                    RowType::Context => " ",
                    RowType::HunkHeader => "@",
                };
                format!("{:>width$} ", marker, width = gutter_width - 1)
            }
            crate::model::composite_buffer::GutterStyle::Both => {
                let marker = match row_type {
                    RowType::Addition => "+",
                    RowType::Deletion => "-",
                    RowType::Modification => "~",
                    _ => " ",
                };
                format!("{}{:>3} ", marker, line_num)
            }
            crate::model::composite_buffer::GutterStyle::None => String::new(),
        };

        let gutter_fg = match row_type {
            RowType::Addition => Color::Green,
            RowType::Deletion => Color::Red,
            RowType::Modification => Color::Yellow,
            _ => Color::DarkGray,
        };

        spans.push(Span::styled(
            gutter_text,
            Style::default().fg(gutter_fg),
        ));

        // Render content
        let content_width = width.saturating_sub(gutter_width as u16) as usize;
        let truncated_text: String = text.chars().take(content_width).collect();
        let padding = content_width.saturating_sub(truncated_text.len());

        spans.push(Span::styled(truncated_text, row_style));
        spans.push(Span::styled(" ".repeat(padding), row_style));

        let line = Line::from(spans);
        frame.render_widget(
            Paragraph::new(vec![line]),
            Rect {
                x,
                y,
                width,
                height: 1,
            },
        );
    }

    fn render_separator(&self, frame: &mut Frame, x: u16, y: u16, height: u16) {
        for row in 0..height {
            let sep_line = Line::from(vec![Span::styled(
                "│",
                Style::default().fg(Color::DarkGray),
            )]);
            frame.render_widget(
                Paragraph::new(vec![sep_line]),
                Rect {
                    x,
                    y: y + row,
                    width: 1,
                    height: 1,
                },
            );
        }
    }

    fn render_focus_indicator(&self, frame: &mut Frame, rect: Rect) {
        // Draw a subtle left border to indicate focus
        let style = Style::default()
            .fg(self.theme.cursor)
            .add_modifier(Modifier::BOLD);

        for row in 0..rect.height {
            // Just highlight the leftmost column
            let y = rect.y + row;
            frame.render_widget(
                Paragraph::new(vec![Line::from(vec![Span::styled("▌", style)])]),
                Rect {
                    x: rect.x,
                    y,
                    width: 1,
                    height: 1,
                },
            );
        }
    }

    fn style_for_row_type(&self, row_type: RowType, pane_index: usize) -> Style {
        let pane_style = &self.composite.sources[pane_index].style;

        match row_type {
            RowType::Addition => {
                let (r, g, b) = pane_style.add_bg.unwrap_or((0, 60, 0));
                Style::default().bg(Color::Rgb(r, g, b))
            }
            RowType::Deletion => {
                let (r, g, b) = pane_style.remove_bg.unwrap_or((60, 0, 0));
                Style::default().bg(Color::Rgb(r, g, b))
            }
            RowType::Modification => {
                let (r, g, b) = pane_style.modify_bg.unwrap_or((60, 60, 0));
                Style::default().bg(Color::Rgb(r, g, b))
            }
            RowType::Context => Style::default(),
            RowType::HunkHeader => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        }
    }

    fn render_stacked<S: BufferSource>(
        &self,
        frame: &mut Frame,
        area: Rect,
        spacing: u16,
        sources: &HashMap<BufferId, S>,
    ) {
        // Divide vertical space equally among panes
        let pane_count = self.composite.sources.len() as u16;
        if pane_count == 0 {
            return;
        }

        let total_spacing = spacing * (pane_count - 1);
        let available_height = area.height.saturating_sub(total_spacing);
        let pane_height = available_height / pane_count;

        let mut y = area.y;
        for (i, pane) in self.composite.sources.iter().enumerate() {
            if let Some(source) = sources.get(&pane.buffer_id) {
                let is_focused = i == self.view_state.focused_pane;
                let rect = Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: pane_height,
                };
                self.render_pane(frame, source, rect, i, is_focused);
            }
            y += pane_height + spacing;
        }
    }

    fn render_unified<S: BufferSource>(
        &self,
        frame: &mut Frame,
        area: Rect,
        sources: &HashMap<BufferId, S>,
    ) {
        // Unified diff: interleave lines from old and new
        let alignment = &self.composite.alignment;
        let gutter_width = 10; // Wider for dual line numbers

        for display_row in
            self.view_state.scroll_row..self.view_state.scroll_row + area.height as usize
        {
            if display_row >= alignment.rows.len() {
                break;
            }

            let aligned_row = &alignment.rows[display_row];
            let y = area.y + (display_row - self.view_state.scroll_row) as u16;

            // Get content from whichever pane has it
            let (text, line_nums, pane_idx) = if let Some(source_ref) =
                aligned_row.get_pane_line(0)
            {
                let source = sources.get(&self.composite.sources[0].buffer_id);
                let text = source
                    .and_then(|s| s.get_line(source_ref.line))
                    .unwrap_or_default();
                (text, format!("-{:<4}", source_ref.line + 1), 0)
            } else if let Some(source_ref) = aligned_row.get_pane_line(1) {
                let source = sources.get(&self.composite.sources[1].buffer_id);
                let text = source
                    .and_then(|s| s.get_line(source_ref.line))
                    .unwrap_or_default();
                (text, format!("+{:<4}", source_ref.line + 1), 1)
            } else {
                ("".to_string(), "     ".to_string(), 0)
            };

            let row_style = self.style_for_row_type(aligned_row.row_type, pane_idx);
            let marker = match aligned_row.row_type {
                RowType::Addition => "+",
                RowType::Deletion => "-",
                RowType::Modification => "~",
                RowType::Context => " ",
                RowType::HunkHeader => "@",
            };

            let gutter_text = format!("{} {} ", line_nums, marker);
            let gutter_style = match aligned_row.row_type {
                RowType::Addition => Style::default().fg(Color::Green),
                RowType::Deletion => Style::default().fg(Color::Red),
                RowType::Modification => Style::default().fg(Color::Yellow),
                RowType::HunkHeader => Style::default().fg(Color::Cyan),
                _ => Style::default().fg(Color::DarkGray),
            };

            let content_width = area.width.saturating_sub(gutter_width) as usize;
            let truncated_text: String = text.chars().take(content_width).collect();
            let padding = content_width.saturating_sub(truncated_text.len());

            let line = Line::from(vec![
                Span::styled(gutter_text, gutter_style),
                Span::styled(truncated_text, row_style),
                Span::styled(" ".repeat(padding), row_style),
            ]);

            frame.render_widget(
                Paragraph::new(vec![line]),
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
            );
        }
    }
}

/// Simple buffer wrapper for testing
#[derive(Debug)]
pub struct SimpleBuffer {
    lines: Vec<String>,
}

impl SimpleBuffer {
    pub fn new(content: &str) -> Self {
        Self {
            lines: content.lines().map(|s| s.to_string()).collect(),
        }
    }
}

impl BufferSource for SimpleBuffer {
    fn get_line(&self, line: usize) -> Option<String> {
        self.lines.get(line).cloned()
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::composite_buffer::{LineAlignment, SourcePane};

    #[test]
    fn test_simple_buffer() {
        let buf = SimpleBuffer::new("line 1\nline 2\nline 3");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.get_line(0), Some("line 1".to_string()));
        assert_eq!(buf.get_line(2), Some("line 3".to_string()));
        assert_eq!(buf.get_line(3), None);
    }
}
