//! WASM-safe syntax highlighting using syntect
//!
//! This module provides syntect-based syntax highlighting that works in both
//! native and WASM builds. It doesn't depend on model::buffer::Buffer, making
//! it suitable for use in the WASM editor.
//!
//! For the native editor, the more feature-rich `highlight_engine.rs` is used
//! which also includes tree-sitter fallback.

use ratatui::style::Color;
use std::ops::Range;
use std::sync::Arc;
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use crate::view::theme::Theme;

/// Highlight category for syntax elements
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightCategory {
    Attribute,
    Comment,
    Constant,
    Function,
    Keyword,
    Number,
    Operator,
    Property,
    String,
    Type,
    Variable,
}

impl HighlightCategory {
    /// Get the color for this category from the theme
    pub fn color(&self, theme: &Theme) -> Color {
        match self {
            Self::Attribute => theme.syntax_constant,
            Self::Comment => theme.syntax_comment,
            Self::Constant => theme.syntax_constant,
            Self::Function => theme.syntax_function,
            Self::Keyword => theme.syntax_keyword,
            Self::Number => theme.syntax_constant,
            Self::Operator => theme.syntax_operator,
            Self::Property => theme.syntax_variable,
            Self::String => theme.syntax_string,
            Self::Type => theme.syntax_type,
            Self::Variable => theme.syntax_variable,
        }
    }
}

/// A highlighted span of text
#[derive(Debug, Clone)]
pub struct HighlightSpan {
    /// Byte range in the source text
    pub range: Range<usize>,
    /// Color for this span
    pub color: Color,
}

/// Cached highlight span (stores category for theme-independent caching)
#[derive(Debug, Clone)]
struct CachedSpan {
    range: Range<usize>,
    category: HighlightCategory,
}

/// Map TextMate scope to highlight category
fn scope_to_category(scope: &str) -> Option<HighlightCategory> {
    let scope_lower = scope.to_lowercase();

    // Comments - highest priority
    if scope_lower.starts_with("comment") {
        return Some(HighlightCategory::Comment);
    }

    // Strings
    if scope_lower.starts_with("string") {
        return Some(HighlightCategory::String);
    }

    // Keywords (but not keyword.operator)
    if scope_lower.starts_with("keyword") && !scope_lower.starts_with("keyword.operator") {
        return Some(HighlightCategory::Keyword);
    }

    // Operators
    if scope_lower.starts_with("keyword.operator") || scope_lower.starts_with("punctuation") {
        return Some(HighlightCategory::Operator);
    }

    // Functions
    if scope_lower.starts_with("entity.name.function")
        || scope_lower.starts_with("support.function")
        || scope_lower.starts_with("meta.function-call")
        || scope_lower.starts_with("variable.function")
    {
        return Some(HighlightCategory::Function);
    }

    // Types
    if scope_lower.starts_with("entity.name.type")
        || scope_lower.starts_with("entity.name.class")
        || scope_lower.starts_with("support.type")
        || scope_lower.starts_with("support.class")
        || scope_lower.starts_with("storage.type")
    {
        return Some(HighlightCategory::Type);
    }

    // Storage modifiers
    if scope_lower.starts_with("storage.modifier") {
        return Some(HighlightCategory::Keyword);
    }

    // Constants and numbers
    if scope_lower.starts_with("constant.numeric")
        || scope_lower.starts_with("constant.language.boolean")
    {
        return Some(HighlightCategory::Number);
    }
    if scope_lower.starts_with("constant") {
        return Some(HighlightCategory::Constant);
    }

    // Variables
    if scope_lower.starts_with("variable") {
        return Some(HighlightCategory::Variable);
    }

    // Properties
    if scope_lower.starts_with("entity.name.tag")
        || scope_lower.starts_with("support.other.property")
        || scope_lower.starts_with("meta.object-literal.key")
    {
        return Some(HighlightCategory::Property);
    }

    // Attributes
    if scope_lower.starts_with("entity.other.attribute")
        || scope_lower.starts_with("meta.attribute")
    {
        return Some(HighlightCategory::Attribute);
    }

    None
}

/// WASM-safe syntax highlighter using syntect
pub struct SyntectHighlighter {
    syntax_set: Arc<SyntaxSet>,
    syntax_index: usize,
    cache: Option<HighlightCache>,
    last_content_len: usize,
}

#[derive(Debug, Clone)]
struct HighlightCache {
    range: Range<usize>,
    spans: Vec<CachedSpan>,
}

impl SyntectHighlighter {
    /// Create a new highlighter for the given file extension
    pub fn for_extension(ext: &str) -> Option<Self> {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let syntax = syntax_set.find_syntax_by_extension(ext)?;

        let syntax_index = syntax_set
            .syntaxes()
            .iter()
            .position(|s| s.name == syntax.name)?;

        Some(Self {
            syntax_set: Arc::new(syntax_set),
            syntax_index,
            cache: None,
            last_content_len: 0,
        })
    }

    /// Create a new highlighter for the given syntax name (e.g., "Rust", "Python")
    pub fn for_syntax_name(name: &str) -> Option<Self> {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let syntax = syntax_set.find_syntax_by_name(name)?;

        let syntax_index = syntax_set
            .syntaxes()
            .iter()
            .position(|s| s.name == syntax.name)?;

        Some(Self {
            syntax_set: Arc::new(syntax_set),
            syntax_index,
            cache: None,
            last_content_len: 0,
        })
    }

    /// Get the syntax reference
    fn syntax(&self) -> &SyntaxReference {
        &self.syntax_set.syntaxes()[self.syntax_index]
    }

    /// Get the syntax name
    pub fn syntax_name(&self) -> &str {
        &self.syntax().name
    }

    /// Highlight a string and return colored spans
    ///
    /// `viewport_start` and `viewport_end` are byte offsets into the content.
    /// Only spans within or overlapping this range are returned.
    /// `context_bytes` controls how much context before the viewport to parse
    /// for accurate multi-line constructs.
    pub fn highlight_viewport(
        &mut self,
        content: &str,
        viewport_start: usize,
        viewport_end: usize,
        theme: &Theme,
        context_bytes: usize,
    ) -> Vec<HighlightSpan> {
        // Check cache validity
        if let Some(cache) = &self.cache {
            if cache.range.start <= viewport_start
                && cache.range.end >= viewport_end
                && self.last_content_len == content.len()
            {
                return cache
                    .spans
                    .iter()
                    .filter(|span| {
                        span.range.start < viewport_end && span.range.end > viewport_start
                    })
                    .map(|span| HighlightSpan {
                        range: span.range.clone(),
                        color: span.category.color(theme),
                    })
                    .collect();
            }
        }

        // Cache miss - parse
        let parse_start = viewport_start.saturating_sub(context_bytes);
        let parse_end = viewport_end.saturating_add(context_bytes).min(content.len());

        if parse_end <= parse_start {
            return Vec::new();
        }

        let mut state = ParseState::new(self.syntax());
        let mut spans = Vec::new();
        let mut current_scopes = ScopeStack::new();
        let mut current_offset = 0;

        // Parse line by line
        for line in content.lines() {
            let line_with_newline = format!("{}\n", line);
            let line_len = line.len();

            // Skip lines before our parse region
            if current_offset + line_len < parse_start {
                // Still need to parse for state, but don't collect spans
                let _ = state.parse_line(&line_with_newline, &self.syntax_set);
                current_offset += line_len + 1; // +1 for newline
                continue;
            }

            // Stop if we're past the parse region
            if current_offset >= parse_end {
                break;
            }

            // Parse this line
            let ops = match state.parse_line(&line_with_newline, &self.syntax_set) {
                Ok(ops) => ops,
                Err(_) => {
                    current_offset += line_len + 1;
                    continue;
                }
            };

            // Convert operations to spans
            let mut syntect_offset = 0;

            for (op_offset, op) in ops {
                let clamped_offset = op_offset.min(line_len);
                if clamped_offset > syntect_offset {
                    if let Some(category) = Self::scope_stack_to_category(&current_scopes) {
                        let byte_start = current_offset + syntect_offset;
                        let byte_end = current_offset + clamped_offset;
                        if byte_start < byte_end {
                            spans.push(CachedSpan {
                                range: byte_start..byte_end,
                                category,
                            });
                        }
                    }
                }
                syntect_offset = clamped_offset;
                let _ = current_scopes.apply(&op);
            }

            // Handle remaining text on line
            if syntect_offset < line_len {
                if let Some(category) = Self::scope_stack_to_category(&current_scopes) {
                    let byte_start = current_offset + syntect_offset;
                    let byte_end = current_offset + line_len;
                    if byte_start < byte_end {
                        spans.push(CachedSpan {
                            range: byte_start..byte_end,
                            category,
                        });
                    }
                }
            }

            current_offset += line_len + 1; // +1 for newline
        }

        // Merge adjacent spans with same category
        Self::merge_adjacent_spans(&mut spans);

        // Update cache
        self.cache = Some(HighlightCache {
            range: parse_start..parse_end,
            spans: spans.clone(),
        });
        self.last_content_len = content.len();

        // Filter to requested viewport and resolve colors
        spans
            .into_iter()
            .filter(|span| span.range.start < viewport_end && span.range.end > viewport_start)
            .map(|span| HighlightSpan {
                range: span.range,
                color: span.category.color(theme),
            })
            .collect()
    }

    /// Map scope stack to highlight category
    fn scope_stack_to_category(scopes: &ScopeStack) -> Option<HighlightCategory> {
        for scope in scopes.as_slice().iter().rev() {
            let scope_str = scope.build_string();
            if let Some(cat) = scope_to_category(&scope_str) {
                return Some(cat);
            }
        }
        None
    }

    /// Merge adjacent spans with same category
    fn merge_adjacent_spans(spans: &mut Vec<CachedSpan>) {
        if spans.len() < 2 {
            return;
        }

        let mut write_idx = 0;
        for read_idx in 1..spans.len() {
            if spans[write_idx].category == spans[read_idx].category
                && spans[write_idx].range.end == spans[read_idx].range.start
            {
                spans[write_idx].range.end = spans[read_idx].range.end;
            } else {
                write_idx += 1;
                if write_idx != read_idx {
                    spans[write_idx] = spans[read_idx].clone();
                }
            }
        }
        spans.truncate(write_idx + 1);
    }

    /// Invalidate the cache (call when content changes)
    pub fn invalidate(&mut self) {
        self.cache = None;
    }
}

/// Get a list of supported file extensions
pub fn supported_extensions() -> Vec<&'static str> {
    vec![
        "rs", "py", "js", "jsx", "ts", "tsx", "html", "css", "c", "h", "cpp", "hpp", "cc", "go",
        "json", "java", "cs", "php", "rb", "sh", "bash", "lua", "md", "yaml", "yml", "toml", "xml",
        "sql", "swift", "kt", "scala", "r", "pl", "pm",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlighter_creation() {
        let h = SyntectHighlighter::for_extension("rs");
        assert!(h.is_some());
        assert_eq!(h.unwrap().syntax_name(), "Rust");

        let h = SyntectHighlighter::for_extension("py");
        assert!(h.is_some());
        assert_eq!(h.unwrap().syntax_name(), "Python");

        let h = SyntectHighlighter::for_extension("unknown_xyz");
        assert!(h.is_none());
    }

    #[test]
    fn test_basic_highlighting() {
        let mut h = SyntectHighlighter::for_extension("rs").unwrap();
        let theme = Theme::dark();
        let content = "fn main() {\n    println!(\"hello\");\n}";

        let spans = h.highlight_viewport(content, 0, content.len(), &theme, 1000);

        // Should have some spans
        assert!(!spans.is_empty());

        // Keywords like "fn" should be highlighted
        let has_keyword = spans.iter().any(|s| s.color == theme.syntax_keyword);
        assert!(has_keyword, "Should highlight keywords");
    }

    #[test]
    fn test_viewport_highlighting() {
        let mut h = SyntectHighlighter::for_extension("rs").unwrap();
        let theme = Theme::dark();

        // Create content with multiple lines
        let content = "fn foo() {}\nfn bar() {}\nfn baz() {}";

        // Highlight only middle portion
        let spans = h.highlight_viewport(content, 12, 23, &theme, 0);

        // Spans should be within or near the viewport
        for span in &spans {
            assert!(
                span.range.start < 30,
                "Span start {} too far from viewport",
                span.range.start
            );
        }
    }
}
