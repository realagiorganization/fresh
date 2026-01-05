//! Event adapter for converting Ratzilla events to crossterm-compatible events
//!
//! The Fresh editor uses crossterm's event types internally. This module provides
//! conversion functions to translate Ratzilla's browser-based events to crossterm format.

use ratzilla::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

/// Crossterm-compatible key event for WASM builds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmKeyEvent {
    pub code: WasmKeyCode,
    pub modifiers: WasmKeyModifiers,
}

/// Crossterm-compatible key code for WASM builds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmKeyCode {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    F(u8),
}

bitflags::bitflags! {
    /// Crossterm-compatible key modifiers for WASM builds
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct WasmKeyModifiers: u8 {
        const NONE = 0b0000_0000;
        const SHIFT = 0b0000_0001;
        const CONTROL = 0b0000_0010;
        const ALT = 0b0000_0100;
    }
}

/// Crossterm-compatible mouse event for WASM builds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmMouseEvent {
    pub kind: WasmMouseEventKind,
    pub column: u16,
    pub row: u16,
    pub modifiers: WasmKeyModifiers,
}

/// Crossterm-compatible mouse button for WASM builds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmMouseButton {
    Left,
    Right,
    Middle,
}

/// Crossterm-compatible mouse event kind for WASM builds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmMouseEventKind {
    Down(WasmMouseButton),
    Up(WasmMouseButton),
    Drag(WasmMouseButton),
    Moved,
    ScrollUp,
    ScrollDown,
}

impl WasmKeyEvent {
    pub fn new(code: WasmKeyCode, modifiers: WasmKeyModifiers) -> Self {
        Self { code, modifiers }
    }
}

/// Convert Ratzilla KeyEvent to WASM-compatible KeyEvent
pub fn convert_key_event(event: &KeyEvent) -> Option<WasmKeyEvent> {
    let code = convert_key_code(&event.code)?;

    let mut modifiers = WasmKeyModifiers::NONE;
    if event.ctrl {
        modifiers |= WasmKeyModifiers::CONTROL;
    }
    if event.alt {
        modifiers |= WasmKeyModifiers::ALT;
    }
    if event.shift {
        modifiers |= WasmKeyModifiers::SHIFT;
    }

    Some(WasmKeyEvent::new(code, modifiers))
}

fn convert_key_code(code: &KeyCode) -> Option<WasmKeyCode> {
    Some(match code {
        KeyCode::Char(c) => WasmKeyCode::Char(*c),
        KeyCode::Enter => WasmKeyCode::Enter,
        KeyCode::Tab => WasmKeyCode::Tab,
        KeyCode::Backspace => WasmKeyCode::Backspace,
        KeyCode::Delete => WasmKeyCode::Delete,
        KeyCode::Esc => WasmKeyCode::Esc,
        KeyCode::Up => WasmKeyCode::Up,
        KeyCode::Down => WasmKeyCode::Down,
        KeyCode::Left => WasmKeyCode::Left,
        KeyCode::Right => WasmKeyCode::Right,
        KeyCode::Home => WasmKeyCode::Home,
        KeyCode::End => WasmKeyCode::End,
        KeyCode::PageUp => WasmKeyCode::PageUp,
        KeyCode::PageDown => WasmKeyCode::PageDown,
        KeyCode::F(n) => WasmKeyCode::F(*n),
        KeyCode::Unidentified => return None,
    })
}

/// Convert Ratzilla MouseEvent to WASM-compatible MouseEvent
pub fn convert_mouse_event(event: &MouseEvent) -> Option<WasmMouseEvent> {
    let button = convert_mouse_button(&event.button);
    let kind = convert_mouse_event_kind(&event.event, button)?;

    let mut modifiers = WasmKeyModifiers::NONE;
    if event.ctrl {
        modifiers |= WasmKeyModifiers::CONTROL;
    }
    if event.alt {
        modifiers |= WasmKeyModifiers::ALT;
    }
    if event.shift {
        modifiers |= WasmKeyModifiers::SHIFT;
    }

    Some(WasmMouseEvent {
        kind,
        column: event.x as u16,
        row: event.y as u16,
        modifiers,
    })
}

fn convert_mouse_button(button: &MouseButton) -> WasmMouseButton {
    match button {
        MouseButton::Left => WasmMouseButton::Left,
        MouseButton::Right => WasmMouseButton::Right,
        MouseButton::Middle => WasmMouseButton::Middle,
        MouseButton::Back | MouseButton::Forward | MouseButton::Unidentified => {
            WasmMouseButton::Left // Default for unhandled buttons
        }
    }
}

fn convert_mouse_event_kind(
    kind: &MouseEventKind,
    button: WasmMouseButton,
) -> Option<WasmMouseEventKind> {
    Some(match kind {
        MouseEventKind::Pressed => WasmMouseEventKind::Down(button),
        MouseEventKind::Released => WasmMouseEventKind::Up(button),
        MouseEventKind::Moved => WasmMouseEventKind::Moved,
        MouseEventKind::Unidentified => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_key_event() {
        let event = KeyEvent {
            code: KeyCode::Char('a'),
            ctrl: true,
            alt: false,
            shift: false,
        };
        let result = convert_key_event(&event).unwrap();
        assert_eq!(result.code, WasmKeyCode::Char('a'));
        assert!(result.modifiers.contains(WasmKeyModifiers::CONTROL));
    }

    #[test]
    fn test_convert_mouse_event() {
        let event = MouseEvent {
            button: MouseButton::Left,
            event: MouseEventKind::Down,
            x: 10,
            y: 20,
            ctrl: false,
            alt: false,
            shift: true,
        };
        let result = convert_mouse_event(&event).unwrap();
        assert_eq!(result.column, 10);
        assert_eq!(result.row, 20);
        assert!(result.modifiers.contains(WasmKeyModifiers::SHIFT));
    }
}
