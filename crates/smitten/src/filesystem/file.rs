//! Files kept open for reading or writing a piece at a time.

use std::io::{self, BufRead, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::search::symlink_refused;
use super::{FileType, Lookup, lookup, save_path, with_search};
use crate::context;

/// What a [`File`] is opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileMode {
    /// Reading, from wherever [`super::read`] would find the file.
    Read,
    /// Writing to the save directory, emptying the file first.
    Write,
    /// Writing to the end of a file in the save directory.
    Append,
    /// Not open. A [`File`] made with this mode, or one that has been
    /// [closed](File::close), knows its name and can be
    /// [opened](File::open) later.
    Closed,
}

/// How writes to a [`File`] are held back before they reach the disk. Set
/// it with [`File::set_buffer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BufferMode {
    /// Every write goes straight to the disk.
    None,
    /// Writes are held until a line ends, or until this many bytes are
    /// waiting. Good for a log that should be complete up to its last
    /// line even if the game crashes.
    Line(usize),
    /// Writes are held until this many bytes are waiting, or the file is
    /// flushed or closed. The fastest for writing a lot.
    Full(usize),
}

impl BufferMode {
    /// The size of the buffer a [`File`] starts with.
    pub const DEFAULT_SIZE: usize = 8 * 1024;

    fn capacity(self) -> usize {
        match self {
            BufferMode::None => 0,
            BufferMode::Line(size) | BufferMode::Full(size) => size,
        }
    }
}

impl Default for BufferMode {
    /// Full buffering with [`BufferMode::DEFAULT_SIZE`] bytes, so that
    /// writing a piece at a time is not slow.
    fn default() -> Self {
        BufferMode::Full(BufferMode::DEFAULT_SIZE)
    }
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
/// are flushed. Call [`File::flush`] or [`File::close`] first to find out
/// whether they succeeded. A closed file can be opened again with
/// [`File::open`], in the same mode or another.
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
    /// `None` while the file is closed.
    stream: Option<Stream>,
    filename: PathBuf,
    mode: FileMode,
    buffer: BufferMode,
}

/// Opens a file to read or write a piece at a time.
///
/// For [`FileMode::Read`], a relative path is looked up as in
/// [`super::read`]: the save directory, then the asset directory and
/// anything mounted. An absolute path, such as that of a file the
/// player dropped on the window, is opened as it is. For writing, the
/// path must be relative and the file is in the save directory, with any
/// missing parent directories created. [`FileMode::Closed`] touches
/// nothing and gives a `File` to [open](File::open) later.
pub fn open_file(path: impl AsRef<Path>, mode: FileMode) -> io::Result<File> {
    let mut file = File {
        stream: None,
        filename: path.as_ref().to_path_buf(),
        mode: FileMode::Closed,
        buffer: BufferMode::default(),
    };
    if mode != FileMode::Closed {
        file.open(mode)?;
    }
    Ok(file)
}

fn open_for_reading(path: &Path) -> io::Result<Stream> {
    let names = match context::with(|ctx| lookup(ctx, path)) {
        Lookup::Disk(file) => return open_disk(&file),
        Lookup::Search(names) => names,
    };
    let found = with_search(|search, save, assets| {
        let found = search.find(save, assets, &names)?;
        if found.info.kind == FileType::Symlink {
            return Some(Err(Err(symlink_refused())));
        }
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

fn open_for_writing(target: &Path, mode: FileMode, buffer: BufferMode) -> io::Result<Stream> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    if mode == FileMode::Append {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    let file = options.open(target)?;
    Ok(Stream::Writer(BufWriter::with_capacity(
        buffer.capacity(),
        file,
    )))
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

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "the file is closed")
}

impl File {
    /// The path the file was opened with.
    pub fn filename(&self) -> &Path {
        &self.filename
    }

    /// What the file is open for, or [`FileMode::Closed`].
    pub fn mode(&self) -> FileMode {
        self.mode
    }

    /// Whether the file is open.
    pub fn is_open(&self) -> bool {
        self.stream.is_some()
    }

    /// Opens a closed file again, by the name it was made with. The path is
    /// looked up afresh, as [`open_file`] does, and reading or writing
    /// starts at the beginning, or at the end for [`FileMode::Append`].
    ///
    /// Fails if the file is already open; [close](File::close) it first
    /// to change its mode.
    pub fn open(&mut self, mode: FileMode) -> io::Result<()> {
        if self.is_open() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is already open", self.filename.display()),
            ));
        }
        let stream = match mode {
            FileMode::Read => open_for_reading(&self.filename)?,
            FileMode::Write | FileMode::Append => {
                let target = save_path(&self.filename)?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                open_for_writing(&target, mode, self.buffer)?
            }
            FileMode::Closed => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a file cannot be opened in the closed mode",
                ));
            }
        };
        self.stream = Some(stream);
        self.mode = mode;
        Ok(())
    }

    /// Flushes any buffered writes and closes the file. The `File` keeps
    /// its name and can be [opened](File::open) again. Closing a closed
    /// file does nothing.
    ///
    /// The file is closed even if the flush fails; the error says the
    /// last writes may be lost.
    pub fn close(&mut self) -> io::Result<()> {
        let stream = self.stream.take();
        self.mode = FileMode::Closed;
        match stream {
            Some(Stream::Writer(mut writer)) => writer.flush(),
            _ => Ok(()),
        }
    }

    /// How writes are buffered.
    pub fn buffer(&self) -> BufferMode {
        self.buffer
    }

    /// Changes how writes are buffered. Anything already buffered is
    /// written first. The mode is kept if the file is closed and opened
    /// again.
    ///
    /// Only writing is affected. Reading always uses a buffer, which does
    /// not change what is read.
    pub fn set_buffer(&mut self, mode: BufferMode) -> io::Result<()> {
        self.buffer = mode;
        let Some(Stream::Writer(writer)) = &mut self.stream else {
            return Ok(());
        };
        writer.flush()?;
        let Some(Stream::Writer(writer)) = self.stream.take() else {
            unreachable!("the stream was a writer a moment ago");
        };
        match writer.into_inner() {
            Ok(file) => {
                let writer = BufWriter::with_capacity(mode.capacity(), file);
                self.stream = Some(Stream::Writer(writer));
                Ok(())
            }
            Err(error) => {
                let kind = error.error().kind();
                let message = error.error().to_string();
                self.stream = Some(Stream::Writer(error.into_inner()));
                Err(io::Error::new(kind, message))
            }
        }
    }

    /// The file's size in bytes, including anything written but not yet
    /// flushed. A closed file is looked up by its name.
    pub fn size(&mut self) -> io::Result<u64> {
        match &mut self.stream {
            Some(Stream::Disk(reader)) => Ok(reader.get_ref().metadata()?.len()),
            Some(Stream::Memory(cursor)) => Ok(cursor.get_ref().len() as u64),
            Some(Stream::Writer(writer)) => {
                writer.flush()?;
                Ok(writer.get_ref().metadata()?.len())
            }
            None => match super::info(&self.filename) {
                Some(info) => Ok(info.size),
                None => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} was not found", self.filename.display()),
                )),
            },
        }
    }

    /// How many bytes from the start the next read or write happens at.
    pub fn tell(&mut self) -> io::Result<u64> {
        self.stream_position()
    }

    /// Whether everything has been read. Always true for a file opened for
    /// writing.
    pub fn is_eof(&mut self) -> io::Result<bool> {
        match &mut self.stream {
            Some(Stream::Writer(_)) => Ok(true),
            None => Err(closed()),
            _ => Ok(self.fill_buf()?.is_empty()),
        }
    }

    /// Reads up to `count` bytes, fewer only at the end of the file.
    pub fn read_bytes(&mut self, count: usize) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        Read::by_ref(self)
            .take(count as u64)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Reads everything from here to the end of the file.
    pub fn read_all(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Writes all of `contents`.
    pub fn write_bytes(&mut self, contents: impl AsRef<[u8]>) -> io::Result<()> {
        self.write_all(contents.as_ref())
    }
}

impl Read for File {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match &mut self.stream {
            Some(Stream::Disk(reader)) => reader.read(buffer),
            Some(Stream::Memory(cursor)) => cursor.read(buffer),
            Some(Stream::Writer(_)) => Err(not_readable()),
            None => Err(closed()),
        }
    }
}

impl BufRead for File {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        match &mut self.stream {
            Some(Stream::Disk(reader)) => reader.fill_buf(),
            Some(Stream::Memory(cursor)) => cursor.fill_buf(),
            Some(Stream::Writer(_)) => Err(not_readable()),
            None => Err(closed()),
        }
    }

    fn consume(&mut self, amount: usize) {
        match &mut self.stream {
            Some(Stream::Disk(reader)) => reader.consume(amount),
            Some(Stream::Memory(cursor)) => cursor.consume(amount),
            _ => {}
        }
    }
}

impl Write for File {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match &mut self.stream {
            Some(Stream::Writer(writer)) => {
                let written = writer.write(buffer)?;
                if matches!(self.buffer, BufferMode::Line(_)) && buffer[..written].contains(&b'\n')
                {
                    writer.flush()?;
                }
                Ok(written)
            }
            None => Err(closed()),
            _ => Err(not_writable()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.stream {
            Some(Stream::Writer(writer)) => writer.flush(),
            _ => Ok(()),
        }
    }
}

impl Seek for File {
    /// Moves where the next read or write happens. In a file opened with
    /// [`FileMode::Append`], writes still go to the end.
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match &mut self.stream {
            Some(Stream::Disk(reader)) => reader.seek(position),
            Some(Stream::Memory(cursor)) => cursor.seek(position),
            Some(Stream::Writer(writer)) => writer.seek(position),
            None => Err(closed()),
        }
    }
}

impl std::fmt::Debug for File {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("File")
            .field("filename", &self.filename)
            .field("mode", &self.mode)
            .field("buffer", &self.buffer)
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
            _ => open_for_writing(path, mode, BufferMode::default()).unwrap(),
        };
        File {
            stream: Some(stream),
            filename: path.to_path_buf(),
            mode,
            buffer: BufferMode::default(),
        }
    }

    #[test]
    fn a_closed_file_keeps_its_name_and_flushes() {
        let dir = TempDir::new("file-close");
        let path = dir.0.join("log.txt");

        let mut file = disk(&path, FileMode::Write);
        assert!(file.is_open());
        file.write_bytes("kept").unwrap();
        file.close().unwrap();
        assert!(!file.is_open());
        assert_eq!(file.mode(), FileMode::Closed);
        assert_eq!(file.filename(), path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept");
        file.close().unwrap();

        assert_eq!(
            file.write(b"x").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            file.read_bytes(1).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(file.is_eof().is_err());
        assert!(file.open(FileMode::Closed).is_err());

        let mut open = disk(&path, FileMode::Read);
        assert!(open.open(FileMode::Read).is_err());
    }

    #[test]
    fn buffer_modes_decide_when_writes_reach_the_disk() {
        let dir = TempDir::new("file-buffer");
        let path = dir.0.join("log.txt");
        let on_disk = || std::fs::read_to_string(&path).unwrap();

        let mut file = disk(&path, FileMode::Write);
        assert_eq!(file.buffer(), BufferMode::Full(BufferMode::DEFAULT_SIZE));
        file.write_bytes("held\n").unwrap();
        assert_eq!(on_disk(), "");

        file.set_buffer(BufferMode::None).unwrap();
        assert_eq!(on_disk(), "held\n");
        file.write_bytes("direct").unwrap();
        assert_eq!(on_disk(), "held\ndirect");

        file.set_buffer(BufferMode::Line(1024)).unwrap();
        assert_eq!(file.buffer(), BufferMode::Line(1024));
        file.write_bytes(" line").unwrap();
        assert_eq!(on_disk(), "held\ndirect");
        file.write_bytes(" ends\nnext").unwrap();
        assert_eq!(on_disk(), "held\ndirect line ends\nnext");
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
            stream: Some(Stream::Memory(Cursor::new(b"zipped\nlines".to_vec()))),
            filename: PathBuf::from("pack/a.txt"),
            mode: FileMode::Read,
            buffer: BufferMode::default(),
        };
        assert_eq!(file.size().unwrap(), 12);
        let mut line = String::new();
        file.read_line(&mut line).unwrap();
        assert_eq!(line, "zipped\n");
        assert_eq!(file.read_all().unwrap(), b"lines");
        assert!(file.is_eof().unwrap());
    }
}
