//! Files kept open for reading or writing a piece at a time, the
//! equivalent of LÖVE's `File` object.

use std::io::{self, BufRead, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::{Lookup, lookup, save_path, with_search};
use crate::context;

/// What a [`File`] is opened for, as LÖVE's `FileMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileMode {
    /// Reading, from wherever [`super::read`] would find the file.
    Read,
    /// Writing to the save directory, emptying the file first.
    Write,
    /// Writing to the end of a file in the save directory.
    Append,
}

enum Stream {
    Disk(BufReader<std::fs::File>),
    /// A file inside a mounted zip archive, which is read whole when it
    /// is opened.
    Memory(Cursor<Vec<u8>>),
    Writer(BufWriter<std::fs::File>),
}

/// An open file, read or written a piece at a time instead of all at
/// once, for files too large to hold in memory or written over time,
/// such as a log.
///
/// A `File` implements [`Read`], [`BufRead`], [`Write`] and [`Seek`], so
/// it works with anything in the standard library that takes those. The
/// file is closed when the `File` is dropped, after any buffered writes
/// are flushed. Call [`File::flush`] first to find out whether they
/// succeeded.
///
/// ```no_run
/// use std::io::{BufRead, Write};
/// use smitten::filesystem::{self, FileMode};
///
/// let mut log = filesystem::open_file("log.txt", FileMode::Append).unwrap();
/// writeln!(log, "level 2 reached").unwrap();
///
/// let file = filesystem::open_file("levels.txt", FileMode::Read).unwrap();
/// for line in file.lines() {
///     println!("{}", line.unwrap());
/// }
/// ```
pub struct File {
    stream: Stream,
    filename: PathBuf,
    mode: FileMode,
}

/// Opens a file to read or write a piece at a time, the equivalent of
/// `love.filesystem.openFile`.
///
/// For [`FileMode::Read`], a relative path is looked up as in
/// [`super::read`]: the save directory, then the asset directory and
/// anything mounted. An absolute path, such as one passed to
/// [`crate::Game::file_dropped`], is opened as it is. For writing, the
/// path must be relative and the file is in the save directory, with any
/// missing parent directories created.
pub fn open_file(path: impl AsRef<Path>, mode: FileMode) -> io::Result<File> {
    let path = path.as_ref();
    let stream = match mode {
        FileMode::Read => open_for_reading(path)?,
        FileMode::Write | FileMode::Append => {
            let target = save_path(path)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            open_for_writing(&target, mode)?
        }
    };
    Ok(File {
        stream,
        filename: path.to_path_buf(),
        mode,
    })
}

fn open_for_reading(path: &Path) -> io::Result<Stream> {
    let names = match context::with(|ctx| lookup(ctx, path)) {
        Lookup::Disk(file) => return open_disk(&file),
        Lookup::Search(names) => names,
    };
    let found = with_search(|search, save, assets| {
        let found = search.find(save, assets, &names)?;
        Some(match &found.location {
            super::search::Location::Disk(file) => Ok(file.clone()),
            _ => Err(search.read(&found)),
        })
    });
    match found {
        Some(Ok(file)) => open_disk(&file),
        Some(Err(bytes)) => Ok(Stream::Memory(Cursor::new(bytes?))),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} was not found", path.display()),
        )),
    }
}

fn open_disk(file: &Path) -> io::Result<Stream> {
    let opened = std::fs::File::open(file)?;
    if opened.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is a directory", file.display()),
        ));
    }
    Ok(Stream::Disk(BufReader::new(opened)))
}

fn open_for_writing(target: &Path, mode: FileMode) -> io::Result<Stream> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    if mode == FileMode::Append {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    Ok(Stream::Writer(BufWriter::new(options.open(target)?)))
}

fn not_readable() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the file was opened for writing, not reading",
    )
}

fn not_writable() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the file was opened for reading, not writing",
    )
}

impl File {
    /// The path the file was opened with.
    pub fn filename(&self) -> &Path {
        &self.filename
    }

    /// What the file was opened for.
    pub fn mode(&self) -> FileMode {
        self.mode
    }

    /// The file's size in bytes, including anything written but not yet
    /// flushed.
    pub fn size(&mut self) -> io::Result<u64> {
        match &mut self.stream {
            Stream::Disk(reader) => Ok(reader.get_ref().metadata()?.len()),
            Stream::Memory(cursor) => Ok(cursor.get_ref().len() as u64),
            Stream::Writer(writer) => {
                writer.flush()?;
                Ok(writer.get_ref().metadata()?.len())
            }
        }
    }

    /// How many bytes from the start the next read or write happens at,
    /// the equivalent of `File:tell`.
    pub fn tell(&mut self) -> io::Result<u64> {
        self.stream_position()
    }

    /// Whether everything has been read, the equivalent of `File:isEOF`.
    /// Always true for a file opened for writing.
    pub fn is_eof(&mut self) -> io::Result<bool> {
        match &mut self.stream {
            Stream::Writer(_) => Ok(true),
            _ => Ok(self.fill_buf()?.is_empty()),
        }
    }

    /// Reads up to `count` bytes, fewer only at the end of the file, the
    /// equivalent of `File:read(count)`.
    pub fn read_bytes(&mut self, count: usize) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        Read::by_ref(self)
            .take(count as u64)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Reads everything from here to the end of the file, the equivalent
    /// of `File:read()`.
    pub fn read_all(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Writes all of `contents`, the equivalent of `File:write`.
    pub fn write_bytes(&mut self, contents: impl AsRef<[u8]>) -> io::Result<()> {
        self.write_all(contents.as_ref())
    }
}

impl Read for File {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match &mut self.stream {
            Stream::Disk(reader) => reader.read(buffer),
            Stream::Memory(cursor) => cursor.read(buffer),
            Stream::Writer(_) => Err(not_readable()),
        }
    }
}

impl BufRead for File {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        match &mut self.stream {
            Stream::Disk(reader) => reader.fill_buf(),
            Stream::Memory(cursor) => cursor.fill_buf(),
            Stream::Writer(_) => Err(not_readable()),
        }
    }

    fn consume(&mut self, amount: usize) {
        match &mut self.stream {
            Stream::Disk(reader) => reader.consume(amount),
            Stream::Memory(cursor) => cursor.consume(amount),
            Stream::Writer(_) => {}
        }
    }
}

impl Write for File {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match &mut self.stream {
            Stream::Writer(writer) => writer.write(buffer),
            _ => Err(not_writable()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.stream {
            Stream::Writer(writer) => writer.flush(),
            _ => Ok(()),
        }
    }
}

impl Seek for File {
    /// Moves where the next read or write happens. In a file opened with
    /// [`FileMode::Append`], writes still go to the end.
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match &mut self.stream {
            Stream::Disk(reader) => reader.seek(position),
            Stream::Memory(cursor) => cursor.seek(position),
            Stream::Writer(writer) => writer.seek(position),
        }
    }
}

impl std::fmt::Debug for File {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("File")
            .field("filename", &self.filename)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::tests::TempDir;

    fn disk(path: &Path, mode: FileMode) -> File {
        let stream = match mode {
            FileMode::Read => open_disk(path).unwrap(),
            _ => open_for_writing(path, mode).unwrap(),
        };
        File {
            stream,
            filename: path.to_path_buf(),
            mode,
        }
    }

    #[test]
    fn a_file_is_read_a_piece_at_a_time() {
        let dir = TempDir::new("file-read");
        let path = dir.0.join("data.bin");
        std::fs::write(&path, b"abcdefghij").unwrap();

        let mut file = disk(&path, FileMode::Read);
        assert_eq!(file.size().unwrap(), 10);
        assert_eq!(file.read_bytes(4).unwrap(), b"abcd");
        assert_eq!(file.tell().unwrap(), 4);
        assert!(!file.is_eof().unwrap());
        file.seek(SeekFrom::Start(8)).unwrap();
        assert_eq!(file.read_bytes(100).unwrap(), b"ij");
        assert!(file.is_eof().unwrap());
        file.seek(SeekFrom::Start(2)).unwrap();
        assert_eq!(file.read_all().unwrap(), b"cdefghij");
    }

    #[test]
    fn lines_are_read_one_at_a_time() {
        let dir = TempDir::new("file-lines");
        let path = dir.0.join("levels.txt");
        std::fs::write(&path, "one\ntwo\r\nthree").unwrap();

        let lines: Vec<String> = disk(&path, FileMode::Read)
            .lines()
            .collect::<io::Result<_>>()
            .unwrap();
        assert_eq!(lines, ["one", "two", "three"]);
    }

    #[test]
    fn writing_replaces_and_appending_adds() {
        let dir = TempDir::new("file-write");
        let path = dir.0.join("log.txt");
        std::fs::write(&path, "old contents").unwrap();

        let mut file = disk(&path, FileMode::Write);
        file.write_bytes("first\n").unwrap();
        assert_eq!(file.size().unwrap(), 6);
        drop(file);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");

        let mut file = disk(&path, FileMode::Append);
        writeln!(file, "second").unwrap();
        file.flush().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn a_file_refuses_the_direction_it_was_not_opened_for() {
        let dir = TempDir::new("file-direction");
        let path = dir.0.join("data.txt");
        std::fs::write(&path, "data").unwrap();

        let mut reading = disk(&path, FileMode::Read);
        assert_eq!(
            reading.write(b"x").unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        let mut writing = disk(&path, FileMode::Append);
        assert_eq!(
            writing.read_bytes(1).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert!(writing.is_eof().unwrap());
    }

    #[test]
    fn a_directory_cannot_be_opened() {
        let dir = TempDir::new("file-directory");
        let Err(error) = open_disk(&dir.0) else {
            panic!("a directory opened as a file");
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_file_inside_an_archive_is_read_from_memory() {
        let mut file = File {
            stream: Stream::Memory(Cursor::new(b"zipped\nlines".to_vec())),
            filename: PathBuf::from("pack/a.txt"),
            mode: FileMode::Read,
        };
        assert_eq!(file.size().unwrap(), 12);
        let mut line = String::new();
        file.read_line(&mut line).unwrap();
        assert_eq!(line, "zipped\n");
        assert_eq!(file.read_all().unwrap(), b"lines");
        assert!(file.is_eof().unwrap());
    }
}
