//! Temporary filesystem utilities for large operations.
//!
//! Provides utilities for storing large data in temporary files
//! to reduce memory usage during agent operations.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, TempDir};

/// Temporary storage for large data operations.
///
/// Uses the filesystem to store data that would otherwise
/// consume too much memory.
#[derive(Debug)]
pub struct TempStorage {
    /// The temporary directory for this storage instance.
    dir: io::Result<TempDir>,
}

impl TempStorage {
    /// Create a new temporary storage instance.
    pub fn new() -> io::Result<Self> {
        let dir =
            TempDir::new().map_err(|e| storage_init_error("creating temporary storage", e))?;
        Ok(Self { dir: Ok(dir) })
    }

    /// Create temporary storage inside a specific parent directory.
    pub fn new_in(parent: &Path) -> io::Result<Self> {
        let dir = TempDir::new_in(parent).map_err(|e| {
            storage_init_error(
                &format!("creating temporary storage in {}", parent.display()),
                e,
            )
        })?;
        Ok(Self { dir: Ok(dir) })
    }

    /// Create temporary storage with a custom prefix.
    pub fn with_prefix(prefix: &str) -> io::Result<Self> {
        let dir = TempDir::with_prefix(prefix)
            .map_err(|e| storage_init_error("creating prefixed temporary storage", e))?;
        Ok(Self { dir: Ok(dir) })
    }

    /// Get the path to the temporary directory.
    ///
    /// If `Default` could not initialize storage, returns an invalid NUL path
    /// that cannot address any filesystem location. Use `try_path` for the error.
    pub fn path(&self) -> &Path {
        self.try_path().unwrap_or_else(|_| Path::new("\0"))
    }

    /// Get the directory path, propagating any default-initialization failure.
    pub fn try_path(&self) -> io::Result<&Path> {
        match &self.dir {
            Ok(dir) => Ok(dir.path()),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }

    fn from_initialization(result: io::Result<Self>) -> Self {
        result.unwrap_or_else(|error| {
            log::error!("TempStorage initialization failed: {error}");
            Self { dir: Err(error) }
        })
    }

    /// Store bytes in a temporary file (blocking).
    pub fn store_bytes(&self, name: &str, data: &[u8]) -> io::Result<PathBuf> {
        let path = self.try_path()?.join(name);
        std::fs::write(&path, data)?;
        Ok(path)
    }

    /// Store bytes in a temporary file — async, uses io_uring when available.
    pub async fn store_bytes_async(&self, name: &str, data: &[u8]) -> io::Result<PathBuf> {
        let path = self.try_path()?.join(name);
        tokio::fs::write(&path, data).await?;
        Ok(path)
    }

    /// Store string data in a temporary file.
    pub fn store_string(&self, name: &str, data: &str) -> io::Result<PathBuf> {
        self.store_bytes(name, data.as_bytes())
    }

    /// Store JSON data in a temporary file (blocking).
    pub fn store_json(&self, name: &str, data: &serde_json::Value) -> io::Result<PathBuf> {
        let json_str = serde_json::to_string(data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.store_string(name, &json_str)
    }

    /// Store JSON data — async, uses io_uring when available.
    pub async fn store_json_async(
        &self,
        name: &str,
        data: &serde_json::Value,
    ) -> io::Result<PathBuf> {
        let json_str = serde_json::to_string(data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.store_bytes_async(name, json_str.as_bytes()).await
    }

    /// Read bytes from a stored file (blocking).
    pub fn read_bytes(&self, name: &str) -> io::Result<Vec<u8>> {
        let path = self.try_path()?.join(name);
        std::fs::read(&path)
    }

    /// Read bytes from a stored file — async, uses io_uring when available.
    pub async fn read_bytes_async(&self, name: &str) -> io::Result<Vec<u8>> {
        let path = self.try_path()?.join(name);
        tokio::fs::read(&path).await
    }

    /// Read string from a stored file (blocking).
    pub fn read_string(&self, name: &str) -> io::Result<String> {
        let path = self.try_path()?.join(name);
        std::fs::read_to_string(&path)
    }

    /// Read string from a stored file — async, uses io_uring when available.
    pub async fn read_string_async(&self, name: &str) -> io::Result<String> {
        let data = self.read_bytes_async(name).await?;
        String::from_utf8(data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Read JSON from a stored file (blocking).
    pub fn read_json(&self, name: &str) -> io::Result<serde_json::Value> {
        let content = self.read_string(name)?;
        serde_json::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Read JSON — async, uses io_uring when available.
    pub async fn read_json_async(&self, name: &str) -> io::Result<serde_json::Value> {
        let content = self.read_string_async(name).await?;
        serde_json::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Check if a file exists in storage.
    pub fn exists(&self, name: &str) -> bool {
        self.try_path()
            .map(|path| path.join(name).exists())
            .unwrap_or(false)
    }

    /// Remove a file from storage (blocking).
    pub fn remove(&self, name: &str) -> io::Result<()> {
        let path = self.try_path()?.join(name);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Remove a file from storage — async, uses io_uring when available.
    pub async fn remove_async(&self, name: &str) -> io::Result<()> {
        let path = self.try_path()?.join(name);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    /// List all files in storage.
    pub fn list(&self) -> io::Result<Vec<String>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(self.try_path()?)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                files.push(name.to_string());
            }
        }
        Ok(files)
    }

    /// Get total size of all stored files in bytes.
    pub fn total_size(&self) -> io::Result<u64> {
        let mut total = 0;
        for entry in std::fs::read_dir(self.try_path()?)? {
            let entry = entry?;
            total += entry.metadata()?.len();
        }
        Ok(total)
    }

    /// Create a new temporary file for streaming writes.
    pub fn create_temp_file(&self) -> io::Result<TempFile> {
        let file = NamedTempFile::new_in(self.try_path()?)?;
        Ok(TempFile { file })
    }

    /// Persist the storage directory (prevents automatic cleanup).
    /// Returns an invalid NUL path if default initialization failed.
    /// Use `try_persist` to receive the storage error.
    pub fn persist(self) -> PathBuf {
        self.try_persist().unwrap_or_else(|_| PathBuf::from("\0"))
    }

    /// Persist initialized storage, propagating any initialization failure.
    pub fn try_persist(self) -> io::Result<PathBuf> {
        self.dir.map(TempDir::keep)
    }
}

fn storage_init_error(context: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{context}: {error}"))
}

impl Default for TempStorage {
    /// Retain an initialization error for subsequent I/O rather than panicking
    /// or redirecting storage. Prefer `new` when construction must be fallible.
    fn default() -> Self {
        Self::from_initialization(Self::new())
    }
}

/// A temporary file for streaming operations.
#[derive(Debug)]
pub struct TempFile {
    file: NamedTempFile,
}

impl TempFile {
    /// Create a new temporary file.
    pub fn new() -> io::Result<Self> {
        let file = NamedTempFile::new()?;
        Ok(Self { file })
    }

    /// Get the path to the temporary file.
    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// Write bytes to the file.
    pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.file.write_all(data)
    }

    /// Write a string to the file.
    pub fn write_str(&mut self, data: &str) -> io::Result<()> {
        self.write_all(data.as_bytes())
    }

    /// Flush the file buffer.
    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    /// Read all contents from the file.
    pub fn read_all(&mut self) -> io::Result<Vec<u8>> {
        use std::io::Seek;
        self.file.seek(std::io::SeekFrom::Start(0))?;
        let mut contents = Vec::new();
        self.file.read_to_end(&mut contents)?;
        Ok(contents)
    }

    /// Read contents as string.
    pub fn read_string(&mut self) -> io::Result<String> {
        let bytes = self.read_all()?;
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Persist the file (returns the path and prevents automatic cleanup).
    pub fn persist(self) -> io::Result<PathBuf> {
        let (_, path) = self.file.keep()?;
        Ok(path)
    }
}

impl Write for TempFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Read for TempFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_safety_temp_storage_initialization_failure() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("not-a-directory");
        std::fs::write(&parent, b"keep").unwrap();
        let error = TempStorage::new_in(&parent).unwrap_err();
        assert!(error.to_string().contains("creating temporary storage in"));
        // Exercise the same error-retaining path used by Default, without
        // changing process-wide TMPDIR or touching a production location.
        let storage = TempStorage::from_initialization(TempStorage::new_in(&parent));
        let kind = storage.try_path().unwrap_err().kind();
        assert_eq!(
            storage.store_bytes("data", b"data").unwrap_err().kind(),
            kind
        );
        assert_eq!(
            storage
                .store_bytes_async("data", b"data")
                .await
                .unwrap_err()
                .kind(),
            kind
        );
        assert!(storage.read_bytes("data").is_err());
        assert!(storage.read_bytes_async("data").await.is_err());
        assert!(storage.remove("data").is_err());
        assert!(storage.remove_async("data").await.is_err());
        assert!(storage.list().is_err());
        assert!(storage.total_size().is_err());
        assert!(storage.create_temp_file().is_err());
        assert!(!storage.exists("data"));
        assert_eq!(storage.path(), Path::new("\0"));
        assert!(std::fs::write(storage.path().join("data"), b"data").is_err());
        assert!(storage.try_persist().is_err());
        let storage = TempStorage::from_initialization(TempStorage::new_in(&parent));
        assert_eq!(storage.persist(), PathBuf::from("\0"));
        assert_eq!(std::fs::read(&parent).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn runtime_safety_temp_storage_cleanup_and_io_errors() {
        let root = tempfile::tempdir().unwrap();
        let storage = TempStorage::new_in(root.path()).unwrap();
        let path = storage.path().to_owned();
        storage.store_bytes_async("data", b"hello").await.unwrap();
        assert_eq!(storage.read_bytes("data").unwrap(), b"hello");
        assert!(storage.store_bytes("missing/data", b"data").is_err());
        assert!(storage
            .store_bytes_async("missing/data", b"data")
            .await
            .is_err());
        assert!(storage.read_bytes("missing").is_err());
        drop(storage);
        assert!(!path.exists());
        let storage = TempStorage::new_in(root.path()).unwrap();
        storage.store_bytes("persisted", b"saved").unwrap();
        let path = storage.try_persist().unwrap();
        assert_eq!(std::fs::read(path.join("persisted")).unwrap(), b"saved");
        // root owns and cleans the explicitly persisted test directory.
    }

    #[test]
    fn test_temp_storage_basic() {
        let storage = TempStorage::new().unwrap();

        // Store and read bytes
        let data = b"hello world";
        storage.store_bytes("test.txt", data).unwrap();
        let read = storage.read_bytes("test.txt").unwrap();
        assert_eq!(read, data);

        // Check exists
        assert!(storage.exists("test.txt"));
        assert!(!storage.exists("nonexistent.txt"));

        // List files
        let files = storage.list().unwrap();
        assert!(files.contains(&"test.txt".to_string()));
    }

    #[test]
    fn test_temp_storage_json() {
        let storage = TempStorage::new().unwrap();

        let json = serde_json::json!({
            "name": "test",
            "value": 42
        });

        storage.store_json("data.json", &json).unwrap();
        let read = storage.read_json("data.json").unwrap();
        assert_eq!(read, json);
    }

    #[test]
    fn test_temp_file() {
        let mut file = TempFile::new().unwrap();

        file.write_str("hello").unwrap();
        file.flush().unwrap();

        let content = file.read_string().unwrap();
        assert_eq!(content, "hello");
    }
}
