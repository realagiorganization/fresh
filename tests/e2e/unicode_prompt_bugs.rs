//! E2E tests for Unicode/multi-byte character bugs in prompts
//!
//! These tests reproduce critical bugs where the editor crashes when handling
//! multi-byte UTF-8 characters in input prompts.

use crate::common::harness::EditorTestHarness;
use crossterm::event::{KeyCode, KeyModifiers};

// =============================================================================
// Bug #472: Turkish character backspace crashes Fresh
// https://github.com/sinelaw/fresh/issues/472
//
// Steps to reproduce:
// 1. Press Ctrl+F (search prompt)
// 2. Type 'ş' (Turkish s-cedilla, 2 bytes in UTF-8)
// 3. Press Backspace
// 4. Crash: "byte index 1 is not a char boundary; it is inside 'ş' (bytes 0..2)"
//
// Root cause: The prompt's backspace() function uses `self.cursor_pos - 1` as
// the byte index for String::remove(), but cursor_pos is incremented by the
// character's byte length (2 for 'ş'), so cursor_pos - 1 = 1 is in the middle
// of the 2-byte character.
// =============================================================================

/// Test backspace on Turkish character 'ş' in search prompt
/// Bug #472: Should not crash when pressing backspace after typing multi-byte char
#[test]
fn test_bug_472_turkish_char_backspace_in_search() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open search prompt
    harness
        .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Verify search prompt is open
    harness.assert_screen_contains("Search:");

    // Type Turkish character 'ş' (2 bytes in UTF-8: 0xC5 0x9F)
    harness.type_text("ş").unwrap();
    harness.render().unwrap();

    // Verify the character appears in the prompt
    harness.assert_screen_contains("Search: ş");

    // Press backspace - this should NOT crash
    // Bug: Currently panics with "byte index 1 is not a char boundary"
    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // After backspace, the prompt should be empty
    harness.assert_screen_contains("Search:");
    // The 'ş' should be gone
    harness.assert_screen_not_contains("ş");
}

/// Test backspace on various multi-byte characters in search prompt
/// Ensures the fix works for all UTF-8 multi-byte characters, not just Turkish
#[test]
fn test_multibyte_char_backspace_in_search() {
    let test_chars = [
        ("ş", "Turkish s-cedilla (2 bytes)"),
        ("ç", "French c-cedilla (2 bytes)"),
        ("ñ", "Spanish n-tilde (2 bytes)"),
        ("ü", "German u-umlaut (2 bytes)"),
        ("中", "Chinese character (3 bytes)"),
        ("日", "Japanese character (3 bytes)"),
        ("🚀", "Rocket emoji (4 bytes)"),
        ("😀", "Smiley emoji (4 bytes)"),
        ("©", "Copyright symbol (2 bytes)"),
    ];

    for (ch, description) in test_chars {
        let mut harness = EditorTestHarness::new(80, 24).unwrap();

        // Open search prompt
        harness
            .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
            .unwrap();
        harness.render().unwrap();

        // Type the multi-byte character
        harness.type_text(ch).unwrap();
        harness.render().unwrap();

        // Verify the character appears
        let expected = format!("Search: {}", ch);
        assert!(
            harness.screen_to_string().contains(&expected),
            "Failed for {}: expected '{}' in screen",
            description,
            expected
        );

        // Press backspace - should NOT crash
        harness
            .send_key(KeyCode::Backspace, KeyModifiers::NONE)
            .unwrap();
        harness.render().unwrap();

        // Verify the character was deleted
        let screen = harness.screen_to_string();
        assert!(
            !screen.contains(ch),
            "Failed for {}: character '{}' should be deleted after backspace",
            description,
            ch
        );

        // Close the prompt
        harness.send_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
    }
}

/// Test cursor movement with multi-byte characters in prompt
/// The cursor_left/cursor_right functions also have byte-index issues
#[test]
fn test_cursor_movement_with_multibyte_chars_in_prompt() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open search prompt
    harness
        .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Type "aşb" - ASCII, Turkish, ASCII
    harness.type_text("aşb").unwrap();
    harness.render().unwrap();

    // Verify content
    harness.assert_screen_contains("Search: aşb");

    // Move cursor left (from end, past 'b')
    harness.send_key(KeyCode::Left, KeyModifiers::NONE).unwrap();

    // Move cursor left again (past 'ş' - 2 bytes)
    // This should not crash
    harness.send_key(KeyCode::Left, KeyModifiers::NONE).unwrap();

    // Move cursor left again (past 'a')
    harness.send_key(KeyCode::Left, KeyModifiers::NONE).unwrap();

    // Now we're at the start - type a character to verify cursor position
    harness.type_text("X").unwrap();
    harness.render().unwrap();

    // Should have "Xaşb"
    harness.assert_screen_contains("Search: Xaşb");
}

/// Test delete (forward delete) with multi-byte characters
#[test]
fn test_delete_with_multibyte_chars_in_prompt() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open search prompt
    harness
        .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Type "şa" - Turkish char followed by ASCII
    harness.type_text("şa").unwrap();
    harness.render().unwrap();

    // Move to start
    harness.send_key(KeyCode::Home, KeyModifiers::NONE).unwrap();

    // Delete the 'ş' (forward delete) - should delete entire character, not just 1 byte
    harness
        .send_key(KeyCode::Delete, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Should have just "a" left
    harness.assert_screen_contains("Search: a");
    harness.assert_screen_not_contains("ş");
}

/// Test multiple multi-byte characters in sequence
#[test]
fn test_multiple_multibyte_chars_backspace_sequence() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open search prompt
    harness
        .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Type "şçü" - three 2-byte Turkish/German characters
    harness.type_text("şçü").unwrap();
    harness.render().unwrap();

    // Verify content
    harness.assert_screen_contains("Search: şçü");

    // Backspace three times - each should delete one character
    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();
    harness.assert_screen_contains("Search: şç");

    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();
    harness.assert_screen_contains("Search: ş");

    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Prompt should be empty now
    let screen = harness.screen_to_string();
    assert!(
        !screen.contains("ş") && !screen.contains("ç") && !screen.contains("ü"),
        "All characters should be deleted"
    );
}

/// Test backspace in command palette (not just search)
#[test]
fn test_multibyte_backspace_in_command_palette() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Type a multi-byte character
    harness.type_text("日本語").unwrap();
    harness.render().unwrap();

    // Verify content
    harness.assert_screen_contains("Command: 日本語");

    // Backspace should delete one character at a time
    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();
    harness.assert_screen_contains("Command: 日本");

    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();
    harness.assert_screen_contains("Command: 日");

    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // All Japanese characters should be deleted
    let screen = harness.screen_to_string();
    assert!(
        !screen.contains("日") && !screen.contains("本") && !screen.contains("語"),
        "All Japanese characters should be deleted"
    );
}

/// Test backspace in replace prompt
#[test]
fn test_multibyte_backspace_in_replace_prompt() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open replace prompt
    harness
        .send_key(KeyCode::Char('r'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Type a multi-byte character
    harness.type_text("ñ").unwrap();
    harness.render().unwrap();

    // Verify content
    harness.assert_screen_contains("Replace: ñ");

    // Backspace should work
    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    harness.assert_screen_not_contains("ñ");
}

// =============================================================================
// Bug #466: Panic on Unicode character deletion in settings
// https://github.com/sinelaw/fresh/issues/466
//
// Steps to reproduce:
// 1. Open settings (Ctrl+,)
// 2. Navigate to key mapping configuration
// 3. Press Alt+G (produces © character in the Key field)
// 4. Press Delete or Backspace
// 5. Crash: "assertion failed: self.is_char_boundary(idx)"
//
// Root cause: Same issue as #472 - text_input uses byte indices instead of
// character-aware indexing for cursor movement and deletion.
// =============================================================================

/// Test backspace on multi-byte character in settings text input
/// Bug #466: Should not crash when deleting multi-byte chars in settings
#[test]
fn test_bug_466_unicode_deletion_in_settings() {
    let mut harness = EditorTestHarness::new(100, 40).unwrap();

    // Open settings
    harness
        .send_key(KeyCode::Char(','), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Verify settings is open
    harness.assert_screen_contains("Settings");

    // Search for "keybinding" to find a text input field
    harness
        .send_key(KeyCode::Char('/'), KeyModifiers::NONE)
        .unwrap();
    harness.type_text("keybinding maps").unwrap();
    harness.render().unwrap();

    // Jump to the keybinding maps section
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Look for "[+] Add new" which indicates we're in a map control
    if harness.screen_to_string().contains("[+] Add new") {
        // Press Enter to start adding a new entry
        harness
            .send_key(KeyCode::Enter, KeyModifiers::NONE)
            .unwrap();
        harness.render().unwrap();

        // Type a multi-byte character (like the © from Alt+G)
        harness.type_text("©").unwrap();
        harness.render().unwrap();

        // Verify the character appears
        harness.assert_screen_contains("©");

        // Press Backspace - this should NOT crash
        harness
            .send_key(KeyCode::Backspace, KeyModifiers::NONE)
            .unwrap();
        harness.render().unwrap();

        // The character should be deleted
        harness.assert_screen_not_contains("©");
    }

    // Close settings
    harness.send_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
    harness.send_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
}

/// Test multi-byte character handling in settings number input
/// Number inputs use the same text_input widget, so they have the same bug
#[test]
fn test_multibyte_in_settings_number_input() {
    let mut harness = EditorTestHarness::new(100, 40).unwrap();

    // Open settings
    harness
        .send_key(KeyCode::Char(','), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Search for "hover delay" which is a number input
    harness
        .send_key(KeyCode::Char('/'), KeyModifiers::NONE)
        .unwrap();
    harness.type_text("hover delay").unwrap();
    harness.render().unwrap();

    // Jump to result
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Start editing mode
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Select all and clear
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .send_key(KeyCode::Delete, KeyModifiers::NONE)
        .unwrap();

    // Type a multi-byte character (this is invalid for a number input,
    // but it shouldn't crash - just not accept it or show an error)
    harness.type_text("ñ").unwrap();
    harness.render().unwrap();

    // Try to delete it - should not crash
    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Cancel editing
    harness.send_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();

    // Close settings
    harness.send_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
}

// =============================================================================
// Mixed multi-byte and ASCII character tests
// These ensure that the cursor position is correctly tracked when mixing
// single-byte ASCII and multi-byte UTF-8 characters
// =============================================================================

/// Test inserting ASCII between multi-byte characters
#[test]
fn test_insert_ascii_between_multibyte_chars() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open search prompt
    harness
        .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Type "你好" (two 3-byte Chinese characters)
    harness.type_text("你好").unwrap();
    harness.render().unwrap();

    // Move cursor left once (between 你 and 好)
    harness.send_key(KeyCode::Left, KeyModifiers::NONE).unwrap();

    // Insert ASCII character
    harness.type_text("X").unwrap();
    harness.render().unwrap();

    // Should have "你X好"
    harness.assert_screen_contains("Search: 你X好");
}

/// Test cursor movement and deletion in mixed content
#[test]
fn test_mixed_content_cursor_and_delete() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open search prompt
    harness
        .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Type "a中b文c" (alternating ASCII and Chinese)
    harness.type_text("a中b文c").unwrap();
    harness.render().unwrap();

    harness.assert_screen_contains("Search: a中b文c");

    // Move to start
    harness.send_key(KeyCode::Home, KeyModifiers::NONE).unwrap();

    // Delete each character one by one from the front
    // This tests forward delete on mixed content

    harness
        .send_key(KeyCode::Delete, KeyModifiers::NONE)
        .unwrap(); // delete 'a'
    harness.render().unwrap();
    harness.assert_screen_contains("Search: 中b文c");

    harness
        .send_key(KeyCode::Delete, KeyModifiers::NONE)
        .unwrap(); // delete '中'
    harness.render().unwrap();
    harness.assert_screen_contains("Search: b文c");

    harness
        .send_key(KeyCode::Delete, KeyModifiers::NONE)
        .unwrap(); // delete 'b'
    harness.render().unwrap();
    harness.assert_screen_contains("Search: 文c");

    harness
        .send_key(KeyCode::Delete, KeyModifiers::NONE)
        .unwrap(); // delete '文'
    harness.render().unwrap();
    harness.assert_screen_contains("Search: c");

    harness
        .send_key(KeyCode::Delete, KeyModifiers::NONE)
        .unwrap(); // delete 'c'
    harness.render().unwrap();

    // Prompt should be empty
    let screen = harness.screen_to_string();
    assert!(
        !screen.contains("a")
            && !screen.contains("中")
            && !screen.contains("b")
            && !screen.contains("文")
            && !screen.contains("c"),
        "All characters should be deleted"
    );
}

/// Test Home and End keys with multi-byte content
#[test]
fn test_home_end_with_multibyte_chars_in_prompt() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Open search prompt
    harness
        .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Type multi-byte content
    harness.type_text("日本語").unwrap();
    harness.render().unwrap();

    // Move to start with Home
    harness.send_key(KeyCode::Home, KeyModifiers::NONE).unwrap();

    // Type at start
    harness.type_text("X").unwrap();
    harness.render().unwrap();

    harness.assert_screen_contains("Search: X日本語");

    // Move to end with End
    harness.send_key(KeyCode::End, KeyModifiers::NONE).unwrap();

    // Type at end
    harness.type_text("Y").unwrap();
    harness.render().unwrap();

    harness.assert_screen_contains("Search: X日本語Y");
}

/// Regression test: Ensure bug doesn't reoccur after fix
/// This is the exact reproduction from the issue report
#[test]
fn test_bug_472_exact_reproduction() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // 1. Press Ctrl+F (search prompt)
    harness
        .send_key(KeyCode::Char('f'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // 2. Type 'ş' (Turkish s-cedilla)
    harness.type_text("ş").unwrap();
    harness.render().unwrap();

    // 3. Press Backspace
    // This was causing: "byte index 1 is not a char boundary; it is inside 'ş' (bytes 0..2)"
    harness
        .send_key(KeyCode::Backspace, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // If we get here without panicking, the bug is fixed
    // Verify the prompt is now empty
    let screen = harness.screen_to_string();
    assert!(
        !screen.contains("ş"),
        "Turkish character should be deleted after backspace"
    );
}
