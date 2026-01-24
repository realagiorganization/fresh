//! Integration tests for theme validation.
//!
//! These tests verify that:
//! 1. All builtin themes pass strict validation
//! 2. All theme files in the themes/ directory pass strict validation
//! 3. Strict validation correctly rejects unknown fields
//! 4. Strict validation correctly rejects type mismatches

use fresh::view::theme::{
    validate_theme_file, validate_theme_json, ValidationErrorKind, BUILTIN_THEMES,
};
use std::path::PathBuf;

/// Get the path to the themes directory.
fn themes_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")
}

#[test]
fn test_all_builtin_themes_pass_strict_validation() {
    for theme in BUILTIN_THEMES {
        let result = validate_theme_json(theme.json);
        assert!(
            result.is_valid,
            "Builtin theme '{}' should pass strict validation.\nErrors: {:#?}",
            theme.name, result.errors
        );
    }
}

#[test]
fn test_all_theme_files_pass_strict_validation() {
    let themes_path = themes_dir();

    // Skip if themes directory doesn't exist (e.g., in minimal builds)
    if !themes_path.exists() {
        eprintln!(
            "Skipping test: themes directory not found at {}",
            themes_path.display()
        );
        return;
    }

    let entries = std::fs::read_dir(&themes_path).expect("Failed to read themes directory");

    let mut theme_count = 0;
    for entry in entries {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        // Only validate .json files
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        theme_count += 1;
        let result = validate_theme_file(&path).expect("Failed to read theme file");
        assert!(
            result.is_valid,
            "Theme file '{}' should pass strict validation.\nErrors: {:#?}",
            path.display(),
            result.errors
        );
    }

    assert!(
        theme_count > 0,
        "Expected to find at least one theme file in {}",
        themes_path.display()
    );
    eprintln!("Validated {} theme files successfully", theme_count);
}

#[test]
fn test_strict_validation_rejects_unknown_fields() {
    // Test unknown field at root level
    let json_root = r#"{
        "name": "test",
        "editor": {},
        "ui": {},
        "search": {},
        "diagnostic": {},
        "syntax": {},
        "completely_unknown_field": "should fail"
    }"#;

    let result = validate_theme_json(json_root);
    assert!(
        !result.is_valid,
        "Should reject unknown field at root level"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.kind == ValidationErrorKind::UnknownField),
        "Should identify as unknown field error. Errors: {:#?}",
        result.errors
    );

    // Test unknown field in nested object (editor)
    let json_nested_editor = r#"{
        "name": "test",
        "editor": {
            "bg": [30, 30, 30],
            "not_a_real_editor_field": true
        },
        "ui": {},
        "search": {},
        "diagnostic": {},
        "syntax": {}
    }"#;

    let result = validate_theme_json(json_nested_editor);
    assert!(
        !result.is_valid,
        "Should reject unknown field in editor section"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.kind == ValidationErrorKind::UnknownField),
        "Should identify as unknown field error. Errors: {:#?}",
        result.errors
    );

    // Test unknown field in UI section
    let json_nested_ui = r#"{
        "name": "test",
        "editor": {},
        "ui": {
            "fake_ui_color": "Red"
        },
        "search": {},
        "diagnostic": {},
        "syntax": {}
    }"#;

    let result = validate_theme_json(json_nested_ui);
    assert!(
        !result.is_valid,
        "Should reject unknown field in ui section"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.kind == ValidationErrorKind::UnknownField),
        "Should identify as unknown field error. Errors: {:#?}",
        result.errors
    );
}

#[test]
fn test_strict_validation_rejects_type_mismatch() {
    // Test wrong type for name (should be string)
    let json_wrong_name_type = r#"{
        "name": 12345,
        "editor": {},
        "ui": {},
        "search": {},
        "diagnostic": {},
        "syntax": {}
    }"#;

    let result = validate_theme_json(json_wrong_name_type);
    assert!(!result.is_valid, "Should reject wrong type for 'name'");
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.kind == ValidationErrorKind::TypeMismatch),
        "Should identify as type mismatch error. Errors: {:#?}",
        result.errors
    );

    // Test wrong type for color (should be array or string, not object)
    let json_wrong_color_type = r#"{
        "name": "test",
        "editor": {
            "bg": {"invalid": "object"}
        },
        "ui": {},
        "search": {},
        "diagnostic": {},
        "syntax": {}
    }"#;

    let result = validate_theme_json(json_wrong_color_type);
    assert!(!result.is_valid, "Should reject wrong type for color");
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.kind == ValidationErrorKind::TypeMismatch),
        "Should identify as type mismatch error. Errors: {:#?}",
        result.errors
    );

    // Test wrong type for editor section (should be object)
    let json_wrong_section_type = r#"{
        "name": "test",
        "editor": "not an object",
        "ui": {},
        "search": {},
        "diagnostic": {},
        "syntax": {}
    }"#;

    let result = validate_theme_json(json_wrong_section_type);
    assert!(
        !result.is_valid,
        "Should reject wrong type for editor section"
    );
    assert!(
        result
            .errors
            .iter()
            .any(|e| e.kind == ValidationErrorKind::TypeMismatch),
        "Should identify as type mismatch error. Errors: {:#?}",
        result.errors
    );
}

#[test]
fn test_strict_validation_accepts_valid_color_formats() {
    // Test RGB array format
    let json_rgb = r#"{
        "name": "test",
        "editor": {
            "bg": [30, 30, 30],
            "fg": [255, 255, 255]
        },
        "ui": {},
        "search": {},
        "diagnostic": {},
        "syntax": {}
    }"#;

    let result = validate_theme_json(json_rgb);
    assert!(
        result.is_valid,
        "Should accept RGB array format. Errors: {:#?}",
        result.errors
    );

    // Test named color format
    let json_named = r#"{
        "name": "test",
        "editor": {
            "bg": "Black",
            "fg": "White"
        },
        "ui": {},
        "search": {},
        "diagnostic": {},
        "syntax": {}
    }"#;

    let result = validate_theme_json(json_named);
    assert!(
        result.is_valid,
        "Should accept named color format. Errors: {:#?}",
        result.errors
    );
}

#[test]
fn test_validation_reports_multiple_errors() {
    // Theme with multiple issues
    let json_multiple_errors = r#"{
        "name": 123,
        "editor": {
            "unknown_field": "bad"
        },
        "ui": {
            "another_unknown": true
        },
        "search": {},
        "diagnostic": {},
        "syntax": {}
    }"#;

    let result = validate_theme_json(json_multiple_errors);
    assert!(!result.is_valid, "Should reject theme with multiple errors");
    assert!(
        result.errors.len() >= 2,
        "Should report multiple errors. Found {} error(s): {:#?}",
        result.errors.len(),
        result.errors
    );
}
