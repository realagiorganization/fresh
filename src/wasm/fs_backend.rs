//! In-memory filesystem backend for WASM builds
//!
//! This module provides a virtual filesystem that runs entirely in memory,
//! suitable for browser environments where direct filesystem access is not available.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

/// Represents a file or directory entry in the virtual filesystem
#[derive(Debug, Clone)]
pub struct WasmFsEntry {
    pub path: PathBuf,
    pub name: String,
    pub entry_type: WasmFsEntryType,
    pub metadata: Option<WasmFsMetadata>,
}

/// Type of filesystem entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmFsEntryType {
    File,
    Directory,
}

/// Metadata about a filesystem entry
#[derive(Debug, Clone)]
pub struct WasmFsMetadata {
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

impl WasmFsEntry {
    pub fn new(path: PathBuf, name: String, entry_type: WasmFsEntryType) -> Self {
        Self {
            path,
            name,
            entry_type,
            metadata: None,
        }
    }

    pub fn with_metadata(mut self, metadata: WasmFsMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn is_dir(&self) -> bool {
        self.entry_type == WasmFsEntryType::Directory
    }

    pub fn is_file(&self) -> bool {
        self.entry_type == WasmFsEntryType::File
    }
}

impl WasmFsMetadata {
    pub fn new() -> Self {
        Self {
            size: None,
            modified: None,
        }
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }
}

impl Default for WasmFsMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// In-memory filesystem for WASM
///
/// Files are stored in memory. Can be extended to support:
/// - IndexedDB for persistence
/// - File System Access API for local files (with user permission)
/// - Server API for remote file access
pub struct WasmFsBackend {
    /// Virtual filesystem: path -> content
    files: RwLock<HashMap<PathBuf, Vec<u8>>>,
    /// Directory structure: path -> child names
    directories: RwLock<HashMap<PathBuf, Vec<String>>>,
}

impl WasmFsBackend {
    pub fn new() -> Self {
        let mut dirs = HashMap::new();
        dirs.insert(PathBuf::from("/"), Vec::new());

        Self {
            files: RwLock::new(HashMap::new()),
            directories: RwLock::new(dirs),
        }
    }

    /// Add a file to the virtual filesystem
    pub fn add_file(&self, path: &Path, content: Vec<u8>) {
        let mut files = self.files.write().unwrap();
        files.insert(path.to_path_buf(), content);

        // Update parent directory listing
        if let Some(parent) = path.parent() {
            let mut dirs = self.directories.write().unwrap();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            dirs.entry(parent.to_path_buf())
                .or_insert_with(Vec::new)
                .push(name);
        }
    }

    /// Read file content
    pub fn read_file(&self, path: &Path) -> Option<Vec<u8>> {
        self.files.read().unwrap().get(path).cloned()
    }

    /// Write file content
    pub fn write_file(&self, path: &Path, content: Vec<u8>) {
        self.add_file(path, content);
    }

    /// Create a directory
    pub fn create_dir(&self, path: &Path) {
        let mut dirs = self.directories.write().unwrap();
        dirs.entry(path.to_path_buf()).or_insert_with(Vec::new);

        // Update parent directory
        if let Some(parent) = path.parent() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            dirs.entry(parent.to_path_buf())
                .or_insert_with(Vec::new)
                .push(name);
        }
    }

    /// List entries in a directory
    pub fn read_dir(&self, path: &Path) -> io::Result<Vec<WasmFsEntry>> {
        let dirs = self.directories.read().unwrap();
        let files = self.files.read().unwrap();

        let entries = dirs.get(path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Directory not found")
        })?;

        Ok(entries
            .iter()
            .map(|name| {
                let full_path = path.join(name);
                let entry_type = if files.contains_key(&full_path) {
                    WasmFsEntryType::File
                } else {
                    WasmFsEntryType::Directory
                };
                WasmFsEntry::new(full_path, name.clone(), entry_type)
            })
            .collect())
    }

    /// Check if path exists
    pub fn exists(&self, path: &Path) -> bool {
        let files = self.files.read().unwrap();
        let dirs = self.directories.read().unwrap();
        files.contains_key(path) || dirs.contains_key(path)
    }

    /// Check if path is a directory
    pub fn is_dir(&self, path: &Path) -> bool {
        let dirs = self.directories.read().unwrap();
        dirs.contains_key(path)
    }

    /// Get file/directory entry
    pub fn get_entry(&self, path: &Path) -> io::Result<WasmFsEntry> {
        let files = self.files.read().unwrap();
        let dirs = self.directories.read().unwrap();

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        if files.contains_key(path) {
            let size = files.get(path).map(|c| c.len() as u64);
            let metadata = WasmFsMetadata::new().with_size(size.unwrap_or(0));
            Ok(WasmFsEntry::new(path.to_path_buf(), name, WasmFsEntryType::File)
                .with_metadata(metadata))
        } else if dirs.contains_key(path) {
            Ok(WasmFsEntry::new(path.to_path_buf(), name, WasmFsEntryType::Directory))
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "Not found"))
        }
    }

    /// Remove a file
    pub fn remove_file(&self, path: &Path) -> io::Result<()> {
        let mut files = self.files.write().unwrap();
        if files.remove(path).is_some() {
            // Update parent directory
            if let Some(parent) = path.parent() {
                let mut dirs = self.directories.write().unwrap();
                if let Some(entries) = dirs.get_mut(parent) {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        entries.retain(|n| n != name);
                    }
                }
            }
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "File not found"))
        }
    }

    /// Get all file paths in the filesystem
    pub fn list_all_files(&self) -> Vec<PathBuf> {
        self.files.read().unwrap().keys().cloned().collect()
    }
}

impl Default for WasmFsBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_read_file() {
        let fs = WasmFsBackend::new();
        let path = Path::new("/test.txt");
        let content = b"Hello, world!".to_vec();

        fs.add_file(path, content.clone());

        let read = fs.read_file(path).unwrap();
        assert_eq!(read, content);
    }

    #[test]
    fn test_directory_listing() {
        let fs = WasmFsBackend::new();
        fs.add_file(Path::new("/file1.txt"), vec![]);
        fs.add_file(Path::new("/file2.txt"), vec![]);

        let entries = fs.read_dir(Path::new("/")).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_exists() {
        let fs = WasmFsBackend::new();
        let path = Path::new("/test.txt");

        assert!(!fs.exists(path));
        fs.add_file(path, vec![]);
        assert!(fs.exists(path));
    }

    #[test]
    fn test_create_dir() {
        let fs = WasmFsBackend::new();
        let path = Path::new("/subdir");

        fs.create_dir(path);
        assert!(fs.is_dir(path));
    }
}
