//! Text content provider trait for shared rendering
//!
//! This module defines a trait that abstracts buffer access for rendering.
//! Both native `Buffer` and WASM `TextBuffer` implement this trait, enabling
//! shared rendering code between native and WASM builds.

use std::ops::Range;

/// Line ending format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// Unix-style line ending (LF)
    #[default]
    Lf,
    /// Windows-style line ending (CRLF)
    CrLf,
    /// Classic Mac-style line ending (CR)
    Cr,
}

impl LineEnding {
    /// Get the line ending as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }
}

/// A line of text with its byte offset in the buffer
#[derive(Debug, Clone)]
pub struct TextLine {
    /// Byte offset where this line starts in the buffer
    pub start_byte: usize,
    /// The line content (without line ending)
    pub content: String,
    /// Length of line ending (0, 1, or 2)
    pub line_ending_len: usize,
}

impl TextLine {
    /// Total byte length including line ending
    pub fn total_len(&self) -> usize {
        self.content.len() + self.line_ending_len
    }
}

/// Minimal interface for text content access during rendering
///
/// This trait is designed to be implementable by both native `Buffer` and
/// WASM `TextBuffer`, enabling shared rendering code between platforms.
///
/// All methods are read-only queries - no modification operations are included
/// as they're not needed for rendering.
pub trait TextContentProvider {
    /// Total bytes in buffer
    fn len(&self) -> usize;

    /// Check if buffer is empty
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether content is binary (affects rendering)
    fn is_binary(&self) -> bool;

    /// Get line ending format (LF, CRLF, CR)
    fn line_ending(&self) -> LineEnding;

    /// Total number of lines in the buffer
    fn line_count(&self) -> usize;

    /// Get a specific line by line number (0-indexed)
    /// Returns None if line number is out of bounds
    fn get_line(&self, line_num: usize) -> Option<TextLine>;

    /// Get raw bytes in range (for binary mode rendering)
    fn slice_bytes(&self, range: Range<usize>) -> Vec<u8>;

    /// Map byte offset to line number (0-indexed)
    fn byte_to_line(&self, byte_offset: usize) -> usize;

    /// Map line number to first byte offset of that line
    fn line_to_byte(&self, line: usize) -> Option<usize>;

    /// Get text in byte range
    fn get_text_range(&self, start: usize, end: usize) -> String;

    /// Get all content as a single string (for highlighting)
    fn content(&self) -> String;
}

/// Iterator over visible lines in a viewport
pub struct ViewportLineIterator<'a, T: TextContentProvider + ?Sized> {
    provider: &'a T,
    current_line: usize,
    end_line: usize,
}

impl<'a, T: TextContentProvider + ?Sized> ViewportLineIterator<'a, T> {
    /// Create a new iterator for the given viewport
    pub fn new(provider: &'a T, start_line: usize, end_line: usize) -> Self {
        Self {
            provider,
            current_line: start_line,
            end_line: end_line.min(provider.line_count()),
        }
    }
}

impl<'a, T: TextContentProvider + ?Sized> Iterator for ViewportLineIterator<'a, T> {
    type Item = (usize, TextLine);

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_line >= self.end_line {
            return None;
        }

        let line_num = self.current_line;
        self.current_line += 1;

        self.provider
            .get_line(line_num)
            .map(|line| (line_num, line))
    }
}

/// Extension trait for viewport iteration
pub trait TextContentProviderExt: TextContentProvider {
    /// Iterate over lines in a viewport range
    fn viewport_lines(&self, start_line: usize, end_line: usize) -> ViewportLineIterator<'_, Self> {
        ViewportLineIterator::new(self, start_line, end_line)
    }
}

impl<T: TextContentProvider + ?Sized> TextContentProviderExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    // Simple test implementation
    struct SimpleBuffer {
        lines: Vec<String>,
    }

    impl TextContentProvider for SimpleBuffer {
        fn len(&self) -> usize {
            self.lines
                .iter()
                .map(|l| l.len() + 1)
                .sum::<usize>()
                .saturating_sub(1)
        }

        fn is_binary(&self) -> bool {
            false
        }

        fn line_ending(&self) -> LineEnding {
            LineEnding::Lf
        }

        fn line_count(&self) -> usize {
            self.lines.len()
        }

        fn get_line(&self, line_num: usize) -> Option<TextLine> {
            self.lines.get(line_num).map(|content| {
                let start_byte = self.lines[..line_num].iter().map(|l| l.len() + 1).sum();
                TextLine {
                    start_byte,
                    content: content.clone(),
                    line_ending_len: if line_num < self.lines.len() - 1 {
                        1
                    } else {
                        0
                    },
                }
            })
        }

        fn slice_bytes(&self, range: Range<usize>) -> Vec<u8> {
            self.content().as_bytes()[range].to_vec()
        }

        fn byte_to_line(&self, byte_offset: usize) -> usize {
            let mut offset = 0;
            for (i, line) in self.lines.iter().enumerate() {
                if byte_offset < offset + line.len() + 1 {
                    return i;
                }
                offset += line.len() + 1;
            }
            self.lines.len().saturating_sub(1)
        }

        fn line_to_byte(&self, line: usize) -> Option<usize> {
            if line >= self.lines.len() {
                return None;
            }
            Some(self.lines[..line].iter().map(|l| l.len() + 1).sum())
        }

        fn get_text_range(&self, start: usize, end: usize) -> String {
            let content = self.content();
            content[start.min(content.len())..end.min(content.len())].to_string()
        }

        fn content(&self) -> String {
            self.lines.join("\n")
        }
    }

    #[test]
    fn test_viewport_iterator() {
        let buffer = SimpleBuffer {
            lines: vec![
                "line 0".into(),
                "line 1".into(),
                "line 2".into(),
                "line 3".into(),
            ],
        };

        let lines: Vec<_> = buffer.viewport_lines(1, 3).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, 1);
        assert_eq!(lines[0].1.content, "line 1");
        assert_eq!(lines[1].0, 2);
        assert_eq!(lines[1].1.content, "line 2");
    }

    #[test]
    fn test_byte_line_mapping() {
        let buffer = SimpleBuffer {
            lines: vec!["abc".into(), "defgh".into(), "i".into()],
        };

        // "abc\ndefgh\ni"
        assert_eq!(buffer.byte_to_line(0), 0); // 'a'
        assert_eq!(buffer.byte_to_line(3), 0); // '\n'
        assert_eq!(buffer.byte_to_line(4), 1); // 'd'
        assert_eq!(buffer.byte_to_line(9), 1); // '\n'
        assert_eq!(buffer.byte_to_line(10), 2); // 'i'

        assert_eq!(buffer.line_to_byte(0), Some(0));
        assert_eq!(buffer.line_to_byte(1), Some(4));
        assert_eq!(buffer.line_to_byte(2), Some(10));
        assert_eq!(buffer.line_to_byte(3), None);
    }
}
