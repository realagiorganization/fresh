use crate::common::harness::EditorTestHarness;
use crossterm::event::{KeyCode, KeyModifiers};

/// Test sorting selected lines alphabetically
#[test]
fn test_sort_lines_basic() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type multiple lines in unsorted order
    harness.type_text("cherry\napple\nbanana").unwrap();

    // Select all lines with Ctrl+A
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Open command palette with Ctrl+P
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Verify the lines are sorted
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "apple\nbanana\ncherry",
        "Lines should be sorted alphabetically"
    );
}

/// Test that sort lines requires selection (single line should not change)
#[test]
fn test_sort_lines_single_line_no_change() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type a single line
    harness.type_text("hello world").unwrap();

    // Select all
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Single line should remain unchanged
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "hello world",
        "Single line should remain unchanged after sort"
    );
}

/// Test sorting lines with numbers
#[test]
fn test_sort_lines_with_numbers() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type lines with numbers (alphabetic sort, not numeric)
    harness.type_text("10 items\n2 items\n1 item").unwrap();

    // Select all
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Alphabetic sort: "1" < "10" < "2"
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "1 item\n10 items\n2 items",
        "Lines should be sorted alphabetically (not numerically)"
    );
}

/// Test sorting lines preserves trailing newline
#[test]
fn test_sort_lines_preserves_trailing_newline() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type lines ending with newline
    harness.type_text("zebra\napple\nmango\n").unwrap();

    // Select all
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Should preserve trailing newline
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "apple\nmango\nzebra\n",
        "Trailing newline should be preserved"
    );
}

/// Test undo after sorting lines
#[test]
fn test_sort_lines_undo() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type unsorted lines
    harness.type_text("cherry\napple\nbanana").unwrap();

    // Select all
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Verify sorted
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(buffer_content, "apple\nbanana\ncherry");

    // Undo with Ctrl+Z
    harness
        .send_key(KeyCode::Char('z'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Should be back to original order
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "cherry\napple\nbanana",
        "Undo should restore original line order"
    );
}

/// Test sorting partial selection (only selected lines)
#[test]
fn test_sort_lines_partial_selection() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type multiple lines
    harness
        .type_text("first\nzebra\napple\nmango\nlast")
        .unwrap();

    // Move to start of file
    harness
        .send_key(KeyCode::Home, KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Move down to second line (zebra)
    harness.send_key(KeyCode::Down, KeyModifiers::NONE).unwrap();
    harness.render().unwrap();

    // Select the line (Ctrl+L)
    harness
        .send_key(KeyCode::Char('l'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Extend selection down to include next two lines (apple, mango)
    harness
        .send_key(KeyCode::Down, KeyModifiers::SHIFT)
        .unwrap();
    harness
        .send_key(KeyCode::Down, KeyModifiers::SHIFT)
        .unwrap();
    harness.render().unwrap();

    // Verify selection
    let selected = harness.get_selected_text();
    assert!(
        selected.contains("zebra") && selected.contains("apple") && selected.contains("mango"),
        "Should have zebra, apple, mango selected, got: {}",
        selected
    );

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Only the middle lines should be sorted
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "first\napple\nmango\nzebra\nlast",
        "Only selected lines should be sorted"
    );
}

/// Test sorting already sorted lines (should be no change)
#[test]
fn test_sort_lines_already_sorted() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type already sorted lines
    harness.type_text("apple\nbanana\ncherry").unwrap();

    // Select all
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Should remain the same
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "apple\nbanana\ncherry",
        "Already sorted lines should remain unchanged"
    );
}

/// Test sorting lines with mixed case (case-sensitive sort)
#[test]
fn test_sort_lines_case_sensitive() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type mixed case lines
    harness.type_text("Banana\napple\nCherry").unwrap();

    // Select all
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Case-sensitive sort: uppercase letters come before lowercase in ASCII
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "Banana\nCherry\napple",
        "Sort should be case-sensitive (uppercase before lowercase)"
    );
}

/// Test sorting lines with empty lines
#[test]
fn test_sort_lines_with_empty_lines() {
    let mut harness = EditorTestHarness::new(80, 24).unwrap();

    // Type lines with empty lines
    harness.type_text("cherry\n\napple\n\nbanana").unwrap();

    // Select all
    harness
        .send_key(KeyCode::Char('a'), KeyModifiers::CONTROL)
        .unwrap();
    harness.render().unwrap();

    // Open command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness
        .wait_until(|h| h.screen_to_string().contains(">command"))
        .unwrap();

    // Search for sort lines command
    harness.type_text("sort lines").unwrap();
    harness.render().unwrap();

    // Execute the command
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();

    // Empty lines should sort to the top
    let buffer_content = harness.get_buffer_content().unwrap();
    assert_eq!(
        buffer_content, "\n\napple\nbanana\ncherry",
        "Empty lines should sort to the beginning"
    );
}
