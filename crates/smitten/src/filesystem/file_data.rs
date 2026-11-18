//! A file's contents kept together with its name, the equivalent of
//! LÖVE's `FileData`.

use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};

/// The bytes of a file together with its name, as LÖVE's `FileData`.
///
/// Smitten's loaders take bytes or paths, so this is only needed where
/// the name has to travel with the contents, such as a file read once and
/// handed on to code that looks at its extension. A `FileData` derefs to
/// `[u8]`, so it can be passed anywhere a byte slice is taken.
///
/// ```no_run
/// use smitten::filesystem::FileData;
///
/// let level = FileData::load("levels/1.json").unwrap();
/// assert_eq!(level.extension(), Some("json"));
/// let text = std::str::from_utf8(&level).unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileData {
    bytes: Vec<u8>,
    filename: PathBuf,
}

impl FileData {
    /// Reads a whole file into a [`FileData`], the equivalent of
    /// `love.filesystem.newFileData(filename)`. The file is looked up as in
    /// [`super::read`].
    #[doc(alias = "love.filesystem.newFileData")]
    pub fn load(path: impl AsRef<Path>) -> io::Result<FileData> {
        let path = path.as_ref();
        Ok(FileData::new(super::read(path)?, path))
    }
}

impl FileData {
    /// Makes a `FileData` from bytes already in memory and the name they
    /// go by, the equivalent of `love.filesystem.newFileData(contents,
    /// name)`. Nothing is read or written.
    #[doc(alias = "love.filesystem.newFileData")]
    pub fn new(bytes: impl Into<Vec<u8>>, filename: impl Into<PathBuf>) -> Self {
        Self {
            bytes: bytes.into(),
            filename: filename.into(),
        }
    }

    /// The name the data goes by, the equivalent of
    /// `FileData:getFilename`.
    #[doc(alias = "FileData:getFilename")]
    pub fn filename(&self) -> &Path {
        &self.filename
    }

    /// The part of the name after its last dot, without the dot, the
    /// equivalent of `FileData:getExtension`. `None` if there is none.
    #[doc(alias = "FileData:getExtension")]
    pub fn extension(&self) -> Option<&str> {
        self.filename.extension()?.to_str()
    }

    /// The contents.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The contents, giving up the name.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Deref for FileData {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl AsRef<[u8]> for FileData {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_data_keeps_its_name_with_its_bytes() {
        let data = FileData::new(b"{}".to_vec(), "levels/1.json");
        assert_eq!(data.filename(), Path::new("levels/1.json"));
        assert_eq!(data.extension(), Some("json"));
        assert_eq!(&*data, b"{}");
        assert_eq!(data.len(), 2);
        assert_eq!(data.into_bytes(), b"{}");

        assert_eq!(FileData::new(Vec::new(), "README").extension(), None);
        assert_eq!(
            FileData::new(Vec::new(), "archive.tar.gz").extension(),
            Some("gz")
        );
    }
}
