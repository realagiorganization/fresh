//! Integration tests for RemoteFileSystem
//!
//! These tests spawn the Python agent locally and use the RemoteFileSystem
//! through AgentChannel to test the full integration stack.
//!
//! These tests use the production code paths:
//! - spawn_local_agent() for agent creation
//! - AgentChannel for communication
//! - RemoteFileSystem for file operations

use fresh::model::filesystem::{FileSystem, WriteOp};
use fresh::services::remote::{spawn_local_agent, RemoteFileSystem};

/// Creates a RemoteFileSystem using production code
fn create_test_filesystem() -> Option<(RemoteFileSystem, tempfile::TempDir, tokio::runtime::Runtime)>
{
    let temp_dir = tempfile::tempdir().ok()?;
    let rt = tokio::runtime::Runtime::new().ok()?;

    let channel = rt.block_on(spawn_local_agent()).ok()?;
    let fs = RemoteFileSystem::new(channel, "test@localhost".to_string());

    Some((fs, temp_dir, rt))
}

#[test]
fn test_read_file_content() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("test.txt");
    let test_content = b"Hello, this is test content!\nLine 2\nLine 3";

    // Write file using std::fs (directly to the temp dir)
    std::fs::write(&test_path, test_content).unwrap();

    // Read via RemoteFileSystem
    let read_content = fs.read_file(&test_path).unwrap();

    assert_eq!(
        read_content, test_content,
        "File content should match what was written"
    );
}

#[test]
fn test_write_and_read_roundtrip() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("write_test.txt");
    let test_content = b"Content written via RemoteFileSystem";

    // Write via RemoteFileSystem
    fs.write_file(&test_path, test_content).unwrap();

    // Read back via RemoteFileSystem
    let read_content = fs.read_file(&test_path).unwrap();

    assert_eq!(
        read_content, test_content,
        "Read content should match written content"
    );

    // Also verify via std::fs
    let direct_read = std::fs::read(&test_path).unwrap();
    assert_eq!(
        direct_read, test_content,
        "Direct file read should match written content"
    );
}

#[test]
fn test_read_large_file() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("large.bin");

    // Create a file larger than the chunk size (65536 bytes)
    let test_content: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();

    std::fs::write(&test_path, &test_content).unwrap();

    // Read via RemoteFileSystem (should handle multiple streaming chunks)
    let read_content = fs.read_file(&test_path).unwrap();

    assert_eq!(
        read_content.len(),
        test_content.len(),
        "File sizes should match"
    );
    assert_eq!(
        read_content, test_content,
        "Large file content should match"
    );
}

#[test]
fn test_is_dir() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let dir_path = temp_dir.path().join("subdir");
    let file_path = temp_dir.path().join("file.txt");

    std::fs::create_dir(&dir_path).unwrap();
    std::fs::write(&file_path, b"content").unwrap();

    assert!(fs.is_dir(&dir_path).unwrap(), "Should detect directory");
    assert!(!fs.is_dir(&file_path).unwrap(), "File should not be a dir");
}

#[test]
fn test_read_dir() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    std::fs::write(temp_dir.path().join("file1.txt"), b"1").unwrap();
    std::fs::write(temp_dir.path().join("file2.txt"), b"2").unwrap();
    std::fs::create_dir(temp_dir.path().join("subdir")).unwrap();

    let entries = fs.read_dir(temp_dir.path()).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();

    assert!(names.contains(&"file1.txt"), "Should contain file1.txt");
    assert!(names.contains(&"file2.txt"), "Should contain file2.txt");
    assert!(names.contains(&"subdir"), "Should contain subdir");
}

#[test]
fn test_remote_connection_info() {
    let Some((fs, _temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    assert_eq!(
        fs.remote_connection_info(),
        Some("test@localhost"),
        "Should return connection string"
    );
}

#[test]
fn test_metadata() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("meta_test.txt");
    let content = b"test content for metadata";
    std::fs::write(&test_path, content).unwrap();

    let meta = fs.metadata(&test_path).unwrap();
    assert_eq!(
        meta.size,
        content.len() as u64,
        "Size should match content length"
    );
}

#[test]
fn test_read_file_larger_than_threshold() {
    // Test reading a file larger than LARGE_FILE_THRESHOLD_BYTES (1MB)
    // This tests that streaming works correctly for very large files
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("very_large.bin");

    // Create a 1.5MB file (larger than the 1MB threshold)
    let size = 1_500_000;
    let test_content: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();

    std::fs::write(&test_path, &test_content).unwrap();

    // Read via RemoteFileSystem
    let read_content = fs.read_file(&test_path).unwrap();

    assert_eq!(
        read_content.len(),
        test_content.len(),
        "File sizes should match for 1.5MB file"
    );
    assert_eq!(
        read_content, test_content,
        "Very large file content should match"
    );
}

#[test]
fn test_write_and_read_file_larger_than_threshold() {
    // Test write+read roundtrip for a file larger than the threshold
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("write_large.bin");

    // Create 2MB of content
    let size = 2_000_000;
    let test_content: Vec<u8> = (0..size).map(|i| ((i * 7) % 256) as u8).collect();

    // Write via RemoteFileSystem
    fs.write_file(&test_path, &test_content).unwrap();

    // Read back via RemoteFileSystem
    let read_content = fs.read_file(&test_path).unwrap();

    assert_eq!(
        read_content.len(),
        test_content.len(),
        "2MB file sizes should match after roundtrip"
    );
    assert_eq!(
        read_content, test_content,
        "2MB file content should match after roundtrip"
    );
}

#[test]
fn test_read_range_on_large_file() {
    // Test read_range on a large file
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("range_large.bin");

    // Create 1.5MB file
    let size = 1_500_000;
    let test_content: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
    std::fs::write(&test_path, &test_content).unwrap();

    // Read a range from the middle of the file
    let offset = 1_000_000; // 1MB into the file
    let len = 100_000; // Read 100KB
    let read_content = fs.read_range(&test_path, offset, len).unwrap();

    assert_eq!(read_content.len(), len, "Read range length should match");
    assert_eq!(
        read_content,
        &test_content[offset as usize..(offset as usize + len)],
        "Read range content should match"
    );
}

// =============================================================================
// Tests for optimized remote operations (Phase 1 & 2 optimizations)
// =============================================================================

#[test]
fn test_append_to_file() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("append_test.txt");

    // Create initial file
    fs.write_file(&test_path, b"Hello").unwrap();

    // Append using open_file_for_append
    {
        use std::io::Write;
        let mut writer = fs.open_file_for_append(&test_path).unwrap();
        writer.write_all(b" World").unwrap();
        writer.sync_all().unwrap();
    }

    // Verify content
    let content = fs.read_file(&test_path).unwrap();
    assert_eq!(
        content, b"Hello World",
        "Append should add to existing content"
    );
}

#[test]
fn test_append_creates_file_if_missing() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("append_new.txt");

    // File doesn't exist yet
    assert!(!test_path.exists());

    // Append to non-existent file (should create it)
    {
        use std::io::Write;
        let mut writer = fs.open_file_for_append(&test_path).unwrap();
        writer.write_all(b"New content").unwrap();
        writer.sync_all().unwrap();
    }

    // Verify file was created with content
    let content = fs.read_file(&test_path).unwrap();
    assert_eq!(content, b"New content");
}

#[test]
fn test_truncate_file() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("truncate_test.txt");

    // Create file with content
    fs.write_file(&test_path, b"Hello World!").unwrap();

    // Truncate to 5 bytes
    fs.set_file_length(&test_path, 5).unwrap();

    // Verify content was truncated
    let content = fs.read_file(&test_path).unwrap();
    assert_eq!(content, b"Hello", "File should be truncated to 5 bytes");
}

#[test]
fn test_truncate_extend_file() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let test_path = temp_dir.path().join("extend_test.txt");

    // Create file with content
    fs.write_file(&test_path, b"Hi").unwrap();

    // Extend to 10 bytes (should pad with zeros)
    fs.set_file_length(&test_path, 10).unwrap();

    // Verify content was extended
    let content = fs.read_file(&test_path).unwrap();
    assert_eq!(content.len(), 10, "File should be extended to 10 bytes");
    assert_eq!(&content[0..2], b"Hi", "Original content preserved");
    assert!(
        content[2..].iter().all(|&b| b == 0),
        "Extended portion should be zeros"
    );
}

#[test]
fn test_write_patched_copy_and_insert() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let src_path = temp_dir.path().join("patch_src.txt");
    let dst_path = temp_dir.path().join("patch_dst.txt");

    // Create source file: "AAABBBCCC"
    fs.write_file(&src_path, b"AAABBBCCC").unwrap();

    // Apply patch: copy "AAA", insert "XXX", copy "CCC"
    let ops = vec![
        WriteOp::Copy { offset: 0, len: 3 }, // "AAA"
        WriteOp::Insert { data: b"XXX" },    // "XXX"
        WriteOp::Copy { offset: 6, len: 3 }, // "CCC"
    ];

    fs.write_patched(&src_path, &dst_path, &ops).unwrap();

    // Verify result
    let content = fs.read_file(&dst_path).unwrap();
    assert_eq!(
        content, b"AAAXXXCCC",
        "Patched content should match expected"
    );
}

#[test]
fn test_write_patched_in_place() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let path = temp_dir.path().join("patch_inplace.txt");

    // Create source file
    fs.write_file(&path, b"Hello World").unwrap();

    // Patch in-place: keep "Hello ", replace "World" with "Rust!"
    let ops = vec![
        WriteOp::Copy { offset: 0, len: 6 }, // "Hello "
        WriteOp::Insert { data: b"Rust!" },  // "Rust!"
    ];

    fs.write_patched(&path, &path, &ops).unwrap();

    // Verify result
    let content = fs.read_file(&path).unwrap();
    assert_eq!(content, b"Hello Rust!", "In-place patch should work");
}

#[test]
fn test_write_patched_large_file_small_edit() {
    // This test verifies the optimization benefit:
    // Edit a large file with a small change, only the change is transferred
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let path = temp_dir.path().join("large_patch.bin");

    // Create a 1MB file
    let size = 1_000_000;
    let original: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
    fs.write_file(&path, &original).unwrap();

    // Patch: keep first 500KB, insert 100 bytes, keep last 500KB
    let insert_data = b"THIS IS THE NEW CONTENT INSERTED IN THE MIDDLE OF A LARGE FILE!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!";
    let ops = vec![
        WriteOp::Copy {
            offset: 0,
            len: 500_000,
        },
        WriteOp::Insert { data: insert_data },
        WriteOp::Copy {
            offset: 500_000,
            len: 500_000,
        },
    ];

    fs.write_patched(&path, &path, &ops).unwrap();

    // Verify result
    let content = fs.read_file(&path).unwrap();
    assert_eq!(
        content.len(),
        size + insert_data.len(),
        "File size should be original + inserted"
    );
    assert_eq!(
        &content[0..500_000],
        &original[0..500_000],
        "First half should match"
    );
    assert_eq!(
        &content[500_000..500_000 + insert_data.len()],
        insert_data,
        "Inserted content should match"
    );
    assert_eq!(
        &content[500_000 + insert_data.len()..],
        &original[500_000..],
        "Second half should match"
    );
}

#[test]
fn test_write_patched_preserves_permissions() {
    let Some((fs, temp_dir, _rt)) = create_test_filesystem() else {
        eprintln!("Skipping test: could not create test filesystem");
        return;
    };

    let path = temp_dir.path().join("perms_test.txt");

    // Create file and set specific permissions
    fs.write_file(&path, b"original").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // Patch the file
    let ops = vec![WriteOp::Insert { data: b"patched" }];
    fs.write_patched(&path, &path, &ops).unwrap();

    // Verify permissions preserved
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o755,
            "Permissions should be preserved after patch"
        );
    }
}
