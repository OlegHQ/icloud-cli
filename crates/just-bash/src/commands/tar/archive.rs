// src/commands/tar/archive.rs

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Cursor, Read, Write};
use tar::{Builder, EntryType, Header};

#[derive(Debug, Clone)]
pub struct TarEntry {
    pub path: String,
    pub content: Vec<u8>,
    pub mode: u32,
    pub size: u64,
    pub mtime: u64,
    pub is_directory: bool,
    pub is_symlink: bool,
    pub link_target: String,
}

impl Default for TarEntry {
    fn default() -> Self {
        Self {
            path: String::new(),
            content: Vec::new(),
            mode: 0o644,
            size: 0,
            mtime: 0,
            is_directory: false,
            is_symlink: false,
            link_target: String::new(),
        }
    }
}

/// Create a tar archive from entries.
pub fn create_archive(entries: &[TarEntry]) -> Vec<u8> {
    let buf = Vec::new();
    let mut builder = Builder::new(buf);

    for entry in entries {
        let mut header = Header::new_ustar();

        if entry.is_directory {
            header.set_entry_type(EntryType::Directory);
            header.set_size(0);
        } else if entry.is_symlink {
            header.set_entry_type(EntryType::Symlink);
            header.set_size(0);
            header
                .set_link_name(&entry.link_target)
                .expect("failed to set link target");
        } else {
            header.set_entry_type(EntryType::Regular);
            header.set_size(entry.content.len() as u64);
        }

        header.set_mode(entry.mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(entry.mtime);
        header.set_username("root").expect("failed to set username");
        header
            .set_groupname("root")
            .expect("failed to set groupname");
        header.set_device_major(0).expect("failed to set devmajor");
        header.set_device_minor(0).expect("failed to set devminor");
        header.set_cksum();

        let mut path = entry.path.clone();
        if entry.is_directory && !path.ends_with('/') {
            path.push('/');
        }

        if entry.is_directory || entry.is_symlink {
            builder
                .append_data(&mut header, &path, &[] as &[u8])
                .expect("failed to append entry");
        } else {
            builder
                .append_data(&mut header, &path, entry.content.as_slice())
                .expect("failed to append entry");
        }
    }

    builder.into_inner().expect("failed to finish archive")
}

/// Parse a tar archive into entries.
pub fn parse_archive(data: &[u8]) -> Result<Vec<TarEntry>, String> {
    let cursor = Cursor::new(data);
    let mut archive = tar::Archive::new(cursor);
    let mut entries = Vec::new();

    for raw_entry in archive.entries().map_err(|e| format!("tar: {}", e))? {
        let mut raw_entry = raw_entry.map_err(|e| format!("tar: {}", e))?;
        let header = raw_entry.header().clone();

        let path = raw_entry
            .path()
            .map_err(|e| format!("tar: {}", e))?
            .to_string_lossy()
            .to_string();

        let mode = header.mode().unwrap_or(0o644);
        let size = header.size().unwrap_or(0);
        let mtime = header.mtime().unwrap_or(0);

        let entry_type = header.entry_type();
        let is_directory = entry_type == EntryType::Directory;
        let is_symlink = entry_type == EntryType::Symlink;

        let link_target = if is_symlink {
            raw_entry
                .link_name()
                .map_err(|e| format!("tar: {}", e))?
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };

        let content = if !is_directory && !is_symlink && size > 0 {
            let mut buf = Vec::with_capacity(size as usize);
            raw_entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("tar: {}", e))?;
            buf
        } else {
            Vec::new()
        };

        entries.push(TarEntry {
            path,
            content,
            mode,
            size,
            mtime,
            is_directory,
            is_symlink,
            link_target,
        });
    }

    Ok(entries)
}

/// Compress data with gzip.
pub fn compress_gzip(data: &[u8], level: u32) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(level));
    encoder.write_all(data).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

/// Decompress gzip data.
pub fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| e.to_string())?;
    Ok(decompressed)
}

/// Check if data is gzip compressed (magic bytes 0x1f 0x8b).
pub fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file_entry(path: &str, content: &[u8]) -> TarEntry {
        TarEntry {
            path: path.to_string(),
            content: content.to_vec(),
            mode: 0o644,
            size: content.len() as u64,
            mtime: 1700000000,
            is_directory: false,
            is_symlink: false,
            link_target: String::new(),
        }
    }

    fn make_dir_entry(path: &str) -> TarEntry {
        TarEntry {
            path: path.to_string(),
            content: Vec::new(),
            mode: 0o755,
            size: 0,
            mtime: 1700000000,
            is_directory: true,
            is_symlink: false,
            link_target: String::new(),
        }
    }

    #[test]
    fn test_create_archive_single_file() {
        let entry = make_file_entry("hello.txt", b"Hello, World!");
        let archive = create_archive(&[entry]);
        // Header (512) + content padded to 512 + 2 end blocks
        assert!(archive.len() >= 512 * 4);
        // Check magic
        assert_eq!(&archive[257..263], b"ustar\0");
    }

    #[test]
    fn test_create_archive_directory() {
        let entry = make_dir_entry("mydir");
        let archive = create_archive(&[entry]);
        // Header (512) + 2 end blocks (no content for dir)
        assert_eq!(archive.len(), 512 * 3);
        // Type flag should be '5'
        assert_eq!(archive[156], b'5');
    }

    #[test]
    fn test_parse_archive_single_file() {
        let entry = make_file_entry("test.txt", b"test content");
        let archive = create_archive(&[entry]);
        let entries = parse_archive(&archive).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "test.txt");
        assert_eq!(entries[0].content, b"test content");
        assert_eq!(entries[0].mode, 0o644);
        assert!(!entries[0].is_directory);
    }

    #[test]
    fn test_round_trip() {
        let entries = vec![
            make_dir_entry("project"),
            make_file_entry("project/main.rs", b"fn main() {}"),
            make_file_entry("project/lib.rs", b"pub fn hello() { println!(\"hi\"); }"),
        ];
        let archive = create_archive(&entries);
        let parsed = parse_archive(&archive).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].path, "project/");
        assert!(parsed[0].is_directory);
        assert_eq!(parsed[1].path, "project/main.rs");
        assert_eq!(parsed[1].content, b"fn main() {}");
        assert_eq!(parsed[2].path, "project/lib.rs");
        assert_eq!(parsed[2].content, b"pub fn hello() { println!(\"hi\"); }");
    }

    #[test]
    fn test_gzip_round_trip() {
        let data = b"Hello, this is test data for gzip compression!";
        let compressed = compress_gzip(data, 6).unwrap();
        assert!(is_gzip(&compressed));
        let decompressed = decompress_gzip(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_create_gzipped_archive() {
        let entry = make_file_entry("file.txt", b"gzip test data");
        let archive = create_archive(&[entry]);
        let compressed = compress_gzip(&archive, 6).unwrap();
        assert!(is_gzip(&compressed));
        assert!(compressed.len() < archive.len());
    }

    #[test]
    fn test_parse_gzipped_archive() {
        let entry = make_file_entry("file.txt", b"gzip archive test");
        let archive = create_archive(&[entry]);
        let compressed = compress_gzip(&archive, 6).unwrap();
        let decompressed = decompress_gzip(&compressed).unwrap();
        let entries = parse_archive(&decompressed).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "file.txt");
        assert_eq!(entries[0].content, b"gzip archive test");
    }

    #[test]
    fn test_header_checksum_validation() {
        // Verify we produce valid archives by round-tripping
        let entry = make_file_entry("check.txt", b"checksum test");
        let archive = create_archive(&[entry]);
        // A valid archive should parse successfully
        let entries = parse_archive(&archive).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "check.txt");

        // Corrupt a byte and verify parsing fails or produces wrong data
        let mut corrupted = archive.clone();
        corrupted[0] ^= 0xFF;
        // The tar crate may or may not error on corrupted data,
        // but the round-trip should not produce the original filename
        if let Ok(entries) = parse_archive(&corrupted) {
            if !entries.is_empty() {
                assert_ne!(entries[0].path, "check.txt");
            }
        }
    }

    #[test]
    fn test_long_filename() {
        let long_prefix = "a/very/deeply/nested/directory/structure/that/goes/on/and/on";
        let long_name = format!("{}/file.txt", long_prefix);
        let entry = make_file_entry(&long_name, b"long path");
        let archive = create_archive(&[entry]);
        let entries = parse_archive(&archive).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, long_name);
        assert_eq!(entries[0].content, b"long path");
    }

    #[test]
    fn test_empty_archive() {
        let archive = create_archive(&[]);
        // The tar crate writes two 512-byte zero blocks as end-of-archive
        assert_eq!(archive.len(), 512 * 2);
        assert!(archive.iter().all(|&b| b == 0));
        let entries = parse_archive(&archive).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_symlink_entry() {
        let entry = TarEntry {
            path: "link.txt".to_string(),
            content: Vec::new(),
            mode: 0o777,
            size: 0,
            mtime: 1700000000,
            is_directory: false,
            is_symlink: true,
            link_target: "target.txt".to_string(),
        };
        let archive = create_archive(&[entry]);
        let entries = parse_archive(&archive).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_symlink);
        assert_eq!(entries[0].link_target, "target.txt");
        assert_eq!(entries[0].path, "link.txt");
    }

    #[test]
    fn test_file_permissions_preserved() {
        let entry = TarEntry {
            path: "script.sh".to_string(),
            content: b"#!/bin/bash\necho hi".to_vec(),
            mode: 0o755,
            size: 19,
            mtime: 1700000000,
            is_directory: false,
            is_symlink: false,
            link_target: String::new(),
        };
        let archive = create_archive(&[entry]);
        let entries = parse_archive(&archive).unwrap();
        assert_eq!(entries[0].mode, 0o755);
    }

    #[test]
    fn test_is_gzip_detection() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzip(&[0x00, 0x00]));
        assert!(!is_gzip(&[0x1f]));
        assert!(!is_gzip(&[]));
    }

    #[test]
    fn test_large_file_content() {
        let content: Vec<u8> = (0..2048).map(|i| (i % 256) as u8).collect();
        let entry = TarEntry {
            path: "large.bin".to_string(),
            content: content.clone(),
            mode: 0o644,
            size: content.len() as u64,
            mtime: 0,
            is_directory: false,
            is_symlink: false,
            link_target: String::new(),
        };
        let archive = create_archive(&[entry]);
        let entries = parse_archive(&archive).unwrap();
        assert_eq!(entries[0].content, content);
    }

    #[test]
    fn test_multiple_files_round_trip() {
        let entries = vec![
            make_file_entry("a.txt", b"aaa"),
            make_file_entry("b.txt", b"bbb"),
            make_file_entry("c.txt", b"ccc"),
        ];
        let archive = create_archive(&entries);
        let parsed = parse_archive(&archive).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].content, b"aaa");
        assert_eq!(parsed[1].content, b"bbb");
        assert_eq!(parsed[2].content, b"ccc");
    }

    #[test]
    fn test_gzip_empty_data() {
        let compressed = compress_gzip(b"", 6).unwrap();
        assert!(is_gzip(&compressed));
        let decompressed = decompress_gzip(&compressed).unwrap();
        assert!(decompressed.is_empty());
    }

    #[test]
    fn test_decompress_invalid_data() {
        let result = decompress_gzip(&[0x1f, 0x8b, 0xFF, 0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn test_directory_path_gets_trailing_slash() {
        let entry = make_dir_entry("mydir");
        let archive = create_archive(&[entry]);
        let entries = parse_archive(&archive).unwrap();
        assert_eq!(entries[0].path, "mydir/");
    }
}
