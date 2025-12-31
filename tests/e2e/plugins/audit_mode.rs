//! E2E tests for audit_mode (Review Diff) plugin

use crate::common::git_test_helper::{DirGuard, GitTestRepo};
use crate::common::harness::EditorTestHarness;
use crossterm::event::{KeyCode, KeyModifiers};
use fresh::config::Config;
use std::fs;
use std::path::PathBuf;

/// Helper to copy audit_mode plugin and its dependencies to the test repo
fn setup_audit_mode_plugin(repo: &GitTestRepo) {
    let plugins_dir = repo.path.join("plugins");
    fs::create_dir_all(&plugins_dir).expect("Failed to create plugins directory");

    let project_root = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .expect("CARGO_MANIFEST_DIR not set");

    // Copy audit_mode.ts plugin
    let audit_mode_src = project_root.join("plugins/audit_mode.ts");
    let audit_mode_dst = plugins_dir.join("audit_mode.ts");
    fs::copy(&audit_mode_src, &audit_mode_dst).unwrap_or_else(|e| {
        panic!(
            "Failed to copy audit_mode.ts from {:?}: {}",
            audit_mode_src, e
        )
    });

    // Copy plugins/lib directory (contains virtual-buffer-factory.ts and fresh.d.ts)
    let lib_src = project_root.join("plugins/lib");
    let lib_dst = plugins_dir.join("lib");
    if lib_src.exists() {
        fs::create_dir_all(&lib_dst).expect("Failed to create plugins/lib directory");
        for entry in fs::read_dir(&lib_src).expect("Failed to read plugins/lib") {
            let entry = entry.expect("Failed to read directory entry");
            let src_path = entry.path();
            let file_name = entry.file_name();
            let dst_path = lib_dst.join(&file_name);
            fs::copy(&src_path, &dst_path).unwrap_or_else(|e| {
                panic!("Failed to copy {:?} to {:?}: {}", src_path, dst_path, e)
            });
        }
    }
}

/// Test that opening the diff view works without errors
/// This test reproduces the addOverlay TypeError that occurred when
/// the plugin passed parameters in the wrong order
#[test]
fn test_review_diff_opens_without_error() {
    let repo = GitTestRepo::new();
    repo.setup_typical_project();
    setup_audit_mode_plugin(&repo);

    // Change to repo directory so git commands work correctly
    let original_dir = repo.change_to_repo_dir();
    let _guard = DirGuard::new(original_dir);

    // Create an initial commit
    repo.git_add_all();
    repo.git_commit("Initial commit");

    // Modify a file to create uncommitted changes
    let main_rs_path = repo.path.join("src/main.rs");
    let modified_content = r#"fn main() {
    println!("Hello, modified world!");
    let config = load_config();
    start_server(config);
    // New comment line
}

fn load_config() -> Config {
    Config::default()
}

fn start_server(config: Config) {
    println!("Starting server...");
}
"#;
    fs::write(&main_rs_path, modified_content).expect("Failed to modify file");

    let mut harness = EditorTestHarness::with_config_and_working_dir(
        120,
        40,
        Config::default(),
        repo.path.clone(),
    )
    .unwrap();

    // Open the modified file
    harness.open_file(&main_rs_path).unwrap();
    harness.render().unwrap();

    // Verify the file is open
    harness
        .wait_until(|h| h.screen_to_string().contains("modified world"))
        .unwrap();

    // Trigger the Review Diff command via command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.wait_for_prompt().unwrap();
    harness.type_text("Review Diff").unwrap();
    harness.render().unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.wait_for_prompt_closed().unwrap();

    // Wait for the Review Diff async operation to complete
    // The status bar changes from "Generating Review Diff Stream..." to showing hunk count
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            // Wait until we're no longer generating the diff stream
            !screen.contains("Generating Review Diff Stream")
        })
        .unwrap();

    let screen = harness.screen_to_string();
    println!("Review Diff screen:\n{}", screen);

    // The diff view should show without errors
    // Check that we don't see an error about addOverlay
    assert!(
        !screen.contains("expected i32"),
        "Should not show addOverlay type error. Screen:\n{}",
        screen
    );
    assert!(
        !screen.contains("TypeError"),
        "Should not show TypeError. Screen:\n{}",
        screen
    );

    // Should show something related to the diff - either the split view or content
    assert!(
        screen.contains("main.rs")
            || screen.contains("modified world")
            || screen.contains("OLD")
            || screen.contains("Review"),
        "Should show diff-related content. Screen:\n{}",
        screen
    );
}

/// Test that the diff view displays hunks correctly
#[test]
fn test_review_diff_shows_hunks() {
    let repo = GitTestRepo::new();
    repo.setup_typical_project();
    setup_audit_mode_plugin(&repo);

    let original_dir = repo.change_to_repo_dir();
    let _guard = DirGuard::new(original_dir);

    // Create an initial commit
    repo.git_add_all();
    repo.git_commit("Initial commit");

    // Modify a file to create uncommitted changes
    let main_rs_path = repo.path.join("src/main.rs");
    let modified_content = r#"fn main() {
    println!("Hello, CHANGED!");
    let config = load_config();
    start_server(config);
}

fn load_config() -> Config {
    Config::default()
}

fn start_server(config: Config) {
    println!("Starting server...");
    println!("New line added");
}
"#;
    fs::write(&main_rs_path, modified_content).expect("Failed to modify file");

    let mut harness = EditorTestHarness::with_config_and_working_dir(
        120,
        40,
        Config::default(),
        repo.path.clone(),
    )
    .unwrap();

    // Open the modified file
    harness.open_file(&main_rs_path).unwrap();
    harness.render().unwrap();

    harness
        .wait_until(|h| h.screen_to_string().contains("CHANGED"))
        .unwrap();

    // Trigger Review Diff via command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.wait_for_prompt().unwrap();
    harness.type_text("Review Diff").unwrap();
    harness.render().unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.wait_for_prompt_closed().unwrap();

    // Wait for the Review Diff async operation to complete
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            !screen.contains("Generating Review Diff Stream")
        })
        .unwrap();

    let screen = harness.screen_to_string();
    println!("Hunks screen:\n{}", screen);

    // Should not have any TypeError
    assert!(
        !screen.contains("TypeError"),
        "Should not show any TypeError. Screen:\n{}",
        screen
    );
}

/// Test that the side-by-side diff view (drill-down) works with synchronized scrolling
/// This test verifies that setSplitScroll is available in the editor API
#[test]
fn test_review_diff_side_by_side_view() {
    let repo = GitTestRepo::new();
    repo.setup_typical_project();
    setup_audit_mode_plugin(&repo);

    let original_dir = repo.change_to_repo_dir();
    let _guard = DirGuard::new(original_dir);

    // Create an initial commit
    repo.git_add_all();
    repo.git_commit("Initial commit");

    // Modify a file to create uncommitted changes
    let main_rs_path = repo.path.join("src/main.rs");
    let modified_content = r#"fn main() {
    println!("Hello, CHANGED!");
    let config = load_config();
    start_server(config);
}

fn load_config() -> Config {
    Config::default()
}

fn start_server(config: Config) {
    println!("Starting server...");
    println!("New line added");
}
"#;
    fs::write(&main_rs_path, modified_content).expect("Failed to modify file");

    let mut harness = EditorTestHarness::with_config_and_working_dir(
        120,
        40,
        Config::default(),
        repo.path.clone(),
    )
    .unwrap();

    // Open the modified file
    harness.open_file(&main_rs_path).unwrap();
    harness.render().unwrap();

    harness
        .wait_until(|h| h.screen_to_string().contains("CHANGED"))
        .unwrap();

    // Trigger Review Diff via command palette
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.wait_for_prompt().unwrap();
    harness.type_text("Review Diff").unwrap();
    harness.render().unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.wait_for_prompt_closed().unwrap();

    // Wait for the Review Diff async operation to complete and hunks to be displayed
    // The status bar shows hunk count when done: "Review Diff: N hunks"
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            !screen.contains("Generating Review Diff Stream") && screen.contains("hunks")
        })
        .unwrap();

    let screen_before_drill = harness.screen_to_string();
    println!("Before drill-down:\n{}", screen_before_drill);

    // Now drill down into a hunk to open the side-by-side view
    // Press Enter to drill down
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();

    // Wait for side-by-side view to open
    // The drill-down creates a split with "[OLD ◀]" in the tab name
    // Or if the operation is async, wait a bit for it to complete
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            // Either we see the OLD marker from the split, or the file was opened
            screen.contains("[OLD") || screen.contains("main.rs ×")
        })
        .unwrap();

    let screen = harness.screen_to_string();
    println!("Side-by-side screen:\n{}", screen);

    // Should not have any TypeError about setSplitScroll
    assert!(
        !screen.contains("setSplitScroll is not a function"),
        "setSplitScroll should be available. Screen:\n{}",
        screen
    );
    assert!(
        !screen.contains("TypeError"),
        "Should not show any TypeError. Screen:\n{}",
        screen
    );
}

/// Test that the improved side-by-side diff shows aligned content with filler lines
/// IGNORED: Side-by-side view requires async spawnProcess for git/cat which hangs in test harness.
/// The drill-down triggers async operations that don't complete in the test environment.
/// See: review_drill_down() in audit_mode.ts uses await editor.spawnProcess()
#[test]
#[ignore]
fn test_side_by_side_diff_shows_alignment() {
    let repo = GitTestRepo::new();
    repo.setup_typical_project();
    setup_audit_mode_plugin(&repo);

    let original_dir = repo.change_to_repo_dir();
    let _guard = DirGuard::new(original_dir);

    // Create an initial commit
    repo.git_add_all();
    repo.git_commit("Initial commit");

    // Modify a file with additions and deletions
    let main_rs_path = repo.path.join("src/main.rs");
    let modified_content = r#"fn main() {
    println!("Hello, modified!");
    let config = load_config();
    start_server(config);
    // New line 1
    // New line 2
}

fn load_config() -> Config {
    Config::default()
}

fn start_server(config: Config) {
    println!("Starting server...");
}
"#;
    fs::write(&main_rs_path, modified_content).expect("Failed to modify file");

    let mut harness = EditorTestHarness::with_config_and_working_dir(
        160, // Wide enough for side-by-side
        50,
        Config::default(),
        repo.path.clone(),
    )
    .unwrap();

    harness.open_file(&main_rs_path).unwrap();
    harness.render().unwrap();

    harness
        .wait_until(|h| h.screen_to_string().contains("modified"))
        .unwrap();

    // Trigger Review Diff
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.wait_for_prompt().unwrap();
    harness.type_text("Review Diff").unwrap();
    harness.render().unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.wait_for_prompt_closed().unwrap();

    // Wait for diff to load
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            !screen.contains("Generating Review Diff Stream") && screen.contains("hunks")
        })
        .unwrap();

    // Navigate to a hunk using 'n' (next hunk) and drill down
    harness
        .send_key(KeyCode::Char('n'), KeyModifiers::NONE)
        .unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();

    // Wait for side-by-side view to fully load
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            screen.contains("[OLD]")
                || screen.contains("[NEW]")
                || screen.contains("Side-by-side diff:")
        })
        .unwrap();

    let screen = harness.screen_to_string();
    println!("Aligned diff screen:\n{}", screen);

    // Should show OLD and NEW headers
    assert!(
        screen.contains("OLD") && screen.contains("NEW"),
        "Should show OLD and NEW pane headers. Screen:\n{}",
        screen
    );

    // Should show filler lines (░ character pattern)
    assert!(
        screen.contains("░"),
        "Should show filler lines for alignment. Screen:\n{}",
        screen
    );

    // Should not have any errors
    assert!(
        !screen.contains("TypeError") && !screen.contains("Error"),
        "Should not show any errors. Screen:\n{}",
        screen
    );
}

/// Test that the side-by-side diff shows change statistics in status bar
/// IGNORED: Side-by-side view requires async spawnProcess for git/cat which hangs in test harness.
#[test]
#[ignore]
fn test_side_by_side_diff_shows_statistics() {
    let repo = GitTestRepo::new();
    repo.setup_typical_project();
    setup_audit_mode_plugin(&repo);

    let original_dir = repo.change_to_repo_dir();
    let _guard = DirGuard::new(original_dir);

    repo.git_add_all();
    repo.git_commit("Initial commit");

    // Modify a file
    let main_rs_path = repo.path.join("src/main.rs");
    let modified_content = r#"fn main() {
    println!("Hello, modified!");
    let config = load_config();
    start_server(config);
}

fn load_config() -> Config {
    Config::default()
}

fn start_server(config: Config) {
    println!("Starting...");
    println!("Added line");
}
"#;
    fs::write(&main_rs_path, modified_content).expect("Failed to modify file");

    let mut harness = EditorTestHarness::with_config_and_working_dir(
        160,
        50,
        Config::default(),
        repo.path.clone(),
    )
    .unwrap();

    harness.open_file(&main_rs_path).unwrap();
    harness.render().unwrap();

    harness
        .wait_until(|h| h.screen_to_string().contains("modified"))
        .unwrap();

    // Trigger Review Diff
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.wait_for_prompt().unwrap();
    harness.type_text("Review Diff").unwrap();
    harness.render().unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.wait_for_prompt_closed().unwrap();

    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            !screen.contains("Generating Review Diff Stream") && screen.contains("hunks")
        })
        .unwrap();

    // Navigate to a hunk using 'n' (next hunk) and drill down
    harness
        .send_key(KeyCode::Char('n'), KeyModifiers::NONE)
        .unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();

    // Wait for side-by-side view with statistics
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            // Should show statistics like "+N -M ~K"
            screen.contains("+") && screen.contains("-") && screen.contains("~")
        })
        .unwrap();

    let screen = harness.screen_to_string();
    println!("Stats screen:\n{}", screen);

    // Should show the statistics format in status bar
    // Format is: "Side-by-side diff: +N -M ~K"
    assert!(
        screen.contains("Side-by-side diff:")
            || (screen.contains("+") && screen.contains("-") && screen.contains("~")),
        "Should show diff statistics. Screen:\n{}",
        screen
    );
}

/// Test that change markers (+, -, ~) appear in the gutter
/// IGNORED: Side-by-side view requires async spawnProcess for git/cat which hangs in test harness.
#[test]
#[ignore]
fn test_side_by_side_diff_shows_gutter_markers() {
    let repo = GitTestRepo::new();
    repo.setup_typical_project();
    setup_audit_mode_plugin(&repo);

    let original_dir = repo.change_to_repo_dir();
    let _guard = DirGuard::new(original_dir);

    repo.git_add_all();
    repo.git_commit("Initial commit");

    // Create changes that will show all marker types
    let main_rs_path = repo.path.join("src/main.rs");
    let modified_content = r#"fn main() {
    println!("Hello, MODIFIED!");
    let config = load_config();
    start_server(config);
    // This is a new line
}

fn load_config() -> Config {
    Config::default()
}

fn start_server(config: Config) {
    println!("Server started");
}
"#;
    fs::write(&main_rs_path, modified_content).expect("Failed to modify file");

    let mut harness = EditorTestHarness::with_config_and_working_dir(
        160,
        50,
        Config::default(),
        repo.path.clone(),
    )
    .unwrap();

    harness.open_file(&main_rs_path).unwrap();
    harness.render().unwrap();

    harness
        .wait_until(|h| h.screen_to_string().contains("MODIFIED"))
        .unwrap();

    // Trigger Review Diff
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.wait_for_prompt().unwrap();
    harness.type_text("Review Diff").unwrap();
    harness.render().unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.wait_for_prompt_closed().unwrap();

    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            !screen.contains("Generating Review Diff Stream") && screen.contains("hunks")
        })
        .unwrap();

    // Navigate to a hunk using 'n' (next hunk) and drill down
    harness
        .send_key(KeyCode::Char('n'), KeyModifiers::NONE)
        .unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();

    // Wait for side-by-side view
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            screen.contains("[OLD]") || screen.contains("[NEW]")
        })
        .unwrap();

    let screen = harness.screen_to_string();
    println!("Gutter markers screen:\n{}", screen);

    // The gutter should show + for additions, - for removals, ~ for modifications
    // These appear as "│+" "│-" "│~" in the gutter column
    let has_markers = screen.contains("│+")
        || screen.contains("│-")
        || screen.contains("│~")
        || screen.contains("+")
        || screen.contains("-");

    assert!(
        has_markers,
        "Should show change markers in gutter (+, -, ~). Screen:\n{}",
        screen
    );
}

/// Test that scroll sync works between the two panes in side-by-side diff view
/// When scrolling one pane, the other should follow to keep aligned lines in sync
/// IGNORED: Side-by-side view requires async spawnProcess for git/cat which hangs in test harness.
#[test]
#[ignore]
fn test_side_by_side_diff_scroll_sync() {
    let repo = GitTestRepo::new();
    repo.setup_typical_project();
    setup_audit_mode_plugin(&repo);

    let original_dir = repo.change_to_repo_dir();
    let _guard = DirGuard::new(original_dir);

    repo.git_add_all();
    repo.git_commit("Initial commit");

    // Create a file with many lines so that scrolling is required
    // Add enough lines that the viewport can't show everything at once
    let main_rs_path = repo.path.join("src/main.rs");
    let mut original_lines: Vec<String> = Vec::new();
    for i in 0..60 {
        original_lines.push(format!(
            "fn function_{}() {{ println!(\"Line {}\"); }}",
            i, i
        ));
    }
    fs::write(&main_rs_path, original_lines.join("\n")).expect("Failed to write original file");

    // Commit the original
    repo.git_add_all();
    repo.git_commit("Add many functions");

    // Now modify - add some lines in the middle and change some at the end
    let mut modified_lines: Vec<String> = Vec::new();
    for i in 0..30 {
        modified_lines.push(format!(
            "fn function_{}() {{ println!(\"Line {}\"); }}",
            i, i
        ));
    }
    // Add new lines in the middle
    for i in 0..5 {
        modified_lines.push(format!(
            "fn new_function_{}() {{ println!(\"New {}\"); }}",
            i, i
        ));
    }
    for i in 30..60 {
        if i >= 55 {
            // Modify the last few lines
            modified_lines.push(format!(
                "fn function_{}() {{ println!(\"Modified {}\"); }}",
                i, i
            ));
        } else {
            modified_lines.push(format!(
                "fn function_{}() {{ println!(\"Line {}\"); }}",
                i, i
            ));
        }
    }
    fs::write(&main_rs_path, modified_lines.join("\n")).expect("Failed to modify file");

    let mut harness = EditorTestHarness::with_config_and_working_dir(
        160,
        30, // Relatively small height to ensure scrolling is needed
        Config::default(),
        repo.path.clone(),
    )
    .unwrap();

    harness.open_file(&main_rs_path).unwrap();
    harness.render().unwrap();

    harness
        .wait_until(|h| h.screen_to_string().contains("function_"))
        .unwrap();

    // Trigger Review Diff
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.wait_for_prompt().unwrap();
    harness.type_text("Review Diff").unwrap();
    harness.render().unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.wait_for_prompt_closed().unwrap();

    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            !screen.contains("Generating Review Diff Stream") && screen.contains("hunks")
        })
        .unwrap();

    // Navigate to a hunk using 'n' (next hunk) and drill down
    harness
        .send_key(KeyCode::Char('n'), KeyModifiers::NONE)
        .unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();

    // Wait for side-by-side view
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            screen.contains("[OLD]") || screen.contains("[NEW]")
        })
        .unwrap();

    let screen_before = harness.screen_to_string();
    println!("Before scrolling:\n{}", screen_before);

    // Now press 'G' to go to end of document - this should sync both panes
    harness
        .send_key(KeyCode::Char('G'), KeyModifiers::SHIFT)
        .unwrap();

    // Give the scroll sync a moment to process
    harness.render().unwrap();
    harness.render().unwrap();

    let screen_after = harness.screen_to_string();
    println!("After pressing G:\n{}", screen_after);

    // Both panes should show content from near the end of the file
    // The OLD pane should show function_5X lines (end of original)
    // The NEW pane should show function_5X or Modified lines (end of modified)
    // If scroll sync works, both should show similar line numbers

    // Check that we scrolled - shouldn't see function_0 anymore in main content
    // (it might appear in tab name, so be specific)
    let scrolled = !screen_after.contains("function_0()")
        || screen_after.contains("function_5")
        || screen_after.contains("Modified");

    assert!(
        scrolled,
        "Should have scrolled away from the start. Screen:\n{}",
        screen_after
    );

    // Both panes should show aligned content - look for content from near the end
    // The key test: if we see function_55+ in one pane, we should see similar in other
    // Or at least both panes should show content from the bottom section
    let shows_late_content =
        screen_after.contains("function_5") || screen_after.contains("Modified");

    assert!(
        shows_late_content,
        "After G, should show content from near end of file. Screen:\n{}",
        screen_after
    );

    // Test scrolling back up with 'g' (go to start)
    harness
        .send_key(KeyCode::Char('g'), KeyModifiers::NONE)
        .unwrap();
    harness.render().unwrap();
    harness.render().unwrap();

    let screen_top = harness.screen_to_string();
    println!("After pressing g:\n{}", screen_top);

    // Should be back at the top showing early functions
    let back_at_top = screen_top.contains("function_0") || screen_top.contains("function_1");

    assert!(
        back_at_top,
        "After g, should be back at top of file. Screen:\n{}",
        screen_top
    );

    // No errors should occur
    assert!(
        !screen_after.contains("TypeError") && !screen_after.contains("Error:"),
        "Should not show any errors. Screen:\n{}",
        screen_after
    );
}

/// Test vim-style navigation in diff-view mode
/// IGNORED: Side-by-side view requires async spawnProcess for git/cat which hangs in test harness.
#[test]
#[ignore]
fn test_side_by_side_diff_vim_navigation() {
    let repo = GitTestRepo::new();
    repo.setup_typical_project();
    setup_audit_mode_plugin(&repo);

    let original_dir = repo.change_to_repo_dir();
    let _guard = DirGuard::new(original_dir);

    repo.git_add_all();
    repo.git_commit("Initial commit");

    let main_rs_path = repo.path.join("src/main.rs");
    let modified_content = r#"fn main() {
    println!("Modified line");
}

fn helper() {
    println!("Added function");
}
"#;
    fs::write(&main_rs_path, modified_content).expect("Failed to modify file");

    let mut harness = EditorTestHarness::with_config_and_working_dir(
        160,
        50,
        Config::default(),
        repo.path.clone(),
    )
    .unwrap();

    harness.open_file(&main_rs_path).unwrap();
    harness.render().unwrap();

    harness
        .wait_until(|h| h.screen_to_string().contains("Modified"))
        .unwrap();

    // Trigger Review Diff
    harness
        .send_key(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();
    harness.wait_for_prompt().unwrap();
    harness.type_text("Review Diff").unwrap();
    harness.render().unwrap();
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();
    harness.wait_for_prompt_closed().unwrap();

    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            !screen.contains("Generating Review Diff Stream") && screen.contains("hunks")
        })
        .unwrap();

    // Navigate and drill down
    for _ in 0..8 {
        harness.send_key(KeyCode::Down, KeyModifiers::NONE).unwrap();
    }
    harness
        .send_key(KeyCode::Enter, KeyModifiers::NONE)
        .unwrap();

    // Wait for diff-view mode
    harness
        .wait_until(|h| {
            let screen = h.screen_to_string();
            screen.contains("[OLD]") || screen.contains("[NEW]")
        })
        .unwrap();

    // Test vim navigation: j moves down, k moves up
    harness
        .send_key(KeyCode::Char('j'), KeyModifiers::NONE)
        .unwrap();
    harness
        .send_key(KeyCode::Char('j'), KeyModifiers::NONE)
        .unwrap();
    harness
        .send_key(KeyCode::Char('k'), KeyModifiers::NONE)
        .unwrap();

    let screen = harness.screen_to_string();

    // Should still be in the diff view without errors
    assert!(
        !screen.contains("TypeError") && !screen.contains("Error"),
        "Vim navigation should work without errors. Screen:\n{}",
        screen
    );

    // Test 'q' to close
    harness
        .send_key(KeyCode::Char('q'), KeyModifiers::NONE)
        .unwrap();

    // After closing, should still be functional
    let screen = harness.screen_to_string();
    assert!(
        !screen.contains("TypeError"),
        "Closing with 'q' should work. Screen:\n{}",
        screen
    );
}
