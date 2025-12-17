//! Entry detail dialog for editing complex map entries
//!
//! Provides a modal dialog for editing complex map entries with proper controls.
//! Fields are built dynamically from the JSON Schema.

use crate::view::controls::FocusState;
use crate::view::settings::schema::{SettingSchema, SettingType};
use serde_json::Value;

/// A field in the entry dialog
#[derive(Debug, Clone)]
pub struct DialogField {
    /// Field name/key
    pub name: String,
    /// Display label
    pub label: String,
    /// Current value
    pub value: FieldValue,
    /// Whether this field is required
    pub required: bool,
    /// Description/help text
    pub description: Option<String>,
}

/// Possible values for dialog fields
#[derive(Debug, Clone)]
pub enum FieldValue {
    /// Boolean toggle
    Bool(bool),
    /// Single-line text
    Text {
        value: String,
        cursor: usize,
        editing: bool,
    },
    /// Optional text (can be null)
    OptionalText {
        value: Option<String>,
        cursor: usize,
        editing: bool,
    },
    /// String array
    StringList {
        items: Vec<String>,
        focused_index: Option<usize>,
        new_text: String,
        cursor: usize,
        editing: bool,
    },
    /// Integer number
    Integer {
        value: i64,
        min: Option<i64>,
        max: Option<i64>,
        editing: bool,
        text: String,
    },
    /// Dropdown selection
    Dropdown {
        options: Vec<String>,
        selected: usize,
        open: bool,
    },
    /// Nested object (show field count, click to expand)
    Object {
        /// JSON representation
        json: Value,
        /// Expanded state
        expanded: bool,
    },
}

impl FieldValue {
    /// Check if the field is currently in edit mode
    pub fn is_editing(&self) -> bool {
        match self {
            FieldValue::Bool(_) => false,
            FieldValue::Text { editing, .. } => *editing,
            FieldValue::OptionalText { editing, .. } => *editing,
            FieldValue::StringList { editing, .. } => *editing,
            FieldValue::Integer { editing, .. } => *editing,
            FieldValue::Dropdown { open, .. } => *open,
            FieldValue::Object { .. } => false,
        }
    }
}

/// State for the entry detail dialog
#[derive(Debug, Clone)]
pub struct EntryDialogState {
    /// The entry key (e.g., "rust" for language)
    pub entry_key: String,
    /// The map path this entry belongs to (e.g., "/languages", "/lsp")
    pub map_path: String,
    /// Human-readable title for the dialog
    pub title: String,
    /// Whether this is a new entry (vs editing existing)
    pub is_new: bool,
    /// Fields in the dialog
    pub fields: Vec<DialogField>,
    /// Currently focused field index
    pub focused_field: usize,
    /// Currently focused button (0=Save, 1=Cancel)
    pub focused_button: usize,
    /// Whether focus is on buttons (true) or fields (false)
    pub focus_on_buttons: bool,
}

impl EntryDialogState {
    /// Create a dialog from a schema definition
    ///
    /// This is the primary, schema-driven constructor. It builds fields
    /// dynamically from the SettingSchema's properties.
    pub fn from_schema(
        key: String,
        value: &Value,
        schema: &SettingSchema,
        map_path: &str,
        is_new: bool,
    ) -> Self {
        let fields = build_fields_from_schema(schema, value);
        let title = if is_new {
            format!("New {}", schema.name)
        } else {
            format!("Edit {}: {}", schema.name, key)
        };

        Self {
            entry_key: key,
            map_path: map_path.to_string(),
            title,
            is_new,
            fields,
            focused_field: 0,
            focused_button: 0,
            focus_on_buttons: false,
        }
    }

    /// Convert dialog state back to JSON value
    pub fn to_value(&self) -> Value {
        let mut obj = serde_json::Map::new();

        for field in &self.fields {
            // Handle nested paths like "process_limits.enabled"
            let parts: Vec<&str> = field.name.split('.').collect();
            let value = field_to_value(&field.value);

            if parts.len() == 1 {
                obj.insert(parts[0].to_string(), value);
            } else if parts.len() == 2 {
                // Nested field
                let parent = obj
                    .entry(parts[0].to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Value::Object(ref mut parent_obj) = parent {
                    parent_obj.insert(parts[1].to_string(), value);
                }
            }
        }

        Value::Object(obj)
    }

    /// Move focus to previous field
    pub fn focus_prev(&mut self) {
        if self.focus_on_buttons {
            if self.focused_button > 0 {
                self.focused_button -= 1;
            } else {
                self.focus_on_buttons = false;
                self.focused_field = self.fields.len().saturating_sub(1);
            }
        } else if self.focused_field > 0 {
            self.focused_field -= 1;
        }
    }

    /// Move focus to next field
    pub fn focus_next(&mut self) {
        if self.focus_on_buttons {
            if self.focused_button < 1 {
                self.focused_button += 1;
            }
        } else if self.focused_field + 1 < self.fields.len() {
            self.focused_field += 1;
        } else {
            self.focus_on_buttons = true;
            self.focused_button = 0;
        }
    }

    /// Get the currently focused field
    pub fn current_field(&self) -> Option<&DialogField> {
        self.fields.get(self.focused_field)
    }

    /// Get the currently focused field mutably
    pub fn current_field_mut(&mut self) -> Option<&mut DialogField> {
        self.fields.get_mut(self.focused_field)
    }

    /// Toggle a boolean field or dropdown
    pub fn toggle_current(&mut self) {
        if let Some(field) = self.current_field_mut() {
            match &mut field.value {
                FieldValue::Bool(b) => *b = !*b,
                FieldValue::Dropdown { open, .. } => *open = !*open,
                _ => {}
            }
        }
    }

    /// Start editing the current text field
    pub fn start_editing(&mut self) {
        if let Some(field) = self.current_field_mut() {
            match &mut field.value {
                FieldValue::Text {
                    editing,
                    cursor,
                    value,
                } => {
                    *editing = true;
                    *cursor = value.len();
                }
                FieldValue::OptionalText {
                    editing,
                    cursor,
                    value,
                } => {
                    *editing = true;
                    *cursor = value.as_ref().map_or(0, |s| s.len());
                }
                FieldValue::StringList { editing, cursor, .. } => {
                    *editing = true;
                    *cursor = 0;
                }
                FieldValue::Integer { editing, text, value, .. } => {
                    *editing = true;
                    *text = value.to_string();
                }
                _ => {}
            }
        }
    }

    /// Stop editing and confirm changes
    pub fn stop_editing(&mut self) {
        if let Some(field) = self.current_field_mut() {
            match &mut field.value {
                FieldValue::Text { editing, .. } => *editing = false,
                FieldValue::OptionalText { editing, .. } => *editing = false,
                FieldValue::StringList { editing, .. } => *editing = false,
                FieldValue::Integer { editing, text, value, .. } => {
                    *editing = false;
                    if let Ok(n) = text.parse::<i64>() {
                        *value = n;
                    }
                }
                FieldValue::Dropdown { open, .. } => *open = false,
                _ => {}
            }
        }
    }

    /// Check if any field is being edited
    pub fn is_editing(&self) -> bool {
        self.fields.iter().any(|f| f.value.is_editing())
    }

    /// Get the focus state for a field
    pub fn field_focus_state(&self, index: usize) -> FocusState {
        if self.focus_on_buttons {
            FocusState::Normal
        } else if index == self.focused_field {
            FocusState::Focused
        } else {
            FocusState::Normal
        }
    }

    /// Insert a character into the current editable field
    pub fn insert_char(&mut self, c: char) {
        if let Some(field) = self.current_field_mut() {
            match &mut field.value {
                FieldValue::Text {
                    value,
                    cursor,
                    editing,
                } if *editing => {
                    value.insert(*cursor, c);
                    *cursor += c.len_utf8();
                }
                FieldValue::OptionalText {
                    value,
                    cursor,
                    editing,
                } if *editing => {
                    if value.is_none() {
                        *value = Some(String::new());
                    }
                    if let Some(ref mut s) = value {
                        s.insert(*cursor, c);
                        *cursor += c.len_utf8();
                    }
                }
                FieldValue::StringList {
                    new_text,
                    cursor,
                    editing,
                    ..
                } if *editing => {
                    new_text.insert(*cursor, c);
                    *cursor += c.len_utf8();
                }
                FieldValue::Integer { text, editing, .. } if *editing => {
                    if c.is_ascii_digit() || (c == '-' && text.is_empty()) {
                        text.push(c);
                    }
                }
                _ => {}
            }
        }
    }

    /// Handle backspace
    pub fn backspace(&mut self) {
        if let Some(field) = self.current_field_mut() {
            match &mut field.value {
                FieldValue::Text {
                    value,
                    cursor,
                    editing,
                } if *editing && *cursor > 0 => {
                    *cursor -= 1;
                    value.remove(*cursor);
                }
                FieldValue::OptionalText {
                    value,
                    cursor,
                    editing,
                } if *editing && *cursor > 0 => {
                    if let Some(ref mut s) = value {
                        *cursor -= 1;
                        s.remove(*cursor);
                        if s.is_empty() {
                            *value = None;
                        }
                    }
                }
                FieldValue::StringList {
                    new_text,
                    cursor,
                    editing,
                    ..
                } if *editing && *cursor > 0 => {
                    *cursor -= 1;
                    new_text.remove(*cursor);
                }
                FieldValue::Integer { text, editing, .. } if *editing => {
                    text.pop();
                }
                _ => {}
            }
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if let Some(field) = self.current_field_mut() {
            match &mut field.value {
                FieldValue::Text { cursor, editing, .. }
                | FieldValue::OptionalText { cursor, editing, .. }
                | FieldValue::StringList { cursor, editing, .. }
                    if *editing && *cursor > 0 =>
                {
                    *cursor -= 1;
                }
                _ => {}
            }
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        if let Some(field) = self.current_field_mut() {
            match &mut field.value {
                FieldValue::Text {
                    value,
                    cursor,
                    editing,
                } if *editing && *cursor < value.len() => {
                    *cursor += 1;
                }
                FieldValue::OptionalText {
                    value,
                    cursor,
                    editing,
                } if *editing => {
                    let max = value.as_ref().map_or(0, |s| s.len());
                    if *cursor < max {
                        *cursor += 1;
                    }
                }
                FieldValue::StringList {
                    new_text,
                    cursor,
                    editing,
                    ..
                } if *editing && *cursor < new_text.len() => {
                    *cursor += 1;
                }
                _ => {}
            }
        }
    }

    /// Navigate within dropdown
    pub fn dropdown_prev(&mut self) {
        if let Some(field) = self.current_field_mut() {
            if let FieldValue::Dropdown {
                options, selected, ..
            } = &mut field.value
            {
                if *selected > 0 {
                    *selected -= 1;
                } else {
                    *selected = options.len().saturating_sub(1);
                }
            }
        }
    }

    /// Navigate within dropdown
    pub fn dropdown_next(&mut self) {
        if let Some(field) = self.current_field_mut() {
            if let FieldValue::Dropdown {
                options, selected, ..
            } = &mut field.value
            {
                if *selected + 1 < options.len() {
                    *selected += 1;
                } else {
                    *selected = 0;
                }
            }
        }
    }

    /// Add item to string list and clear input
    pub fn add_list_item(&mut self) {
        if let Some(field) = self.current_field_mut() {
            if let FieldValue::StringList {
                items,
                new_text,
                cursor,
                ..
            } = &mut field.value
            {
                if !new_text.is_empty() {
                    items.push(std::mem::take(new_text));
                    *cursor = 0;
                }
            }
        }
    }

    /// Delete focused item from string list
    pub fn delete_list_item(&mut self) {
        if let Some(field) = self.current_field_mut() {
            if let FieldValue::StringList {
                items,
                focused_index,
                ..
            } = &mut field.value
            {
                if let Some(idx) = *focused_index {
                    if idx < items.len() {
                        items.remove(idx);
                        if items.is_empty() {
                            *focused_index = None;
                        } else if idx >= items.len() {
                            *focused_index = Some(items.len() - 1);
                        }
                    }
                }
            }
        }
    }

    /// Navigate within string list
    pub fn list_prev(&mut self) {
        if let Some(field) = self.current_field_mut() {
            if let FieldValue::StringList {
                items,
                focused_index,
                editing,
                ..
            } = &mut field.value
            {
                if *editing {
                    return;
                }
                match *focused_index {
                    None if !items.is_empty() => *focused_index = Some(items.len() - 1),
                    Some(0) => *focused_index = None,
                    Some(idx) => *focused_index = Some(idx - 1),
                    _ => {}
                }
            }
        }
    }

    /// Navigate within string list
    pub fn list_next(&mut self) {
        if let Some(field) = self.current_field_mut() {
            if let FieldValue::StringList {
                items,
                focused_index,
                editing,
                ..
            } = &mut field.value
            {
                if *editing {
                    return;
                }
                match *focused_index {
                    Some(idx) if idx + 1 < items.len() => *focused_index = Some(idx + 1),
                    Some(_) => *focused_index = None,
                    None if !items.is_empty() => *focused_index = Some(0),
                    _ => {}
                }
            }
        }
    }
}

/// Convert field value to JSON
fn field_to_value(field: &FieldValue) -> Value {
    match field {
        FieldValue::Bool(b) => Value::Bool(*b),
        FieldValue::Text { value, .. } => Value::String(value.clone()),
        FieldValue::OptionalText { value, .. } => {
            value.clone().map_or(Value::Null, Value::String)
        }
        FieldValue::StringList { items, .. } => {
            Value::Array(items.iter().map(|s| Value::String(s.clone())).collect())
        }
        FieldValue::Integer { value, .. } => Value::Number((*value).into()),
        FieldValue::Dropdown {
            options, selected, ..
        } => options
            .get(*selected)
            .map(|s| Value::String(s.clone()))
            .unwrap_or(Value::Null),
        FieldValue::Object { json, .. } => json.clone(),
    }
}

/// Build dialog fields from a schema definition
fn build_fields_from_schema(schema: &SettingSchema, value: &Value) -> Vec<DialogField> {
    let mut fields = Vec::new();

    // Extract properties from schema if it's an Object type
    let properties = match &schema.setting_type {
        SettingType::Object { properties } => properties,
        _ => return fields, // Not an object schema, return empty
    };

    for prop in properties {
        let field_value = value.get(&prop.path.trim_start_matches('/'));
        let field = build_field_from_property(prop, field_value);
        fields.push(field);
    }

    fields
}

/// Build a single dialog field from a schema property
fn build_field_from_property(prop: &SettingSchema, value: Option<&Value>) -> DialogField {
    let field_value = match &prop.setting_type {
        SettingType::Boolean => {
            let checked = value
                .and_then(|v| v.as_bool())
                .or_else(|| prop.default.as_ref().and_then(|d| d.as_bool()))
                .unwrap_or(false);
            FieldValue::Bool(checked)
        }

        SettingType::Integer { minimum, maximum } => {
            let val = value
                .and_then(|v| v.as_i64())
                .or_else(|| prop.default.as_ref().and_then(|d| d.as_i64()))
                .unwrap_or(0);
            FieldValue::Integer {
                value: val,
                min: *minimum,
                max: *maximum,
                editing: false,
                text: String::new(),
            }
        }

        SettingType::Number { .. } => {
            // Treat as integer for simplicity (could be extended)
            let val = value
                .and_then(|v| v.as_f64())
                .or_else(|| prop.default.as_ref().and_then(|d| d.as_f64()))
                .map(|f| f as i64)
                .unwrap_or(0);
            FieldValue::Integer {
                value: val,
                min: None,
                max: None,
                editing: false,
                text: String::new(),
            }
        }

        SettingType::String => {
            // Check if the value can be null (nullable string)
            let is_nullable = value.map_or(false, |v| v.is_null())
                || prop.default.as_ref().map_or(false, |d| d.is_null());

            if is_nullable {
                FieldValue::OptionalText {
                    value: value.and_then(|v| v.as_str()).map(String::from),
                    cursor: 0,
                    editing: false,
                }
            } else {
                let text = value
                    .and_then(|v| v.as_str())
                    .or_else(|| prop.default.as_ref().and_then(|d| d.as_str()))
                    .unwrap_or("")
                    .to_string();
                FieldValue::Text {
                    value: text,
                    cursor: 0,
                    editing: false,
                }
            }
        }

        SettingType::Enum { options } => {
            let current = value
                .and_then(|v| v.as_str())
                .or_else(|| prop.default.as_ref().and_then(|d| d.as_str()))
                .unwrap_or("");
            let option_values: Vec<String> = options.iter().map(|o| o.value.clone()).collect();
            let selected = option_values
                .iter()
                .position(|v| v == current)
                .unwrap_or(0);
            FieldValue::Dropdown {
                options: options.iter().map(|o| o.name.clone()).collect(),
                selected,
                open: false,
            }
        }

        SettingType::StringArray => {
            let items: Vec<String> = value
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .or_else(|| {
                    prop.default.as_ref().and_then(|d| {
                        d.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                    })
                })
                .unwrap_or_default();
            FieldValue::StringList {
                items,
                focused_index: None,
                new_text: String::new(),
                cursor: 0,
                editing: false,
            }
        }

        SettingType::Object { .. } | SettingType::Map { .. } | SettingType::Complex => {
            // For complex nested objects, store as JSON for now
            let json = value.cloned().unwrap_or_else(|| {
                prop.default.clone().unwrap_or(serde_json::json!({}))
            });
            FieldValue::Object {
                json,
                expanded: false,
            }
        }

        SettingType::KeybindingArray => {
            // Treat as a complex object for now
            let json = value.cloned().unwrap_or_else(|| serde_json::json!([]));
            FieldValue::Object {
                json,
                expanded: false,
            }
        }
    };

    // Extract property name from path (e.g., "/extensions" -> "extensions")
    let name = prop.path.trim_start_matches('/').to_string();

    DialogField {
        name,
        label: prop.name.clone(),
        value: field_value,
        required: false, // Could be derived from schema if we had "required" info
        description: prop.description.clone(),
    }
}
