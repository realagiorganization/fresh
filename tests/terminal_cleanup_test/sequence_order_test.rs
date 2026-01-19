//! Test that verifies the ORDER of escape sequences sent during terminal setup/cleanup.
//!
//! This test captures escape sequences and verifies that keyboard enhancement
//! is pushed AFTER entering alternate screen (not before).
//!
//! Run with: cargo test --release

use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
    Command,
};

/// Capture the ANSI escape sequence for a command
fn capture_ansi<C: Command>(cmd: C) -> String {
    let mut buf = String::new();
    cmd.write_ansi(&mut buf).unwrap();
    buf
}

/// Escape sequence identifiers (for order checking)
const PUSH_KB: &str = "PUSH_KB";
const POP_KB: &str = "POP_KB";
const ENTER_ALT: &str = "ENTER_ALT";
const LEAVE_ALT: &str = "LEAVE_ALT";

/// Identify what type of escape sequence this is
fn identify_sequence(seq: &str) -> Option<&'static str> {
    if seq.contains("[>") && seq.ends_with('u') {
        Some(PUSH_KB)
    } else if seq.contains("[<") && seq.ends_with('u') {
        Some(POP_KB)
    } else if seq.contains("[?1049h") {
        Some(ENTER_ALT)
    } else if seq.contains("[?1049l") {
        Some(LEAVE_ALT)
    } else {
        None
    }
}

/// Simulate the CURRENT (buggy) order from terminal_modes.rs
fn simulate_buggy_enable_order() -> Vec<&'static str> {
    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;

    let mut order = Vec::new();

    // Current order in enable():
    // 1. enable_raw_mode() - no escape sequence
    // 2. PushKeyboardEnhancementFlags - BEFORE alternate screen (BUG!)
    let seq = capture_ansi(PushKeyboardEnhancementFlags(flags));
    if let Some(id) = identify_sequence(&seq) {
        order.push(id);
    }

    // 3. EnterAlternateScreen
    let seq = capture_ansi(EnterAlternateScreen);
    if let Some(id) = identify_sequence(&seq) {
        order.push(id);
    }

    // 4. EnableMouseCapture (not relevant)
    // 5. EnableBracketedPaste (not relevant)

    order
}

/// Simulate the CURRENT (buggy) undo order from terminal_modes.rs
fn simulate_buggy_undo_order() -> Vec<&'static str> {
    let mut order = Vec::new();

    // Current order in undo():
    // 1. DisableMouseCapture (not relevant)
    // 2. DisableBracketedPaste (not relevant)
    // 3. Reset cursor (not relevant)
    // 4. PopKeyboardEnhancementFlags - BEFORE leaving alternate screen
    let seq = capture_ansi(PopKeyboardEnhancementFlags);
    if let Some(id) = identify_sequence(&seq) {
        order.push(id);
    }

    // 5. disable_raw_mode() - no escape sequence
    // 6. LeaveAlternateScreen - AFTER popping KB (this is fine for undo)
    let seq = capture_ansi(LeaveAlternateScreen);
    if let Some(id) = identify_sequence(&seq) {
        order.push(id);
    }

    order
}

/// Simulate the FIXED enable order
fn simulate_fixed_enable_order() -> Vec<&'static str> {
    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;

    let mut order = Vec::new();

    // Fixed order:
    // 1. enable_raw_mode()
    // 2. EnterAlternateScreen FIRST
    let seq = capture_ansi(EnterAlternateScreen);
    if let Some(id) = identify_sequence(&seq) {
        order.push(id);
    }

    // 3. PushKeyboardEnhancementFlags AFTER entering alternate screen
    let seq = capture_ansi(PushKeyboardEnhancementFlags(flags));
    if let Some(id) = identify_sequence(&seq) {
        order.push(id);
    }

    order
}

#[test]
fn test_buggy_order_pushes_kb_before_alternate_screen() {
    let order = simulate_buggy_enable_order();

    println!("Buggy enable order: {:?}", order);

    // The bug: keyboard enhancement is pushed BEFORE entering alternate screen
    assert_eq!(order.len(), 2);
    assert_eq!(order[0], PUSH_KB, "First should be PUSH_KB in buggy order");
    assert_eq!(
        order[1], ENTER_ALT,
        "Second should be ENTER_ALT in buggy order"
    );

    // This is the BUG: PUSH_KB comes before ENTER_ALT
    // This means keyboard enhancement goes to the MAIN screen's stack,
    // not the alternate screen's stack
}

#[test]
fn test_buggy_undo_order() {
    let order = simulate_buggy_undo_order();

    println!("Buggy undo order: {:?}", order);

    // In undo, we pop KB before leaving alternate screen
    // This pops from the ALTERNATE screen's stack (which may be empty)
    // Then we leave alternate screen (returning to main screen with KB still pushed)
    assert_eq!(order.len(), 2);
    assert_eq!(order[0], POP_KB);
    assert_eq!(order[1], LEAVE_ALT);
}

#[test]
fn test_fixed_order_pushes_kb_after_alternate_screen() {
    let order = simulate_fixed_enable_order();

    println!("Fixed enable order: {:?}", order);

    // The fix: enter alternate screen FIRST, then push keyboard enhancement
    assert_eq!(order.len(), 2);
    assert_eq!(
        order[0], ENTER_ALT,
        "First should be ENTER_ALT in fixed order"
    );
    assert_eq!(
        order[1], PUSH_KB,
        "Second should be PUSH_KB in fixed order"
    );
}

#[test]
fn test_sequence_identification() {
    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;

    // Verify we can correctly identify each sequence type
    assert_eq!(
        identify_sequence(&capture_ansi(PushKeyboardEnhancementFlags(flags))),
        Some(PUSH_KB)
    );
    assert_eq!(
        identify_sequence(&capture_ansi(PopKeyboardEnhancementFlags)),
        Some(POP_KB)
    );
    assert_eq!(
        identify_sequence(&capture_ansi(EnterAlternateScreen)),
        Some(ENTER_ALT)
    );
    assert_eq!(
        identify_sequence(&capture_ansi(LeaveAlternateScreen)),
        Some(LEAVE_ALT)
    );
}

fn main() {
    println!("=== Escape Sequence Order Test ===\n");

    println!("Current (BUGGY) enable order:");
    for (i, seq) in simulate_buggy_enable_order().iter().enumerate() {
        println!("  {}. {}", i + 1, seq);
    }

    println!("\nCurrent (BUGGY) undo order:");
    for (i, seq) in simulate_buggy_undo_order().iter().enumerate() {
        println!("  {}. {}", i + 1, seq);
    }

    println!("\nFIXED enable order:");
    for (i, seq) in simulate_fixed_enable_order().iter().enumerate() {
        println!("  {}. {}", i + 1, seq);
    }

    println!("\n=== Analysis ===");
    println!("The bug: In the current code, keyboard enhancement (PUSH_KB) happens");
    println!("BEFORE entering alternate screen (ENTER_ALT).");
    println!("");
    println!("According to the Kitty keyboard protocol, each screen maintains its");
    println!("own independent keyboard mode stack. So:");
    println!("  1. PUSH_KB goes to MAIN screen's stack");
    println!("  2. ENTER_ALT switches to alternate screen (separate stack)");
    println!("  3. ... app runs ...");
    println!("  4. POP_KB pops from ALTERNATE screen's stack (wrong stack!)");
    println!("  5. LEAVE_ALT returns to MAIN screen (KB enhancement still pushed!)");
    println!("");
    println!("The fix: Enter alternate screen FIRST, then push keyboard enhancement.");
    println!("This ensures we push/pop on the same (alternate) screen's stack.");
}
