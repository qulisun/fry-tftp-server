use std::collections::HashMap;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FsError {
    #[error("access violation: {0}")]
    AccessViolation(String),
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

/// Virtual roots mapping: alias prefix → physical directory
#[derive(Debug, Clone, Default)]
pub struct VirtualRoots {
    roots: Vec<(String, PathBuf)>, // sorted by prefix length descending for longest-match
}

impl VirtualRoots {
    pub fn new(map: &HashMap<String, String>) -> Self {
        let mut roots: Vec<(String, PathBuf)> = map
            .iter()
            .map(|(alias, path)| {
                let alias = alias
                    .trim_start_matches('/')
                    .trim_start_matches('\\')
                    .to_string();
                (alias, PathBuf::from(path))
            })
            .collect();
        // Sort by length descending for longest prefix match
        roots.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Self { roots }
    }

    /// Try to resolve a path through virtual roots.
    /// Returns (physical_root, remaining_path) if matched.
    pub fn resolve(&self, requested: &str) -> Option<(PathBuf, String)> {
        let normalized = requested.trim_start_matches('/').trim_start_matches('\\');
        for (alias, physical) in &self.roots {
            if normalized.starts_with(alias.as_str()) {
                let rest = &normalized[alias.len()..];
                let rest = rest.trim_start_matches('/').trim_start_matches('\\');
                if rest.is_empty() {
                    return None; // can't request a directory
                }
                return Some((physical.clone(), rest.to_string()));
            }
        }
        None
    }
}

fn validate_requested_path(requested: &str) -> Result<(), FsError> {
    for b in requested.bytes() {
        if b < 0x20 {
            return Err(FsError::InvalidPath(format!(
                "control character 0x{:02x} in path",
                b
            )));
        }
    }
    if requested.contains("..") {
        return Err(FsError::AccessViolation(
            "path traversal attempt (..)".to_string(),
        ));
    }
    if requested.contains('~') {
        return Err(FsError::AccessViolation("tilde in path".to_string()));
    }
    Ok(())
}

/// Resolve a requested TFTP path securely.
/// Checks virtual roots first, then falls back to main root.
pub fn resolve_path_with_virtual(
    root: &Path,
    virtual_roots: &VirtualRoots,
    requested: &str,
    must_exist: bool,
    follow_symlinks: bool,
) -> Result<PathBuf, FsError> {
    validate_requested_path(requested)?;

    // Try virtual roots first
    if let Some((vroot, remaining)) = virtual_roots.resolve(requested) {
        return resolve_against_root(&vroot, &remaining, must_exist, follow_symlinks);
    }

    // Fall back to main root
    resolve_path(root, requested, must_exist, follow_symlinks)
}

/// Resolve a requested TFTP path securely against the root directory.
pub fn resolve_path(
    root: &Path,
    requested: &str,
    must_exist: bool,
    follow_symlinks: bool,
) -> Result<PathBuf, FsError> {
    validate_requested_path(requested)?;
    let stripped = requested.trim_start_matches('/').trim_start_matches('\\');
    if stripped.is_empty() {
        return Err(FsError::InvalidPath("empty path".to_string()));
    }
    resolve_against_root(root, stripped, must_exist, follow_symlinks)
}

fn resolve_against_root(
    root: &Path,
    stripped: &str,
    must_exist: bool,
    follow_symlinks: bool,
) -> Result<PathBuf, FsError> {
    #[cfg(target_os = "windows")]
    let stripped = stripped.replace('/', "\\");

    #[allow(clippy::needless_borrows_for_generic_args)]
    let joined = root.join(&stripped);

    if must_exist {
        // Check symlink before canonicalization (if follow_symlinks=false)
        if !follow_symlinks {
            check_no_symlinks(&joined)?;
        }

        let canonical_root = std::fs::canonicalize(root)
            .map_err(|_| FsError::InvalidPath("root dir not found".to_string()))?;
        let canonical = std::fs::canonicalize(&joined)
            .map_err(|_| FsError::FileNotFound(stripped.to_string()))?;

        if !canonical.starts_with(&canonical_root) {
            return Err(FsError::AccessViolation(format!(
                "resolved path escapes root: {}",
                stripped
            )));
        }

        let meta = std::fs::metadata(&canonical)?;
        if !meta.is_file() {
            return Err(FsError::InvalidPath(format!(
                "not a regular file: {}",
                stripped
            )));
        }

        Ok(canonical)
    } else {
        if let Some(parent) = joined.parent() {
            if parent.exists() {
                if !follow_symlinks {
                    check_no_symlinks(parent)?;
                }
                let canonical_root = std::fs::canonicalize(root)
                    .map_err(|_| FsError::InvalidPath("root dir not found".to_string()))?;
                let canonical_parent = std::fs::canonicalize(parent).map_err(FsError::Io)?;
                if !canonical_parent.starts_with(&canonical_root) {
                    return Err(FsError::AccessViolation(format!(
                        "resolved path escapes root: {}",
                        stripped
                    )));
                }
            }
        }
        Ok(joined)
    }
}

// ─── Memory-Mapped File Handle ──────────────────────────────────────────────

/// Threshold below which files are read into memory rather than mmap'd.
const MMAP_THRESHOLD: u64 = 65536; // 64 KB

/// A file handle that provides zero-copy access to file data.
/// Uses mmap for large files and buffered reads for small files.
pub enum FileHandle {
    /// Memory-mapped file (for files >= MMAP_THRESHOLD).
    Mapped(Mmap),
    /// Buffered file data in memory (for files < MMAP_THRESHOLD).
    Buffered(Vec<u8>),
    /// Zero-byte file.
    Empty,
}

impl FileHandle {
    /// Open a file, choosing mmap or buffered read based on file size.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        let file_size = meta.len();

        if file_size == 0 {
            return Ok(FileHandle::Empty);
        }

        if file_size >= MMAP_THRESHOLD {
            let file = std::fs::File::open(path)?;
            // SAFETY: The file is opened read-only and the mapping is immutable.
            // We verify the mapped size matches the expected size to detect
            // truncation between metadata read and mmap creation.
            let mmap = unsafe { Mmap::map(&file)? };
            if mmap.len() as u64 != file_size {
                // File was modified between metadata and mmap — fall back to buffered read
                let data = std::fs::read(path)?;
                return Ok(FileHandle::Buffered(data));
            }
            Ok(FileHandle::Mapped(mmap))
        } else {
            let data = std::fs::read(path)?;
            Ok(FileHandle::Buffered(data))
        }
    }

    /// Get the entire file contents as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            FileHandle::Mapped(mmap) => mmap.as_ref(),
            FileHandle::Buffered(vec) => vec.as_slice(),
            FileHandle::Empty => &[],
        }
    }

    /// Get the file size.
    pub fn len(&self) -> usize {
        match self {
            FileHandle::Mapped(mmap) => mmap.len(),
            FileHandle::Buffered(vec) => vec.len(),
            FileHandle::Empty => 0,
        }
    }

    /// Check if the file is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get a slice of the file data at the given offset.
    pub fn slice(&self, offset: usize, len: usize) -> &[u8] {
        let data = self.as_bytes();
        let end = std::cmp::min(offset + len, data.len());
        if offset < data.len() {
            &data[offset..end]
        } else {
            &[]
        }
    }
}

/// Check that no component of the path is a symlink.
fn check_no_symlinks(path: &Path) -> Result<(), FsError> {
    // Check the target file/dir itself
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(FsError::AccessViolation(format!(
                "symlinks not allowed: {}",
                path.display()
            )));
        }
    }
    // Also check parent components
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if let Ok(meta) = std::fs::symlink_metadata(&current) {
            if meta.file_type().is_symlink() {
                return Err(FsError::AccessViolation(format!(
                    "symlink in path: {}",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_basic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("test.bin"), b"hello").unwrap();
        let resolved = resolve_path(root, "test.bin", true, true).unwrap();
        assert!(resolved.ends_with("test.bin"));
    }

    #[test]
    fn test_reject_traversal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_path(dir.path(), "../../etc/passwd", true, true).is_err());
    }

    #[test]
    fn test_reject_tilde() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_path(dir.path(), "~root/.ssh/keys", true, true).is_err());
    }

    #[test]
    fn test_reject_control_chars() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_path(dir.path(), "file\x01name", true, true).is_err());
    }

    #[test]
    fn test_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_path(dir.path(), "nonexistent.bin", true, true).is_err());
    }

    #[test]
    fn test_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("firmware")).unwrap();
        fs::write(root.join("firmware/test.bin"), b"data").unwrap();
        let resolved = resolve_path(root, "firmware/test.bin", true, true).unwrap();
        assert!(resolved.ends_with("test.bin"));
    }

    #[test]
    fn test_empty_path() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_path(dir.path(), "", true, true).is_err());
    }

    #[test]
    fn test_leading_slash_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("test.bin"), b"hello").unwrap();
        let resolved = resolve_path(root, "/test.bin", true, true).unwrap();
        assert!(resolved.ends_with("test.bin"));
    }

    #[test]
    fn test_virtual_roots() {
        let main_dir = tempfile::tempdir().unwrap();
        let vroot_dir = tempfile::tempdir().unwrap();

        // File in virtual root
        fs::write(vroot_dir.path().join("fw.bin"), b"firmware").unwrap();

        let mut map = HashMap::new();
        map.insert(
            "firmware".to_string(),
            vroot_dir.path().to_string_lossy().to_string(),
        );
        let vroots = VirtualRoots::new(&map);

        let resolved =
            resolve_path_with_virtual(main_dir.path(), &vroots, "firmware/fw.bin", true, true)
                .unwrap();
        assert!(resolved.ends_with("fw.bin"));

        // File not in virtual root falls back to main
        fs::write(main_dir.path().join("main.bin"), b"main").unwrap();
        let resolved =
            resolve_path_with_virtual(main_dir.path(), &vroots, "main.bin", true, true).unwrap();
        assert!(resolved.ends_with("main.bin"));
    }
}
