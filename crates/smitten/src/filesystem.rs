//! Game files, the equivalent of `love.filesystem`.
//!
//! A game reads from two places and writes to one:
//!
//! - The **asset directory** holds the files shipped with the game. While
//!   developing with `cargo run` it is the `assets` folder next to the
//!   game's `Cargo.toml`; in a released game it is the `assets` folder next
//!   to the executable.
//! - The **save directory** is a per-user folder the game may write to,
//!   named after the game's identity.
//!
//! Relative paths are looked up in the save directory first and then in
//! the asset directory, as in LÖVE, so a saved file can override a shipped
//! one. Writing always goes to the save directory. Image and font loading
//! use the same lookup.

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::Config;
use crate::context::{self, Context};

pub(crate) struct FilesystemState {
    asset_directory: PathBuf,
    identity: String,
    /// Looked up from the backend the first time it is needed.
    save_directory: Option<PathBuf>,
}

impl FilesystemState {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            asset_directory: default_asset_directory(),
            identity: config.identity.clone().unwrap_or_else(default_identity),
            save_directory: None,
        }
    }
}

/// The `assets` folder next to the game's `Cargo.toml` under `cargo run`,
/// which sets `CARGO_MANIFEST_DIR`, and next to the executable otherwise.
fn default_asset_directory() -> PathBuf {
    let base = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let executable = std::env::current_exe().ok()?;
            Some(executable.parent()?.to_path_buf())
        })
        .unwrap_or_default();
    base.join("assets")
}

/// The executable's name without its extension.
fn default_identity() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "smitten-game".to_string())
}

/// Checks that `path` cannot leave the directory it is joined to: it must
/// be relative and must not contain `..`.
fn confined(path: &Path) -> io::Result<&Path> {
    let stays_inside = path
        .components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if stays_inside
        && path
            .components()
            .any(|part| matches!(part, Component::Normal(_)))
    {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is not a relative path inside the save directory",
                path.display()
            ),
        ))
    }
}

/// Picks the file a read of `path` refers to. Absolute paths are used as
/// they are; relative ones prefer the save directory over the assets.
fn locate(asset_directory: &Path, save_directory: Option<&Path>, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if let Some(saved) = save_directory.map(|directory| directory.join(path))
        && saved.exists()
    {
        return saved;
    }
    asset_directory.join(path)
}

fn save_directory_in(ctx: &mut Context) -> io::Result<PathBuf> {
    if let Some(directory) = &ctx.fs.save_directory {
        return Ok(directory.clone());
    }
    let directory = ctx
        .backend
        .save_directory(&ctx.fs.identity)
        .map_err(io::Error::other)?;
    ctx.fs.save_directory = Some(directory.clone());
    Ok(directory)
}

/// The file a read of `path` refers to.
pub(crate) fn resolve(path: &Path) -> PathBuf {
    context::with(|ctx| {
        // A relative path that could escape the save directory is only
        // looked up among the assets.
        let save_directory = confined(path)
            .ok()
            .and_then(|_| save_directory_in(ctx).ok());
        locate(&ctx.fs.asset_directory, save_directory.as_deref(), path)
    })
}

/// Where `path` would be written, inside the save directory.
fn save_path(path: &Path) -> io::Result<PathBuf> {
    let path = confined(path)?;
    context::with(|ctx| Ok(save_directory_in(ctx)?.join(path)))
}

/// The directory that holds the files shipped with the game.
pub fn asset_directory() -> PathBuf {
    context::with(|ctx| ctx.fs.asset_directory.clone())
}

/// Changes where shipped files are looked up.
pub fn set_asset_directory(path: impl Into<PathBuf>) {
    let path = path.into();
    context::with(|ctx| ctx.fs.asset_directory = path);
}

/// The name that selects the game's save directory.
pub fn identity() -> String {
    context::with(|ctx| ctx.fs.identity.clone())
}

/// Changes the game's identity, and with it the save directory.
pub fn set_identity(identity: &str) {
    context::with(|ctx| {
        ctx.fs.identity = identity.to_string();
        ctx.fs.save_directory = None;
    });
}

/// The per-user directory the game may write to, created if needed.
pub fn save_directory() -> io::Result<PathBuf> {
    context::with(save_directory_in)
}

/// Reads a whole file.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    std::fs::read(resolve(path.as_ref()))
}

/// Reads a whole file as UTF-8 text.
pub fn read_string(path: impl AsRef<Path>) -> io::Result<String> {
    std::fs::read_to_string(resolve(path.as_ref()))
}

/// Returns whether a file or directory exists at `path`.
pub fn exists(path: impl AsRef<Path>) -> bool {
    resolve(path.as_ref()).exists()
}

/// What kind of thing a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    File,
    Directory,
    /// Something else, such as a device.
    Other,
}

/// What is known about a file or directory, as `love.filesystem.getInfo`
/// reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileInfo {
    pub kind: FileType,
    /// The size in bytes. Zero for a directory.
    pub size: u64,
    /// When it was last changed, where the system records that.
    pub modified: Option<SystemTime>,
}

fn info_of(file: &Path) -> Option<FileInfo> {
    let metadata = std::fs::metadata(file).ok()?;
    let kind = if metadata.is_file() {
        FileType::File
    } else if metadata.is_dir() {
        FileType::Directory
    } else {
        FileType::Other
    };
    Some(FileInfo {
        kind,
        size: if kind == FileType::File {
            metadata.len()
        } else {
            0
        },
        modified: metadata.modified().ok(),
    })
}

/// Returns what is at `path`, or `None` if nothing is.
pub fn info(path: impl AsRef<Path>) -> Option<FileInfo> {
    info_of(&resolve(path.as_ref()))
}

/// Returns whether `path` names a file.
pub fn is_file(path: impl AsRef<Path>) -> bool {
    info(path).is_some_and(|info| info.kind == FileType::File)
}

/// Returns whether `path` names a directory.
pub fn is_directory(path: impl AsRef<Path>) -> bool {
    info(path).is_some_and(|info| info.kind == FileType::Directory)
}

/// The names in `path` under each of `roots`, merged and sorted. A root
/// without that directory adds nothing.
fn list_in<'a>(roots: impl IntoIterator<Item = &'a Path>, path: &Path) -> Vec<String> {
    let mut names = BTreeSet::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root.join(path)) else {
            continue;
        };
        for entry in entries.flatten() {
            names.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.into_iter().collect()
}

/// Lists the files and directories inside `path`, by name and sorted.
///
/// As in LÖVE, the save directory and the asset directory are looked at
/// together, so the list has what the game shipped with and what it has
/// saved. An empty path lists the top level. A directory that does not
/// exist gives an empty list.
pub fn directory_items(path: impl AsRef<Path>) -> Vec<String> {
    let path = path.as_ref();
    if path.is_absolute() {
        return list_in([Path::new("")], path);
    }
    // The top level is spelled as an empty path or a dot.
    let top_level = path
        .components()
        .all(|part| matches!(part, Component::CurDir));
    if !top_level && confined(path).is_err() {
        return Vec::new();
    }
    context::with(|ctx| {
        let save_directory = save_directory_in(ctx).ok();
        let roots = save_directory
            .as_deref()
            .into_iter()
            .chain([ctx.fs.asset_directory.as_path()]);
        list_in(roots, path)
    })
}

/// Reads a text file as a list of its lines, without the line endings.
pub fn lines(path: impl AsRef<Path>) -> io::Result<Vec<String>> {
    Ok(read_string(path)?.lines().map(str::to_string).collect())
}

/// Writes a file in the save directory, replacing it if it exists.
/// Missing parent directories are created.
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let target = save_path(path.as_ref())?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, contents)
}

/// Adds to the end of a file in the save directory, creating it if needed.
pub fn append(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let target = save_path(path.as_ref())?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(target)?
        .write_all(contents.as_ref())
}

/// Creates a directory in the save directory, along with any missing
/// parents.
pub fn create_directory(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::create_dir_all(save_path(path.as_ref())?)
}

/// Deletes a file or an empty directory from the save directory.
pub fn remove(path: impl AsRef<Path>) -> io::Result<()> {
    let target = save_path(path.as_ref())?;
    if target.is_dir() {
        std::fs::remove_dir(target)
    } else {
        std::fs::remove_file(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp folder, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("smitten-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp directory can be created");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn plain_relative_paths_are_confined() {
        assert!(confined(Path::new("save.txt")).is_ok());
        assert!(confined(Path::new("slots/1/save.txt")).is_ok());
        assert!(confined(Path::new("./save.txt")).is_ok());
    }

    #[test]
    fn paths_that_could_escape_are_rejected() {
        assert!(confined(Path::new("../save.txt")).is_err());
        assert!(confined(Path::new("slots/../../save.txt")).is_err());
        assert!(confined(&std::env::temp_dir().join("save.txt")).is_err());
        assert!(confined(Path::new("")).is_err());
        assert!(confined(Path::new(".")).is_err());
    }

    #[test]
    fn a_file_only_in_assets_is_read_from_assets() {
        let assets = TempDir::new("assets-only");
        let saves = TempDir::new("assets-only-saves");
        std::fs::write(assets.0.join("level.txt"), "shipped").unwrap();

        let found = locate(&assets.0, Some(&saves.0), Path::new("level.txt"));
        assert_eq!(found, assets.0.join("level.txt"));
    }

    #[test]
    fn a_saved_file_overrides_the_shipped_one() {
        let assets = TempDir::new("override");
        let saves = TempDir::new("override-saves");
        std::fs::write(assets.0.join("level.txt"), "shipped").unwrap();
        std::fs::write(saves.0.join("level.txt"), "edited").unwrap();

        let found = locate(&assets.0, Some(&saves.0), Path::new("level.txt"));
        assert_eq!(std::fs::read_to_string(found).unwrap(), "edited");
    }

    #[test]
    fn a_missing_file_points_into_assets() {
        let assets = TempDir::new("missing");
        let found = locate(&assets.0, None, Path::new("nope.png"));
        assert_eq!(found, assets.0.join("nope.png"));
    }

    #[test]
    fn a_listing_merges_saved_and_shipped_names_in_order() {
        let assets = TempDir::new("list-assets");
        let saves = TempDir::new("list-saves");
        std::fs::create_dir_all(assets.0.join("levels")).unwrap();
        std::fs::create_dir_all(saves.0.join("levels")).unwrap();
        std::fs::write(assets.0.join("levels/b.txt"), "").unwrap();
        std::fs::write(assets.0.join("levels/a.txt"), "").unwrap();
        std::fs::write(saves.0.join("levels/a.txt"), "").unwrap();
        std::fs::write(saves.0.join("levels/custom.txt"), "").unwrap();

        let roots = [saves.0.as_path(), assets.0.as_path()];
        assert_eq!(
            list_in(roots, Path::new("levels")),
            ["a.txt", "b.txt", "custom.txt"]
        );
        assert_eq!(list_in(roots, Path::new("")), ["levels"]);
        assert!(list_in(roots, Path::new("nowhere")).is_empty());
    }

    #[test]
    fn info_tells_files_from_directories() {
        let directory = TempDir::new("info");
        std::fs::write(directory.0.join("save.txt"), "12345").unwrap();

        let file = info_of(&directory.0.join("save.txt")).unwrap();
        assert_eq!((file.kind, file.size), (FileType::File, 5));
        assert!(file.modified.is_some());

        let folder = info_of(&directory.0).unwrap();
        assert_eq!((folder.kind, folder.size), (FileType::Directory, 0));

        assert!(info_of(&directory.0.join("missing.txt")).is_none());
    }

    #[test]
    fn absolute_paths_are_used_as_given() {
        let assets = TempDir::new("absolute");
        let elsewhere = std::env::temp_dir().join("somewhere-else.png");
        assert_eq!(locate(&assets.0, None, &elsewhere), elsewhere);
    }
}
