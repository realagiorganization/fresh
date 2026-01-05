//! Theme loading from filesystem (runtime-only)
//!
//! This module provides functions for loading themes from JSON files and
//! scanning theme directories. These functions require filesystem access
//! and are only available in runtime builds (not WASM).
//!
//! For pure theme types and built-in themes, see the `types` module.

use std::path::Path;

use super::types::{Theme, ThemeFile};

impl Theme {
    /// Load theme from a JSON file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read theme file: {}", e))?;
        let theme_file: ThemeFile = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse theme file: {}", e))?;
        Ok(theme_file.into())
    }

    /// Load builtin theme from the themes directory
    pub fn load_builtin_theme(name: &str) -> Option<Self> {
        // Build list of paths to search
        let mut theme_paths = vec![
            format!("themes/{}.json", name),
            format!("../themes/{}.json", name),
            format!("../../themes/{}.json", name),
        ];

        // Also check user config themes directory
        if let Some(config_dir) = dirs::config_dir() {
            let user_theme_path = config_dir
                .join("fresh")
                .join("themes")
                .join(format!("{}.json", name));
            theme_paths.insert(0, user_theme_path.to_string_lossy().to_string());
        }

        for path in &theme_paths {
            if let Ok(theme) = Self::from_file(path) {
                return Some(theme);
            }
        }

        None
    }

    /// Get a theme by name, defaults to dark if not found
    ///
    /// Tries to load from JSON file first, falls back to hardcoded themes.
    pub fn from_name(name: &str) -> Self {
        let normalized_name = name.to_lowercase().replace('_', "-");

        // Try to load from JSON file first
        if let Some(theme) = Self::load_builtin_theme(&normalized_name) {
            return theme;
        }

        // Fall back to hardcoded themes
        Self::from_name_embedded(&normalized_name)
    }

    /// Get all available theme names (builtin + user themes)
    ///
    /// Scans both the built-in themes directory and user config themes directory.
    pub fn available_themes() -> Vec<String> {
        let mut themes = Self::embedded_themes();

        // Scan built-in themes directory (themes/*.json in the project)
        if let Ok(entries) = std::fs::read_dir("themes") {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    if let Some(stem) = path.file_stem() {
                        let name = stem.to_string_lossy().to_string();
                        // Avoid duplicates
                        if !themes.iter().any(|t| t == &name) {
                            themes.push(name);
                        }
                    }
                }
            }
        }

        // Scan user themes directory (user themes can override built-ins)
        if let Some(config_dir) = dirs::config_dir() {
            let user_themes_dir = config_dir.join("fresh").join("themes");
            if let Ok(entries) = std::fs::read_dir(&user_themes_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "json") {
                        if let Some(stem) = path.file_stem() {
                            let name = stem.to_string_lossy().to_string();
                            // Avoid duplicates (user theme overriding builtin)
                            if !themes.iter().any(|t| t == &name) {
                                themes.push(name);
                            }
                        }
                    }
                }
            }
        }

        themes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_from_name() {
        let theme = Theme::from_name("light");
        assert_eq!(theme.name, "light");

        let theme = Theme::from_name("high-contrast");
        assert_eq!(theme.name, "high-contrast");

        let theme = Theme::from_name("unknown");
        assert_eq!(theme.name, "dark");
    }

    #[test]
    fn test_available_themes() {
        let themes = Theme::available_themes();
        // At minimum, should have the 4 builtin themes
        assert!(themes.len() >= 4);
        assert!(themes.contains(&"dark".to_string()));
        assert!(themes.contains(&"light".to_string()));
        assert!(themes.contains(&"high-contrast".to_string()));
        assert!(themes.contains(&"nostalgia".to_string()));
    }
}
