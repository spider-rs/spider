use std::io::{self, Read, Seek};
use std::ops::{Deref, DerefMut};
use std::path::Path;

use zip::{
    read::ZipFile,
    result::{ZipError, ZipResult},
};

#[derive(Clone, Debug)]
pub struct ZipArchive<R: Read + Seek>(zip::ZipArchive<R>);

impl<R: Read + Seek> Deref for ZipArchive<R> {
    type Target = zip::ZipArchive<R>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<R: Read + Seek> DerefMut for ZipArchive<R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<R: Read + Seek> ZipArchive<R> {
    pub fn new(reader: R) -> ZipResult<Self> {
        zip::ZipArchive::new(reader).map(|z| Self(z))
    }

    /// We need this custom extract function to support symlinks.
    /// This is based on https://github.com/zip-rs/zip/pull/213.
    ///
    /// We must be careful with this implementation since it is not
    /// protected against malicious symlinks, but we trust the binaries
    /// provided by chromium.
    pub fn extract<P: AsRef<Path>>(&mut self, directory: P) -> ZipResult<()> {
        use std::fs;
        for i in 0..self.len() {
            let mut file = self.by_index(i)?;
            let filepath = file
                .enclosed_name()
                .ok_or(ZipError::InvalidArchive("Invalid file path"))?;
            let outpath = directory.as_ref().join(filepath);
            if file.name().ends_with('/') {
                fs::create_dir_all(&outpath)?;
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        fs::create_dir_all(p)?;
                    }
                }

                match read_symlink(&mut file)? {
                    Some(target) => {
                        create_symlink(target, &outpath)?;
                    }
                    None => {
                        let mut outfile = fs::File::create(&outpath)?;
                        io::copy(&mut file, &mut outfile)?;

                        // Get and Set permissions
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if let Some(mode) = file.unix_mode() {
                                fs::set_permissions(&outpath, fs::Permissions::from_mode(mode))?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn read_symlink(entry: &mut ZipFile<'_>) -> ZipResult<Option<Vec<u8>>> {
    if let Some(mode) = entry.unix_mode() {
        const S_IFLNK: u32 = 0o120000; // symbolic link
        if mode & S_IFLNK == S_IFLNK {
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents)?;
            return Ok(Some(contents));
        }
    }
    Ok(None)
}

#[cfg(target_family = "unix")]
fn create_symlink(link_target: Vec<u8>, link_path: &Path) -> ZipResult<()> {
    use std::os::unix::ffi::OsStringExt as _;

    let link_target = std::ffi::OsString::from_vec(link_target);
    std::os::unix::fs::symlink(link_target, link_path)?;

    Ok(())
}

#[cfg(target_family = "windows")]
fn create_symlink(link_target: Vec<u8>, link_path: &Path) -> ZipResult<()> {
    // Only supports UTF-8 paths which is enough for our usecase
    let link_target = String::from_utf8(link_target)
        .map_err(|_| ZipError::InvalidArchive("Invalid synmlink target name"))?;
    std::os::windows::fs::symlink_file(link_target, link_path)?;

    Ok(())
}
