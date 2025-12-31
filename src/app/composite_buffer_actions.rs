//! Composite buffer management actions
//!
//! This module handles creating, managing, and closing composite buffers
//! which display multiple source buffers in a single tab.

use crate::app::types::BufferMetadata;
use crate::app::Editor;
use crate::model::composite_buffer::{CompositeBuffer, CompositeLayout, LineAlignment, SourcePane};
use crate::model::event::{BufferId, SplitId};
use crate::view::composite_view::CompositeViewState;

impl Editor {
    // =========================================================================
    // Composite Buffer Methods
    // =========================================================================

    /// Check if a buffer is a composite buffer
    pub fn is_composite_buffer(&self, buffer_id: BufferId) -> bool {
        self.composite_buffers.contains_key(&buffer_id)
    }

    /// Get a composite buffer by ID
    pub fn get_composite(&self, buffer_id: BufferId) -> Option<&CompositeBuffer> {
        self.composite_buffers.get(&buffer_id)
    }

    /// Get a mutable composite buffer by ID
    pub fn get_composite_mut(&mut self, buffer_id: BufferId) -> Option<&mut CompositeBuffer> {
        self.composite_buffers.get_mut(&buffer_id)
    }

    /// Get or create composite view state for a split
    pub fn get_composite_view_state(
        &mut self,
        split_id: SplitId,
        buffer_id: BufferId,
    ) -> Option<&mut CompositeViewState> {
        if !self.composite_buffers.contains_key(&buffer_id) {
            return None;
        }

        let pane_count = self.composite_buffers.get(&buffer_id)?.pane_count();

        Some(
            self.composite_view_states
                .entry((split_id, buffer_id))
                .or_insert_with(|| CompositeViewState::new(buffer_id, pane_count)),
        )
    }

    /// Create a new composite buffer
    ///
    /// # Arguments
    /// * `name` - Display name for the composite buffer (shown in tab)
    /// * `mode` - Mode for keybindings (e.g., "diff-view")
    /// * `layout` - How panes are arranged (side-by-side, stacked, unified)
    /// * `sources` - Source panes to display
    ///
    /// # Returns
    /// The ID of the newly created composite buffer
    pub fn create_composite_buffer(
        &mut self,
        name: String,
        mode: String,
        layout: CompositeLayout,
        sources: Vec<SourcePane>,
    ) -> BufferId {
        let buffer_id = BufferId(self.next_buffer_id);
        self.next_buffer_id += 1;

        let composite = CompositeBuffer::new(
            buffer_id,
            name.clone(),
            mode.clone(),
            layout,
            sources,
        );
        self.composite_buffers.insert(buffer_id, composite);

        // Add metadata for display
        let metadata = BufferMetadata::virtual_buffer(name, mode, true);
        self.buffer_metadata.insert(buffer_id, metadata);

        buffer_id
    }

    /// Set the line alignment for a composite buffer
    ///
    /// The alignment determines how lines from different source buffers
    /// are paired up for display (important for diff views).
    pub fn set_composite_alignment(&mut self, buffer_id: BufferId, alignment: LineAlignment) {
        if let Some(composite) = self.composite_buffers.get_mut(&buffer_id) {
            composite.set_alignment(alignment);
        }
    }

    /// Close a composite buffer and clean up associated state
    pub fn close_composite_buffer(&mut self, buffer_id: BufferId) {
        self.composite_buffers.remove(&buffer_id);
        self.buffer_metadata.remove(&buffer_id);

        // Remove all view states for this buffer
        self.composite_view_states
            .retain(|(_, bid), _| *bid != buffer_id);
    }

    /// Switch focus to the next pane in a composite buffer
    pub fn composite_focus_next(&mut self, buffer_id: BufferId) {
        if let Some(composite) = self.composite_buffers.get_mut(&buffer_id) {
            composite.focus_next();
        }
    }

    /// Switch focus to the previous pane in a composite buffer
    pub fn composite_focus_prev(&mut self, buffer_id: BufferId) {
        if let Some(composite) = self.composite_buffers.get_mut(&buffer_id) {
            composite.focus_prev();
        }
    }

    /// Navigate to the next hunk in a composite buffer's diff view
    pub fn composite_next_hunk(&mut self, split_id: SplitId, buffer_id: BufferId) -> bool {
        if let (Some(composite), Some(view_state)) = (
            self.composite_buffers.get(&buffer_id),
            self.composite_view_states.get_mut(&(split_id, buffer_id)),
        ) {
            if let Some(next_row) = composite.alignment.next_hunk_row(view_state.scroll_row) {
                view_state.scroll_row = next_row;
                return true;
            }
        }
        false
    }

    /// Navigate to the previous hunk in a composite buffer's diff view
    pub fn composite_prev_hunk(&mut self, split_id: SplitId, buffer_id: BufferId) -> bool {
        if let (Some(composite), Some(view_state)) = (
            self.composite_buffers.get(&buffer_id),
            self.composite_view_states.get_mut(&(split_id, buffer_id)),
        ) {
            if let Some(prev_row) = composite.alignment.prev_hunk_row(view_state.scroll_row) {
                view_state.scroll_row = prev_row;
                return true;
            }
        }
        false
    }

    /// Scroll a composite buffer view
    pub fn composite_scroll(&mut self, split_id: SplitId, buffer_id: BufferId, delta: isize) {
        if let (Some(composite), Some(view_state)) = (
            self.composite_buffers.get(&buffer_id),
            self.composite_view_states.get_mut(&(split_id, buffer_id)),
        ) {
            let max_row = composite.row_count().saturating_sub(1);
            view_state.scroll(delta, max_row);
        }
    }

    /// Scroll composite buffer to a specific row
    pub fn composite_scroll_to(&mut self, split_id: SplitId, buffer_id: BufferId, row: usize) {
        if let (Some(composite), Some(view_state)) = (
            self.composite_buffers.get(&buffer_id),
            self.composite_view_states.get_mut(&(split_id, buffer_id)),
        ) {
            let max_row = composite.row_count().saturating_sub(1);
            view_state.set_scroll_row(row, max_row);
        }
    }

    // =========================================================================
    // Plugin Command Handlers
    // =========================================================================

    /// Handle the CreateCompositeBuffer plugin command
    pub(crate) fn handle_create_composite_buffer(
        &mut self,
        name: String,
        mode: String,
        layout_config: crate::services::plugins::api::CompositeLayoutConfig,
        source_configs: Vec<crate::services::plugins::api::CompositeSourceConfig>,
        hunks: Option<Vec<crate::services::plugins::api::CompositeHunk>>,
        _request_id: Option<u64>,
    ) {
        use crate::model::composite_buffer::{
            CompositeLayout, DiffHunk, GutterStyle, LineAlignment, PaneStyle, SourcePane,
        };

        // Convert layout config
        let layout = match layout_config.layout_type.as_str() {
            "stacked" => CompositeLayout::Stacked {
                spacing: layout_config.spacing.unwrap_or(1),
            },
            "unified" => CompositeLayout::Unified,
            _ => CompositeLayout::SideBySide {
                ratios: layout_config.ratios.unwrap_or_else(|| vec![0.5, 0.5]),
                show_separator: layout_config.show_separator,
            },
        };

        // Convert source configs
        let sources: Vec<SourcePane> = source_configs
            .into_iter()
            .map(|src| {
                let mut pane = SourcePane::new(
                    BufferId(src.buffer_id),
                    src.label,
                    src.editable,
                );
                if let Some(style_config) = src.style {
                    let gutter_style = match style_config.gutter_style.as_deref() {
                        Some("diff-markers") => GutterStyle::DiffMarkers,
                        Some("both") => GutterStyle::Both,
                        Some("none") => GutterStyle::None,
                        _ => GutterStyle::LineNumbers,
                    };
                    pane.style = PaneStyle {
                        add_bg: style_config.add_bg,
                        remove_bg: style_config.remove_bg,
                        modify_bg: style_config.modify_bg,
                        gutter_style,
                    };
                }
                pane
            })
            .collect();

        // Create the composite buffer
        let buffer_id = self.create_composite_buffer(name.clone(), mode.clone(), layout, sources);

        // Set alignment from hunks if provided
        if let Some(hunk_configs) = hunks {
            let diff_hunks: Vec<DiffHunk> = hunk_configs
                .into_iter()
                .map(|h| DiffHunk::new(h.old_start, h.old_count, h.new_start, h.new_count))
                .collect();

            // Get line counts from source buffers
            let old_line_count = self
                .buffers
                .get(&self.composite_buffers.get(&buffer_id).unwrap().sources[0].buffer_id)
                .and_then(|s| s.buffer.line_count())
                .unwrap_or(0);
            let new_line_count = self
                .buffers
                .get(&self.composite_buffers.get(&buffer_id).unwrap().sources[1].buffer_id)
                .and_then(|s| s.buffer.line_count())
                .unwrap_or(0);

            let alignment = LineAlignment::from_hunks(&diff_hunks, old_line_count, new_line_count);
            self.set_composite_alignment(buffer_id, alignment);
        }

        tracing::info!(
            "Created composite buffer '{}' with mode '{}' (id={:?})",
            name,
            mode,
            buffer_id
        );

        // TODO: Send response with buffer_id if request_id is provided
    }

    /// Handle the UpdateCompositeAlignment plugin command
    pub(crate) fn handle_update_composite_alignment(
        &mut self,
        buffer_id: BufferId,
        hunk_configs: Vec<crate::services::plugins::api::CompositeHunk>,
    ) {
        use crate::model::composite_buffer::{DiffHunk, LineAlignment};

        if let Some(composite) = self.composite_buffers.get(&buffer_id) {
            let diff_hunks: Vec<DiffHunk> = hunk_configs
                .into_iter()
                .map(|h| DiffHunk::new(h.old_start, h.old_count, h.new_start, h.new_count))
                .collect();

            // Get line counts from source buffers
            let old_line_count = self
                .buffers
                .get(&composite.sources[0].buffer_id)
                .and_then(|s| s.buffer.line_count())
                .unwrap_or(0);
            let new_line_count = self
                .buffers
                .get(&composite.sources[1].buffer_id)
                .and_then(|s| s.buffer.line_count())
                .unwrap_or(0);

            let alignment = LineAlignment::from_hunks(&diff_hunks, old_line_count, new_line_count);
            self.set_composite_alignment(buffer_id, alignment);
        }
    }
}
