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
    dir: TempDir,
}

impl TempStorage {
    /// Create a new temporary storage instance.
    pub fn new() -> io::Result<Self> {
        let dir = TempDir::new()?;
        Ok(Self { dir })
    }

    /// Create temporary storage with a custom prefix.
    pub fn with_prefix(prefix: &str) -> io::Result<Self> {
        let dir = TempDir::with_prefix(prefix)?;
        Ok(Self { dir })
    }

    /// Get the path to the temporary directory.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Store bytes in a temporary file.
    pub fn store_bytes(&self, name: &str, data: &[u8]) -> io::Result<PathBuf> {
        let path = self.dir.path().join(name);
        std::fs::write(&path, data)?;
        Ok(path)
    }

    /// Store string data in a temporary file.
    pub fn store_string(&self, name: &str, data: &str) -> io::Result<PathBuf> {
        self.store_bytes(name, data.as_bytes())
    }

    /// Store JSON data in a temporary file.
    pub fn store_json(&self, name: &str, data: &serde_json::Value) -> io::Result<PathBuf> {
        let json_str = serde_json::to_string(data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.store_string(name, &json_str)
    }

    /// Read bytes from a stored file.
    pub fn read_bytes(&self, name: &str) -> io::Result<Vec<u8>> {
        let path = self.dir.path().join(name);
        std::fs::read(&path)
    }

    /// Read string from a stored file.
    pub fn read_string(&self, name: &str) -> io::Result<String> {
        let path = self.dir.path().join(name);
        std::fs::read_to_string(&path)
    }

    /// Read JSON from a stored file.
    pub fn read_json(&self, name: &str) -> io::Result<serde_json::Value> {
        let content = self.read_string(name)?;
        serde_json::from_str(&content)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Check if a file exists in storage.
    pub fn exists(&self, name: &str) -> bool {
        self.dir.path().join(name).exists()
    }

    /// Remove a file from storage.
    pub fn remove(&self, name: &str) -> io::Result<()> {
        let path = self.dir.path().join(name);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// List all files in storage.
    pub fn list(&self) -> io::Result<Vec<String>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(self.dir.path())? {
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
        for entry in std::fs::read_dir(self.dir.path())? {
            let entry = entry?;
            total += entry.metadata()?.len();
        }
        Ok(total)
    }

    /// Create a new temporary file for streaming writes.
    pub fn create_temp_file(&self) -> io::Result<TempFile> {
        let file = NamedTempFile::new_in(self.dir.path())?;
        Ok(TempFile { file })
    }

    /// Persist the storage directory (prevents automatic cleanup).
    pub fn persist(self) -> PathBuf {
        self.dir.keep()
    }
}

impl Default for TempStorage {
    fn default() -> Self {
        Self::new().expect("Failed to create temporary storage")
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
        String::from_utf8(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
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
