//! The places a relative path is looked up in, and in what order: zip
//! archives and directories the game has mounted, the save directory, and
//! the game's own files, built into it or in the asset directory.
//!
//! Paths here are lists of names, already checked not to leave the place
//! they are looked up in. An empty list is the top level.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use super::{FileInfo, FileType, info_of};

/// A zip archive or directory made to look like part of the game's files.
pub(crate) struct Mount {
    /// The archive or directory on disk.
    pub file: PathBuf,
    /// Where its contents appear.
    point: Vec<String>,
    source: Source,
}

enum Source {
    Directory(PathBuf),
    Archive(Archive),
}

/// Something a zip archive can be read from: a file, or bytes in memory.
trait ReadSeek: Read + Seek {}

impl<T: Read + Seek> ReadSeek for T {}

pub(crate) struct Archive {
    zip: ZipArchive<Box<dyn ReadSeek>>,
    /// Each file by its path inside the archive, with its index there and
    /// its size once uncompressed.
    files: BTreeMap<String, (usize, u64)>,
    /// Every directory that holds something, listed or not.
    directories: BTreeSet<String>,
}

impl Archive {
    fn open(path: &Path) -> io::Result<Archive> {
        let file = BufReader::new(File::open(path)?);
        Archive::read(Box::new(file), &path.display().to_string())
    }

    /// An archive in memory, such as the game's assets built into it.
    pub fn from_bytes(bytes: &'static [u8]) -> io::Result<Archive> {
        Archive::read(Box::new(Cursor::new(bytes)), "the built-in assets")
    }

    fn read(reader: Box<dyn ReadSeek>, name: &str) -> io::Result<Archive> {
        let mut zip = ZipArchive::new(reader).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name} is not a zip archive: {error}"),
            )
        })?;
        let mut files = BTreeMap::new();
        let mut directories = BTreeSet::new();
        for index in 0..zip.len() {
            let entry = zip.by_index_raw(index).map_err(io::Error::other)?;
            // An entry whose name would reach outside the archive, such as
            // one starting with `..`, is left out.
            let Some(name) = entry.enclosed_name() else {
                continue;
            };
            let parts: Vec<String> = name
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            if parts.is_empty() {
                continue;
            }
            for depth in 1..parts.len() {
                directories.insert(parts[..depth].join("/"));
            }
            let key = parts.join("/");
            if entry.is_dir() {
                directories.insert(key);
            } else {
                files.insert(key, (index, entry.size()));
            }
        }
        Ok(Archive {
            zip,
            files,
            directories,
        })
    }

    fn find(&self, rest: &[String]) -> Option<(Location, FileInfo)> {
        if rest.is_empty() {
            return Some((Location::Directory, DIRECTORY));
        }
        let key = rest.join("/");
        if let Some(&(index, size)) = self.files.get(&key) {
            let info = FileInfo {
                kind: FileType::File,
                size,
                modified: None,
            };
            return Some((Location::Archive { index }, info));
        }
        self.directories
            .contains(&key)
            .then_some((Location::Directory, DIRECTORY))
    }

    fn list(&self, rest: &[String], names: &mut BTreeSet<String>) {
        let prefix = if rest.is_empty() {
            String::new()
        } else {
            format!("{}/", rest.join("/"))
        };
        let paths = self.files.keys().chain(&self.directories);
        for inside in paths.filter_map(|path| path.strip_prefix(&prefix)) {
            if let Some(name) = inside.split('/').next().filter(|name| !name.is_empty()) {
                names.insert(name.to_string());
            }
        }
    }
}

const DIRECTORY: FileInfo = FileInfo {
    kind: FileType::Directory,
    size: 0,
    modified: None,
};

/// How a path relates to a mount point.
enum Within<'a> {
    /// The path is a directory on the way to the mount point, and `next`
    /// is the name inside it that leads there.
    Above {
        next: &'a str,
    },
    /// The path is at or below the mount point, by `rest`.
    Inside {
        rest: &'a [String],
    },
    Outside,
}

fn within<'a>(point: &'a [String], parts: &'a [String]) -> Within<'a> {
    if parts.len() < point.len() && point.starts_with(parts) {
        Within::Above {
            next: &point[parts.len()],
        }
    } else if parts.starts_with(point) {
        Within::Inside {
            rest: &parts[point.len()..],
        }
    } else {
        Within::Outside
    }
}

fn disk_path(root: &Path, parts: &[String]) -> PathBuf {
    parts
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn find_on_disk(root: &Path, parts: &[String], follow: bool) -> Option<(Location, FileInfo)> {
    let path = disk_path(root, parts);
    if !follow {
        // A symbolic link anywhere on the way hides what is behind it,
        // and one at the end is reported as a link.
        let mut step = root.to_path_buf();
        for (index, part) in parts.iter().enumerate() {
            step.push(part);
            let metadata = std::fs::symlink_metadata(&step).ok()?;
            if metadata.file_type().is_symlink() {
                return (index + 1 == parts.len()).then(|| {
                    let info = FileInfo {
                        kind: FileType::Symlink,
                        size: 0,
                        modified: metadata.modified().ok(),
                    };
                    (Location::Disk(path.clone()), info)
                });
            }
        }
    }
    let info = info_of(&path)?;
    Some((Location::Disk(path), info))
}

/// Adds the names inside `directory` to `names`.
pub(crate) fn list_directory(directory: &Path, names: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        names.insert(entry.file_name().to_string_lossy().into_owned());
    }
}

impl Mount {
    fn find(&self, parts: &[String], follow: bool) -> Option<(Location, FileInfo)> {
        match within(&self.point, parts) {
            Within::Above { .. } => Some((Location::Directory, DIRECTORY)),
            Within::Inside { rest } => match &self.source {
                Source::Directory(root) => find_on_disk(root, rest, follow),
                Source::Archive(archive) => archive.find(rest),
            },
            Within::Outside => None,
        }
    }

    fn list(&self, parts: &[String], names: &mut BTreeSet<String>) {
        match within(&self.point, parts) {
            Within::Above { next } => {
                names.insert(next.to_string());
            }
            Within::Inside { rest } => match &self.source {
                Source::Directory(root) => list_directory(&disk_path(root, rest), names),
                Source::Archive(archive) => archive.list(rest, names),
            },
            Within::Outside => {}
        }
    }
}

/// Where a path was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Location {
    Disk(PathBuf),
    /// A file inside the archive of the mount the path was found in.
    Archive {
        index: usize,
    },
    /// A directory inside an archive, or one leading to a mount point,
    /// which has no file of its own.
    Directory,
}

/// Which mount, by its position in [`SearchPath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    First(usize),
    Last(usize),
    /// The assets built into the game.
    Embedded,
}

/// One place to look, in search order.
enum Place<'a> {
    Disk(&'a Path),
    Mount(Slot, &'a Mount),
}

/// What a lookup found: where, in which mount if any, and what it is.
pub(crate) struct Found {
    pub location: Location,
    slot: Option<Slot>,
    /// The save or asset directory it was found in, if neither a mount.
    root: Option<PathBuf>,
    pub info: FileInfo,
}

pub(crate) struct SearchPath {
    /// Looked at before the game's own files, the newest first.
    first: Vec<Mount>,
    /// Looked at after the game's own files, the oldest first.
    last: Vec<Mount>,
    /// Whether symbolic links on disk are followed.
    pub follow_symlinks: bool,
    /// Whether the asset directory is looked at before the save directory.
    pub assets_first: bool,
    /// The assets built into the game, looked at just before the asset
    /// directory.
    embedded: Option<Mount>,
}

impl Default for SearchPath {
    fn default() -> Self {
        Self {
            first: Vec::new(),
            last: Vec::new(),
            follow_symlinks: true,
            assets_first: false,
            embedded: None,
        }
    }
}

/// The error for reading through a symbolic link while they are turned
/// off.
pub(crate) fn symlink_refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "that path is a symbolic link, and symbolic links are turned off",
    )
}

impl SearchPath {
    pub fn new(assets_first: bool) -> Self {
        Self {
            assets_first,
            ..Self::default()
        }
    }

    fn places<'a>(&'a self, save: Option<&'a Path>, assets: &'a Path) -> Vec<Place<'a>> {
        let first = self
            .first
            .iter()
            .enumerate()
            .map(|(index, mount)| Place::Mount(Slot::First(index), mount));
        let last = self
            .last
            .iter()
            .enumerate()
            .map(|(index, mount)| Place::Mount(Slot::Last(index), mount));
        let saves = save.map(Place::Disk);
        let shipped = self
            .embedded
            .iter()
            .map(|mount| Place::Mount(Slot::Embedded, mount))
            .chain(std::iter::once(Place::Disk(assets)));
        let mut places: Vec<Place<'a>> = first.collect();
        if self.assets_first {
            places.extend(shipped);
            places.extend(saves);
        } else {
            places.extend(saves);
            places.extend(shipped);
        }
        places.extend(last);
        places
    }

    /// Makes `archive`, the assets built into the game, part of its files.
    /// `file` is what [`SearchPath::real_directory`] reports for them.
    pub fn embed(&mut self, file: PathBuf, archive: Archive) {
        self.embedded = Some(Mount {
            file,
            point: Vec::new(),
            source: Source::Archive(archive),
        });
    }

    fn mount(&self, slot: Slot) -> &Mount {
        match slot {
            Slot::First(index) => &self.first[index],
            Slot::Last(index) => &self.last[index],
            Slot::Embedded => self
                .embedded
                .as_ref()
                .expect("the game has built-in assets"),
        }
    }

    fn mount_mut(&mut self, slot: Slot) -> &mut Mount {
        match slot {
            Slot::First(index) => &mut self.first[index],
            Slot::Last(index) => &mut self.last[index],
            Slot::Embedded => self
                .embedded
                .as_mut()
                .expect("the game has built-in assets"),
        }
    }

    /// The first place that has something at `parts`.
    pub fn find(&self, save: Option<&Path>, assets: &Path, parts: &[String]) -> Option<Found> {
        let follow = self.follow_symlinks;
        self.places(save, assets)
            .into_iter()
            .find_map(|place| match place {
                Place::Disk(root) => {
                    find_on_disk(root, parts, follow).map(|(location, info)| Found {
                        location,
                        slot: None,
                        root: Some(root.to_path_buf()),
                        info,
                    })
                }
                Place::Mount(slot, mount) => {
                    mount.find(parts, follow).map(|(location, info)| Found {
                        location,
                        slot: Some(slot),
                        root: None,
                        info,
                    })
                }
            })
    }

    /// The directory or archive on disk that what `find` returned is in:
    /// the save directory, the asset directory, or a mounted file.
    pub fn real_directory(&self, found: &Found) -> PathBuf {
        match (&found.root, found.slot) {
            (Some(root), _) => root.clone(),
            (None, Some(slot)) => self.mount(slot).file.clone(),
            (None, None) => unreachable!("a lookup is found on disk or in a mount"),
        }
    }

    /// A path on disk for what `find` returned. A file inside an archive
    /// gets a path inside the archive's own, which does not exist.
    pub fn disk_path(&self, found: &Found, parts: &[String]) -> Option<PathBuf> {
        match (&found.location, found.slot) {
            (Location::Disk(path), _) => Some(path.clone()),
            (Location::Archive { .. }, Some(slot)) => {
                let mount = self.mount(slot);
                Some(disk_path(&mount.file, &parts[mount.point.len()..]))
            }
            _ => None,
        }
    }

    /// Reads the file `find` returned.
    pub fn read(&mut self, found: &Found) -> io::Result<Vec<u8>> {
        if found.info.kind == FileType::Symlink {
            return Err(symlink_refused());
        }
        match (&found.location, found.slot) {
            (Location::Disk(path), _) => std::fs::read(path),
            (Location::Archive { index }, Some(slot)) => {
                let Source::Archive(archive) = &mut self.mount_mut(slot).source else {
                    unreachable!("only archives hold archive locations");
                };
                let mut entry = archive.zip.by_index(*index).map_err(io::Error::other)?;
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut bytes)?;
                Ok(bytes)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "that path is a directory",
            )),
        }
    }

    /// The names inside the directory `parts`, from every place.
    pub fn list(&self, save: Option<&Path>, assets: &Path, parts: &[String]) -> Vec<String> {
        let mut names = BTreeSet::new();
        for place in self.places(save, assets) {
            match place {
                Place::Disk(root) => list_directory(&disk_path(root, parts), &mut names),
                Place::Mount(_, mount) => mount.list(parts, &mut names),
            }
        }
        names.into_iter().collect()
    }

    /// Mounts the zip archive or directory `file` at `point`. A file
    /// already mounted is moved to its new place.
    pub fn add(&mut self, file: PathBuf, point: Vec<String>, last: bool) -> io::Result<()> {
        let source = if std::fs::metadata(&file)?.is_dir() {
            Source::Directory(file.clone())
        } else {
            Source::Archive(Archive::open(&file)?)
        };
        self.remove(&file);
        let mount = Mount {
            file,
            point,
            source,
        };
        if last {
            self.last.push(mount);
        } else {
            self.first.insert(0, mount);
        }
        Ok(())
    }

    /// Unmounts `file`, returning whether it was mounted.
    pub fn remove(&mut self, file: &Path) -> bool {
        let before = self.first.len() + self.last.len();
        self.first.retain(|mount| mount.file != file);
        self.last.retain(|mount| mount.file != file);
        self.first.len() + self.last.len() != before
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::filesystem::tests::TempDir;

    fn parts(path: &str) -> Vec<String> {
        path.split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Writes a zip archive holding `files`, compressed.
    fn write_zip(path: &Path, files: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, contents) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn built_in_assets_come_before_the_asset_directory() {
        let dir = TempDir::new("embedded");
        let assets = dir.0.join("assets");
        let save = dir.0.join("save");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&save).unwrap();
        std::fs::write(assets.join("level.txt"), "loose").unwrap();
        std::fs::write(assets.join("extra.txt"), "only loose").unwrap();
        let zip_path = dir.0.join("built-in.zip");
        write_zip(
            &zip_path,
            &[("level.txt", "built in"), ("music/theme.txt", "theme")],
        );
        let bytes: &'static [u8] = std::fs::read(&zip_path).unwrap().leak();

        let mut search = SearchPath::default();
        search.embed(
            PathBuf::from("game.exe"),
            Archive::from_bytes(bytes).unwrap(),
        );
        assert_eq!(
            read(&mut search, &assets, "level.txt").as_deref(),
            Some("built in")
        );
        assert_eq!(
            read(&mut search, &assets, "music/theme.txt").as_deref(),
            Some("theme")
        );
        assert_eq!(
            read(&mut search, &assets, "extra.txt").as_deref(),
            Some("only loose")
        );
        assert_eq!(
            search.list(None, &assets, &parts("")),
            ["extra.txt", "level.txt", "music"]
        );
        let found = search.find(None, &assets, &parts("level.txt")).unwrap();
        assert_eq!(search.real_directory(&found), PathBuf::from("game.exe"));

        // A save still overrides a built-in file, unless shipped files
        // come first.
        std::fs::write(save.join("level.txt"), "saved").unwrap();
        let found = search
            .find(Some(&save), &assets, &parts("level.txt"))
            .unwrap();
        assert_eq!(
            String::from_utf8(search.read(&found).unwrap()).unwrap(),
            "saved"
        );
        search.assets_first = true;
        let found = search
            .find(Some(&save), &assets, &parts("level.txt"))
            .unwrap();
        assert_eq!(
            String::from_utf8(search.read(&found).unwrap()).unwrap(),
            "built in"
        );

        assert!(Archive::from_bytes(b"not a zip").is_err());
    }

    fn read(search: &mut SearchPath, assets: &Path, path: &str) -> Option<String> {
        let found = search.find(None, assets, &parts(path))?;
        Some(String::from_utf8(search.read(&found).unwrap()).unwrap())
    }

    #[test]
    fn files_in_a_mounted_zip_can_be_read_and_listed() {
        let dir = TempDir::new("zip-read");
        let assets = dir.0.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let archive = dir.0.join("dlc.zip");
        write_zip(
            &archive,
            &[("levels/one.txt", "first"), ("levels/two.txt", "second")],
        );

        let mut search = SearchPath::default();
        search.add(archive, Vec::new(), false).unwrap();

        assert_eq!(
            read(&mut search, &assets, "levels/two.txt").as_deref(),
            Some("second")
        );
        let found = search.find(None, &assets, &parts("levels")).unwrap();
        assert_eq!(found.info.kind, FileType::Directory);
        let found = search
            .find(None, &assets, &parts("levels/one.txt"))
            .unwrap();
        assert_eq!((found.info.kind, found.info.size), (FileType::File, 5));
        assert_eq!(search.list(None, &assets, &parts("")), ["levels"]);
        assert_eq!(
            search.list(None, &assets, &parts("levels")),
            ["one.txt", "two.txt"]
        );
        assert!(
            search
                .find(None, &assets, &parts("levels/three.txt"))
                .is_none()
        );
    }

    #[test]
    fn a_mount_point_puts_the_contents_in_a_directory() {
        let dir = TempDir::new("zip-point");
        let assets = dir.0.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let archive = dir.0.join("music.zip");
        write_zip(&archive, &[("theme.txt", "la la")]);

        let mut search = SearchPath::default();
        search.add(archive, parts("packs/music"), false).unwrap();

        assert_eq!(
            read(&mut search, &assets, "packs/music/theme.txt").as_deref(),
            Some("la la")
        );
        assert!(search.find(None, &assets, &parts("theme.txt")).is_none());
        assert_eq!(search.list(None, &assets, &parts("")), ["packs"]);
        assert_eq!(search.list(None, &assets, &parts("packs")), ["music"]);
        let found = search.find(None, &assets, &parts("packs")).unwrap();
        assert_eq!(found.info.kind, FileType::Directory);
    }

    #[test]
    fn appending_the_identity_puts_shipped_files_first() {
        let dir = TempDir::new("append-identity");
        let assets = dir.0.join("assets");
        let save = dir.0.join("save");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(&save).unwrap();
        std::fs::write(assets.join("level.txt"), "shipped").unwrap();
        std::fs::write(save.join("level.txt"), "saved").unwrap();
        std::fs::write(save.join("progress.txt"), "3").unwrap();

        let mut search = SearchPath::default();
        let read_from = |search: &mut SearchPath, path: &str| {
            let found = search.find(Some(&save), &assets, &parts(path)).unwrap();
            String::from_utf8(search.read(&found).unwrap()).unwrap()
        };
        assert_eq!(read_from(&mut search, "level.txt"), "saved");
        search.assets_first = true;
        assert_eq!(read_from(&mut search, "level.txt"), "shipped");
        // A file only in the save directory is still found.
        assert_eq!(read_from(&mut search, "progress.txt"), "3");
    }

    #[test]
    fn mounts_go_before_or_after_the_games_files() {
        let dir = TempDir::new("zip-order");
        let assets = dir.0.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("level.txt"), "shipped").unwrap();
        let patch = dir.0.join("patch.zip");
        write_zip(&patch, &[("level.txt", "patched")]);
        let fallback = dir.0.join("fallback.zip");
        write_zip(
            &fallback,
            &[("level.txt", "fallback"), ("extra.txt", "extra")],
        );

        let mut search = SearchPath::default();
        search.add(fallback, Vec::new(), true).unwrap();
        assert_eq!(
            read(&mut search, &assets, "level.txt").as_deref(),
            Some("shipped")
        );
        assert_eq!(
            read(&mut search, &assets, "extra.txt").as_deref(),
            Some("extra")
        );

        search.add(patch.clone(), Vec::new(), false).unwrap();
        assert_eq!(
            read(&mut search, &assets, "level.txt").as_deref(),
            Some("patched")
        );

        assert!(search.remove(&patch));
        assert!(!search.remove(&patch));
        assert_eq!(
            read(&mut search, &assets, "level.txt").as_deref(),
            Some("shipped")
        );
    }

    #[test]
    fn a_directory_can_be_mounted_too() {
        let dir = TempDir::new("dir-mount");
        let assets = dir.0.join("assets");
        let mods = dir.0.join("mods");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::create_dir_all(mods.join("sprites")).unwrap();
        std::fs::write(mods.join("sprites/hero.txt"), "hero").unwrap();

        let mut search = SearchPath::default();
        search.add(mods, parts("mods"), false).unwrap();

        assert_eq!(
            read(&mut search, &assets, "mods/sprites/hero.txt").as_deref(),
            Some("hero")
        );
        assert_eq!(search.list(None, &assets, &parts("mods")), ["sprites"]);
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_refused() {
        let dir = TempDir::new("not-zip");
        let file = dir.0.join("notes.zip");
        std::fs::write(&file, "plain text").unwrap();

        let mut search = SearchPath::default();
        let error = search.add(file, Vec::new(), false).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_file_in_an_archive_reports_a_path_that_does_not_exist() {
        let dir = TempDir::new("zip-path");
        let assets = dir.0.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let archive = dir.0.join("dlc.zip");
        write_zip(&archive, &[("a/b.txt", "b")]);

        let mut search = SearchPath::default();
        search.add(archive.clone(), parts("dlc"), false).unwrap();

        let wanted = parts("dlc/a/b.txt");
        let found = search.find(None, &assets, &wanted).unwrap();
        let path = search.disk_path(&found, &wanted).unwrap();
        assert_eq!(path, archive.join("a").join("b.txt"));
        assert!(!path.exists());
    }
}
