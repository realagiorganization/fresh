use crate::event::ViewEventRange;
use crate::overlay::{Overlay, OverlayFace, OverlayNamespace};
use crate::state::EditorState;
use crate::text_buffer::Buffer;
use lsp_types::{Diagnostic, DiagnosticSeverity};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Mutex;

pub fn lsp_diagnostic_namespace() -> OverlayNamespace {
    OverlayNamespace::from_string("lsp-diagnostic".to_string())
}

static DIAGNOSTIC_CACHE: Mutex<Option<u64>> = Mutex::new(None);

fn compute_diagnostic_hash(diagnostics: &[Diagnostic]) -> u64 {
    let mut hasher = DefaultHasher::new();
    diagnostics.len().hash(&mut hasher);
    for diag in diagnostics {
        diag.range.start.line.hash(&mut hasher);
        diag.range.start.character.hash(&mut hasher);
        diag.range.end.line.hash(&mut hasher);
        diag.range.end.character.hash(&mut hasher);
        let severity_value: i32 = match diag.severity {
            Some(DiagnosticSeverity::ERROR) => 1,
            Some(DiagnosticSeverity::WARNING) => 2,
            Some(DiagnosticSeverity::INFORMATION) => 3,
            Some(DiagnosticSeverity::HINT) => 4,
            None => 0,
            _ => -1,
        };
        severity_value.hash(&mut hasher);
        diag.message.hash(&mut hasher);
        if let Some(source) = &diag.source {
            source.hash(&mut hasher);
        }
    }
    hasher.finish()
}

pub fn apply_diagnostics_to_state_cached(
    state: &mut EditorState,
    diagnostics: &[Diagnostic],
    theme: &crate::theme::Theme,
) {
    let new_hash = compute_diagnostic_hash(diagnostics);

    if let Ok(cache) = DIAGNOSTIC_CACHE.lock() {
        if let Some(cached_hash) = *cache {
            if cached_hash == new_hash {
                return;
            }
        }
    }

    apply_diagnostics_to_state(state, diagnostics, theme);

    if let Ok(mut cache) = DIAGNOSTIC_CACHE.lock() {
        *cache = Some(new_hash);
    }
}

pub fn diagnostic_to_overlay(
    diagnostic: &Diagnostic,
    buffer: &Buffer,
    theme: &crate::theme::Theme,
) -> Option<(ViewEventRange, Option<Range<usize>>, OverlayFace, i32)> {
    let start_line = diagnostic.range.start.line as usize;
    let start_char = diagnostic.range.start.character as usize;
    let end_line = diagnostic.range.end.line as usize;
    let end_char = diagnostic.range.end.character as usize;

    let start_byte = buffer.lsp_position_to_byte(start_line, start_char);
    let end_byte = buffer.lsp_position_to_byte(end_line, end_char);

    let (face, priority) = match diagnostic.severity {
        Some(DiagnosticSeverity::ERROR) => (
            OverlayFace::Background {
                color: theme.diagnostic_error_bg,
            },
            100,
        ),
        Some(DiagnosticSeverity::WARNING) => (
            OverlayFace::Background {
                color: theme.diagnostic_warning_bg,
            },
            50,
        ),
        Some(DiagnosticSeverity::INFORMATION) => (
            OverlayFace::Background {
                color: theme.diagnostic_info_bg,
            },
            30,
        ),
        Some(DiagnosticSeverity::HINT) | None => (
            OverlayFace::Background {
                color: theme.diagnostic_hint_bg,
            },
            10,
        ),
        _ => return None,
    };

    let view_range = ViewEventRange::from_source_range(start_byte..end_byte);
    Some((view_range, Some(start_byte..end_byte), face, priority))
}

pub fn apply_diagnostics_to_state(
    state: &mut EditorState,
    diagnostics: &[Diagnostic],
    theme: &crate::theme::Theme,
) {
    let ns = lsp_diagnostic_namespace();
    state.overlays.clear_namespace(&ns, &mut state.marker_list);

    for diagnostic in diagnostics {
        if let Some((view_range, source_range, face, priority)) =
            diagnostic_to_overlay(diagnostic, &state.buffer, theme)
        {
            let message = diagnostic.message.clone();
            let range = source_range.unwrap_or(0..0); // marker list still uses byte ranges

            let overlay = Overlay::with_namespace(&mut state.marker_list, range, face, ns.clone())
                .with_priority_value(priority)
                .with_message(message);

            state.overlays.add(overlay);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_buffer::Buffer;
    use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

    #[test]
    fn diagnostic_hash_changes_with_content() {
        let diag1 = Diagnostic {
            range: Range {
                start: Position::new(0, 0),
                end: Position::new(0, 1),
            },
            severity: Some(DiagnosticSeverity::ERROR),
            message: "err".to_string(),
            ..Default::default()
        };
        let mut diag2 = diag1.clone();
        diag2.message = "other".to_string();
        assert_ne!(compute_diagnostic_hash(&[diag1]), compute_diagnostic_hash(&[diag2]));
    }

    #[test]
    fn diagnostic_to_overlay_maps_range() {
        let buffer = Buffer::from_str_test("hello\nworld");
        let diagnostic = Diagnostic {
            range: Range {
                start: Position::new(0, 1),
                end: Position::new(0, 3),
            },
            severity: Some(DiagnosticSeverity::WARNING),
            message: "warn".to_string(),
            ..Default::default()
        };

        let (view_range, source_range, _face, _priority) =
            diagnostic_to_overlay(&diagnostic, &buffer, &crate::theme::Theme::default()).unwrap();
        assert_eq!(view_range.start.source_byte, Some(1));
        assert_eq!(view_range.end.source_byte, Some(3));
        assert_eq!(source_range, Some(1..3));
    }
}
