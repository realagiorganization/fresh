//! Filesystem abstraction for platform-independent file operations
//!
//! This module provides a trait for filesystem operations, allowing the buffer
//! layer to work with different backends (native filesystem, browser storage,
//! remote agents, etc.)
//!
//! # Relationship to `services::fs::FsBackend`
//!
//! This crate has two filesystem abstractions:
//!
//! - **`model::filesystem::FileSystem`** (this module): Sync trait for file content I/O.
//!   Used by `Buffer` for loading/saving file contents. Lives in `model` to avoid
//!   circular dependencies and to support WASM builds where `services` is unavailable.
//!
//! - **`services::fs::FsBackend`**: Async trait for directory traversal and metadata.
//!   Used by the file tree UI for listing directories. Lives in `services` (runtime-only).
//!
//! These are kept separate because:
//! 1. Different concerns: content I/O vs directory navigation
//! 2. Different styles: sync (buffer ops) vs async (UI with slow/network FS)
//! 3. Different availability: `model` works in WASM, `services` is runtime-only

use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

/// Metadata about a file
#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// File size in bytes
    pub size: u64,
    /// File permissions (opaque, platform-specific)
    pub permissions: Option<FilePermissions>,
    /// File owner UID (Unix only)
    #[cfg(unix)]
    pub uid: Option<u32>,
    /// File owner GID (Unix only)
    #[cfg(unix)]
    pub gid: Option<u32>,
}

/// Opaque file permissions wrapper
#[derive(Debug, Clone)]
pub struct FilePermissions {
    #[cfg(unix)]
    mode: u32,
    #[cfg(not(unix))]
    readonly: bool,
}

impl FilePermissions {
    /// Create from std::fs::Permissions
    #[cfg(unix)]
    pub fn from_std(perms: std::fs::Permissions) -> Self {
        use std::os::unix::fs::PermissionsExt;
        Self { mode: perms.mode() }
    }

    #[cfg(not(unix))]
    pub fn from_std(perms: std::fs::Permissions) -> Self {
        Self {
            readonly: perms.readonly(),
        }
    }

    /// Convert to std::fs::Permissions
    #[cfg(unix)]
    pub fn to_std(&self) -> std::fs::Permissions {
        use std::os::unix::fs::PermissionsExt;
        std::fs::Permissions::from_mode(self.mode)
    }

    #[cfg(not(unix))]
    pub fn to_std(&self) -> std::fs::Permissions {
        let mut perms = std::fs::Permissions::from(std::fs::metadata(".").unwrap().permissions());
        perms.set_readonly(self.readonly);
        perms
    }

    /// Get the Unix mode (if available)
    #[cfg(unix)]
    pub fn mode(&self) -> u32 {
        self.mode
    }
}

/// A writable file handle
pub trait FileWriter: Write + Send {
    /// Sync all data to disk
    fn sync_all(&self) -> io::Result<()>;
}

/// Wrapper around std::fs::File that implements FileWriter
struct StdFileWriter(std::fs::File);

impl Write for StdFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl FileWriter for StdFileWriter {
    fn sync_all(&self) -> io::Result<()> {
        self.0.sync_all()
    }
}

/// A readable and seekable file handle
pub trait FileReader: Read + Seek + Send {}

/// Wrapper around std::fs::File that implements FileReader
struct StdFileReader(std::fs::File);

impl Read for StdFileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for StdFileReader {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

impl FileReader for StdFileReader {}

/// Trait for filesystem operations
///
/// This abstraction allows the buffer layer to work with different filesystem
/// implementations:
/// - `StdFileSystem`: Uses `std::fs` for native builds
/// - `NoopFileSystem`: Returns errors, for WASM where files come from JavaScript
///
/// # Example
///
/// ```ignore
/// // Native build
/// let fs = StdFileSystem;
/// let buffer = TextBuffer::load_from_file(&fs, "file.txt", 0)?;
///
/// // WASM build - content comes from JavaScript, no filesystem needed
/// let buffer = TextBuffer::from_bytes(content_from_js);
/// ```
pub trait FileSystem: Send + Sync {
    /// Read entire file into memory
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Read a range of bytes from a file
    ///
    /// Used for lazy loading of large files. Reads `len` bytes starting at `offset`.
    fn read_range(&self, path: &Path, offset: u64, len: usize) -> io::Result<Vec<u8>>;

    /// Write data to file atomically
    ///
    /// Implementations should use a temp file + rename pattern to avoid
    /// corrupting the original file if something goes wrong.
    fn write_file(&self, path: &Path, data: &[u8]) -> io::Result<()>;

    /// Get file metadata (size and permissions)
    fn metadata(&self, path: &Path) -> io::Result<FileMetadata>;

    /// Check if metadata exists (file exists), returns None if not
    fn metadata_if_exists(&self, path: &Path) -> Option<FileMetadata> {
        self.metadata(path).ok()
    }

    /// Create a file for writing, returns a writer handle
    fn create_file(&self, path: &Path) -> io::Result<Box<dyn FileWriter>>;

    /// Open a file for reading, returns a reader handle
    fn open_file(&self, path: &Path) -> io::Result<Box<dyn FileReader>>;

    /// Open a file for writing in-place (truncating existing content)
    /// This is used when we need to preserve file ownership on Unix
    fn open_file_for_write(&self, path: &Path) -> io::Result<Box<dyn FileWriter>>;

    /// Rename/move a file atomically
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// Copy a file (fallback when rename fails across filesystems)
    fn copy(&self, from: &Path, to: &Path) -> io::Result<u64>;

    /// Remove a file
    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Set file permissions
    fn set_permissions(&self, path: &Path, permissions: &FilePermissions) -> io::Result<()>;

    /// Get the current user's UID (Unix only, returns 0 on other platforms)
    fn current_uid(&self) -> u32;

    /// Check if the current user is the owner of the file
    fn is_owner(&self, path: &Path) -> bool {
        #[cfg(unix)]
        {
            if let Ok(meta) = self.metadata(path) {
                if let Some(uid) = meta.uid {
                    return uid == self.current_uid();
                }
            }
            true // Default to true if we can't determine
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            true
        }
    }

    /// Get a temporary file path for atomic writes
    fn temp_path_for(&self, path: &Path) -> PathBuf {
        path.with_extension("tmp")
    }

    /// Get a unique temporary file path (using timestamp and PID)
    fn unique_temp_path(&self, dest_path: &Path) -> PathBuf {
        let temp_dir = std::env::temp_dir();
        let file_name = dest_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("fresh-save"));
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        temp_dir.join(format!(
            "{}-{}-{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            timestamp
        ))
    }
}

/// Standard filesystem implementation using `std::fs`
///
/// This is the default implementation for native builds.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdFileSystem;

impl FileSystem for StdFileSystem {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn read_range(&self, path: &Path, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(path)?;
        file.seek(io::SeekFrom::Start(offset))?;

        let mut buffer = vec![0u8; len];
        file.read_exact(&mut buffer)?;

        Ok(buffer)
    }

    fn write_file(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        // Get original metadata to preserve permissions
        let original_metadata = self.metadata_if_exists(path);

        // Use temp file for atomic write
        let temp_path = self.temp_path_for(path);
        {
            let mut file = self.create_file(&temp_path)?;
            file.write_all(data)?;
            file.sync_all()?;
        }

        // Restore permissions if original file existed
        if let Some(ref meta) = original_metadata {
            if let Some(ref perms) = meta.permissions {
                let _ = self.set_permissions(&temp_path, perms);
            }
        }

        // Atomic rename
        self.rename(&temp_path, path)?;

        Ok(())
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        let meta = std::fs::metadata(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(FileMetadata {
                size: meta.len(),
                permissions: Some(FilePermissions::from_std(meta.permissions())),
                uid: Some(meta.uid()),
                gid: Some(meta.gid()),
            })
        }
        #[cfg(not(unix))]
        {
            Ok(FileMetadata {
                size: meta.len(),
                permissions: Some(FilePermissions::from_std(meta.permissions())),
            })
        }
    }

    fn create_file(&self, path: &Path) -> io::Result<Box<dyn FileWriter>> {
        let file = std::fs::File::create(path)?;
        Ok(Box::new(StdFileWriter(file)))
    }

    fn open_file(&self, path: &Path) -> io::Result<Box<dyn FileReader>> {
        let file = std::fs::File::open(path)?;
        Ok(Box::new(StdFileReader(file)))
    }

    fn open_file_for_write(&self, path: &Path) -> io::Result<Box<dyn FileWriter>> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Box::new(StdFileWriter(file)))
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn copy(&self, from: &Path, to: &Path) -> io::Result<u64> {
        std::fs::copy(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn set_permissions(&self, path: &Path, permissions: &FilePermissions) -> io::Result<()> {
        std::fs::set_permissions(path, permissions.to_std())
    }

    fn current_uid(&self) -> u32 {
        #[cfg(unix)]
        {
            unsafe { libc::getuid() }
        }
        #[cfg(not(unix))]
        {
            0
        }
    }
}

/// No-op filesystem that returns errors for all operations
///
/// Used in WASM builds where there is no filesystem access.
/// Content should be loaded via `TextBuffer::from_bytes()` instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopFileSystem;

impl FileSystem for NoopFileSystem {
    fn read_file(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn read_range(&self, _path: &Path, _offset: u64, _len: usize) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn write_file(&self, _path: &Path, _data: &[u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn metadata(&self, _path: &Path) -> io::Result<FileMetadata> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn create_file(&self, _path: &Path) -> io::Result<Box<dyn FileWriter>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn open_file(&self, _path: &Path) -> io::Result<Box<dyn FileReader>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn open_file_for_write(&self, _path: &Path) -> io::Result<Box<dyn FileWriter>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn copy(&self, _from: &Path, _to: &Path) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn set_permissions(&self, _path: &Path, _permissions: &FilePermissions) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem not available (WASM build)",
        ))
    }

    fn current_uid(&self) -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_std_filesystem_read_write() {
        let fs = StdFileSystem;
        let mut temp = NamedTempFile::new().unwrap();
        let path = temp.path().to_path_buf();

        // Write some content
        std::io::Write::write_all(&mut temp, b"Hello, World!").unwrap();
        std::io::Write::flush(&mut temp).unwrap();

        // Read it back
        let content = fs.read_file(&path).unwrap();
        assert_eq!(content, b"Hello, World!");

        // Read a range
        let range = fs.read_range(&path, 7, 5).unwrap();
        assert_eq!(range, b"World");

        // Check metadata
        let meta = fs.metadata(&path).unwrap();
        assert_eq!(meta.size, 13);
    }

    #[test]
    fn test_noop_filesystem() {
        let fs = NoopFileSystem;
        let path = Path::new("/some/path");

        assert!(fs.read_file(path).is_err());
        assert!(fs.read_range(path, 0, 10).is_err());
        assert!(fs.write_file(path, b"data").is_err());
        assert!(fs.metadata(path).is_err());
        assert!(fs.create_file(path).is_err());
        assert!(fs.open_file(path).is_err());
        assert!(fs.rename(path, path).is_err());
    }

    #[test]
    fn test_create_and_write_file() {
        let fs = StdFileSystem;
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create and write
        {
            let mut writer = fs.create_file(&path).unwrap();
            writer.write_all(b"test content").unwrap();
            writer.sync_all().unwrap();
        }

        // Read back
        let content = fs.read_file(&path).unwrap();
        assert_eq!(content, b"test content");
    }

    #[test]
    fn test_open_and_read_file() {
        let fs = StdFileSystem;
        let mut temp = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut temp, b"seekable content").unwrap();
        std::io::Write::flush(&mut temp).unwrap();

        let mut reader = fs.open_file(temp.path()).unwrap();

        // Seek and read
        reader.seek(io::SeekFrom::Start(9)).unwrap();
        let mut buf = [0u8; 7];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"content");
    }

    #[test]
    fn test_atomic_write() {
        let fs = StdFileSystem;
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("atomic_test.txt");

        // Write initial content
        fs.write_file(&path, b"initial").unwrap();
        assert_eq!(fs.read_file(&path).unwrap(), b"initial");

        // Write new content atomically
        fs.write_file(&path, b"updated").unwrap();
        assert_eq!(fs.read_file(&path).unwrap(), b"updated");
    }
}
