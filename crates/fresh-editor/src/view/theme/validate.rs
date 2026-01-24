//! Strict theme validation using JSON Schema.
//!
//! This module provides validation for theme JSON files that rejects unknown fields
//! and checks type compliance. It uses the `jsonschema` crate with a modified schema
//! that adds `additionalProperties: false` to all object types.

use std::path::Path;

use serde_json::Value;

/// A single validation error with path and message.
#[derive(Debug, Clone)]
pub struct ThemeValidationError {
    /// JSON path to the error (e.g., "ui.tab_active_fg")
    pub path: String,
    /// Human-readable error message
    pub message: String,
    /// Kind of error
    pub kind: ValidationErrorKind,
}

/// The kind of validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorKind {
    /// Unknown field that is not in the schema
    UnknownField,
    /// Type mismatch (e.g., expected string, got number)
    TypeMismatch,
    /// Missing required field
    MissingField,
    /// Other validation error
    Other,
}

/// Result of theme validation.
#[derive(Debug)]
pub struct ValidationResult {
    /// Whether the theme is valid
    pub is_valid: bool,
    /// List of validation errors (empty if valid)
    pub errors: Vec<ThemeValidationError>,
}

impl ValidationResult {
    /// Create a successful validation result.
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
        }
    }

    /// Create a failed validation result with errors.
    pub fn invalid(errors: Vec<ThemeValidationError>) -> Self {
        Self {
            is_valid: false,
            errors,
        }
    }
}

/// Transform a JSON Schema to add `additionalProperties: false` to all object types.
/// This makes the schema strict, rejecting any unknown fields.
fn make_schema_strict(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            // If this is an object type definition, add additionalProperties: false
            if let Some(Value::String(type_str)) = map.get("type") {
                if type_str == "object" {
                    // Only add if not already present
                    if !map.contains_key("additionalProperties") {
                        map.insert("additionalProperties".to_string(), Value::Bool(false));
                    }
                }
            }

            // Recursively process all nested schemas
            for (key, value) in map.iter_mut() {
                // Skip $ref as it shouldn't be modified
                if key != "$ref" {
                    make_schema_strict(value);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                make_schema_strict(item);
            }
        }
        _ => {}
    }
}

/// Get a strict version of the theme schema that rejects unknown fields.
pub fn get_strict_theme_schema() -> Value {
    let mut schema = super::get_theme_schema();
    make_schema_strict(&mut schema);
    schema
}

/// Determine the kind of validation error from the error message/type.
fn classify_error(error: &jsonschema::ValidationError) -> ValidationErrorKind {
    let error_str = error.to_string();

    if error_str.contains("Additional properties are not allowed")
        || error_str.contains("additionalProperties")
    {
        ValidationErrorKind::UnknownField
    } else if error_str.contains("is not of type")
        || error_str.contains("is not valid under any of the given schemas")
        || error_str.contains("'anyOf'")
        || error_str.contains("'oneOf'")
    {
        ValidationErrorKind::TypeMismatch
    } else if error_str.contains("is a required property") || error_str.contains("required") {
        ValidationErrorKind::MissingField
    } else {
        ValidationErrorKind::Other
    }
}

/// Format the JSON path from a validation error.
fn format_path(error: &jsonschema::ValidationError) -> String {
    let path = error.instance_path().to_string();
    if path.is_empty() || path == "/" {
        "(root)".to_string()
    } else {
        // Convert /foo/bar to foo.bar for readability
        path.trim_start_matches('/').replace('/', ".")
    }
}

/// Validate theme JSON string against the strict schema.
///
/// Returns a `ValidationResult` indicating whether the theme is valid
/// and any validation errors found.
pub fn validate_theme_json(json: &str) -> ValidationResult {
    // Parse the JSON
    let instance: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            return ValidationResult::invalid(vec![ThemeValidationError {
                path: "(root)".to_string(),
                message: format!("Invalid JSON: {}", e),
                kind: ValidationErrorKind::Other,
            }]);
        }
    };

    // Get the strict schema
    let schema = get_strict_theme_schema();

    // Compile the schema
    let compiled = match jsonschema::validator_for(&schema) {
        Ok(v) => v,
        Err(e) => {
            return ValidationResult::invalid(vec![ThemeValidationError {
                path: "(root)".to_string(),
                message: format!("Internal error: failed to compile schema: {}", e),
                kind: ValidationErrorKind::Other,
            }]);
        }
    };

    // Validate using iter_errors to get all validation errors
    let validation_errors: Vec<ThemeValidationError> = compiled
        .iter_errors(&instance)
        .map(|e| ThemeValidationError {
            path: format_path(&e),
            message: e.to_string(),
            kind: classify_error(&e),
        })
        .collect();

    if validation_errors.is_empty() {
        ValidationResult::valid()
    } else {
        ValidationResult::invalid(validation_errors)
    }
}

/// Validate a theme file at the given path.
///
/// Returns `Ok(ValidationResult)` with validation status,
/// or `Err` if the file could not be read.
pub fn validate_theme_file(path: &Path) -> Result<ValidationResult, std::io::Error> {
    let contents = std::fs::read_to_string(path)?;
    Ok(validate_theme_json(&contents))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_minimal_theme() {
        let json = r#"{
            "name": "test",
            "editor": {},
            "ui": {},
            "search": {},
            "diagnostic": {},
            "syntax": {}
        }"#;
        let result = validate_theme_json(json);
        assert!(
            result.is_valid,
            "Minimal theme should be valid: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_unknown_field_at_root() {
        let json = r#"{
            "name": "test",
            "editor": {},
            "ui": {},
            "search": {},
            "diagnostic": {},
            "syntax": {},
            "unknown_field": "value"
        }"#;
        let result = validate_theme_json(json);
        assert!(
            !result.is_valid,
            "Theme with unknown root field should be invalid"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::UnknownField),
            "Should have an unknown field error: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_unknown_field_nested() {
        let json = r#"{
            "name": "test",
            "editor": {
                "bg": [30, 30, 30],
                "unknown_editor_field": "value"
            },
            "ui": {},
            "search": {},
            "diagnostic": {},
            "syntax": {}
        }"#;
        let result = validate_theme_json(json);
        assert!(
            !result.is_valid,
            "Theme with unknown nested field should be invalid"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::UnknownField),
            "Should have an unknown field error: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_type_mismatch() {
        let json = r#"{
            "name": 123,
            "editor": {},
            "ui": {},
            "search": {},
            "diagnostic": {},
            "syntax": {}
        }"#;
        let result = validate_theme_json(json);
        assert!(
            !result.is_valid,
            "Theme with type mismatch should be invalid"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::TypeMismatch),
            "Should have a type mismatch error: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_missing_required_field() {
        let json = r#"{
            "editor": {},
            "ui": {},
            "search": {},
            "diagnostic": {},
            "syntax": {}
        }"#;
        let result = validate_theme_json(json);
        assert!(!result.is_valid, "Theme missing 'name' should be invalid");
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.kind == ValidationErrorKind::MissingField),
            "Should have a missing field error: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_invalid_json() {
        let json = "{ not valid json }";
        let result = validate_theme_json(json);
        assert!(!result.is_valid, "Invalid JSON should fail validation");
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("Invalid JSON")),
            "Should mention invalid JSON: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_color_rgb_format() {
        let json = r#"{
            "name": "test",
            "editor": {
                "bg": [30, 30, 30]
            },
            "ui": {},
            "search": {},
            "diagnostic": {},
            "syntax": {}
        }"#;
        let result = validate_theme_json(json);
        assert!(
            result.is_valid,
            "RGB color array should be valid: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_color_named_format() {
        let json = r#"{
            "name": "test",
            "editor": {
                "bg": "Black"
            },
            "ui": {},
            "search": {},
            "diagnostic": {},
            "syntax": {}
        }"#;
        let result = validate_theme_json(json);
        assert!(
            result.is_valid,
            "Named color should be valid: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_strict_schema_has_additional_properties_false() {
        let schema = get_strict_theme_schema();

        // Check that the root has additionalProperties: false
        if let Value::Object(map) = &schema {
            assert_eq!(
                map.get("additionalProperties"),
                Some(&Value::Bool(false)),
                "Root schema should have additionalProperties: false"
            );
        } else {
            panic!("Schema should be an object");
        }
    }

    #[test]
    fn test_builtin_themes_pass_strict_validation() {
        use super::super::BUILTIN_THEMES;

        for theme in BUILTIN_THEMES {
            let result = validate_theme_json(theme.json);
            assert!(
                result.is_valid,
                "Builtin theme '{}' should pass strict validation. Errors: {:?}",
                theme.name, result.errors
            );
        }
    }
}
