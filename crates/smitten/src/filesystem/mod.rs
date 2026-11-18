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
//! one. Writing always goes to the save directory. Image, font, shader and
//! sound loading use the same lookup.
//!
//! A game can also [`mount`] a zip archive or a directory, such as a
//! downloaded level pack, so that the files inside it can be read like
//! any others. Mounted files cannot be written.
//!
//! For a file too large to read at once, or one written over time, use
//! [`open_file`] to read or write it a piece at a time.

mod file;
mod file_data;
mod search;

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::Config;
use crate::context::{self, Context};

pub use file::{BufferMode, File, FileMode, open_file};
pub use file_data::{FileData, new_file_data};
use search::SearchPath;

pub(crate) struct FilesystemState {
    asset_directory: PathBuf,
    identity: String,
    /// Looked up from the backend the first time it is needed.
    save_directory: Option<PathBuf>,
    mounts: SearchPath,
}

impl FilesystemState {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            asset_directory: default_asset_directory(),
            identity: config.identity.clone().unwrap_or_else(default_identity),
            save_directory: None,
            mounts: SearchPath::default(),
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

/// The names in a relative path, or `None` if it could leave the
/// directory it is looked up in. The top level, spelled as an empty path
/// or a dot, has no names.
fn names(path: &Path) -> Option<Vec<String>> {
    path.components()
        .filter(|part| !matches!(part, Component::CurDir))
        .map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// How a path given to a read is looked up.
enum Lookup {
    /// An absolute path, or one that could escape: used on disk as it is,
    /// the second joined to the asset directory.
    Disk(PathBuf),
    /// A relative path, looked up along the search path.
    Search(Vec<String>),
}

fn lookup(ctx: &Context, path: &Path) -> Lookup {
    if path.is_absolute() {
        Lookup::Disk(path.to_path_buf())
    } else if let Some(names) = names(path) {
        Lookup::Search(names)
    } else {
        Lookup::Disk(ctx.fs.asset_directory.join(path))
    }
}

/// Runs `f` with the search path, the save directory if there is one,
/// and the asset directory.
fn with_search<R>(f: impl FnOnce(&mut SearchPath, Option<&Path>, &Path) -> R) -> R {
    context::with(|ctx| {
        let save_directory = save_directory_in(ctx).ok();
        let fs = &mut ctx.fs;
        f(
            &mut fs.mounts,
            save_directory.as_deref(),
            &fs.asset_directory,
        )
    })
}

/// The file on disk a read of `path` refers to, for watching it. A file
/// inside a mounted archive gets a path that does not exist, and a file
/// that is nowhere gets one in the asset directory.
pub(crate) fn resolve(path: &Path) -> PathBuf {
    let names = match context::with(|ctx| lookup(ctx, path)) {
        Lookup::Disk(file) => return file,
        Lookup::Search(names) => names,
    };
    with_search(|search, save, assets| {
        search
            .find(save, assets, &names)
            .and_then(|found| search.disk_path(&found, &names))
            .unwrap_or_else(|| assets.join(path))
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
    let path = path.as_ref();
    let names = match context::with(|ctx| lookup(ctx, path)) {
        Lookup::Disk(file) => return std::fs::read(file),
        Lookup::Search(names) => names,
    };
    with_search(
        |search, save, assets| match search.find(save, assets, &names) {
            Some(found) => search.read(&found),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} was not found", path.display()),
            )),
        },
    )
}

/// Reads a whole file as UTF-8 text.
pub fn read_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Returns whether a file or directory exists at `path`.
pub fn exists(path: impl AsRef<Path>) -> bool {
    info(path).is_some()
}

/// What kind of thing a path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    File,
    Directory,
    /// A symbolic link. Only reported while links are turned off with
    /// [`set_symlinks_enabled`]; otherwise a link is reported as what it
    /// points to.
    Symlink,
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

/// Returns what is at `path`, or `None` if nothing is. A file inside a
/// mounted zip archive has no modification time.
pub fn info(path: impl AsRef<Path>) -> Option<FileInfo> {
    let names = match context::with(|ctx| lookup(ctx, path.as_ref())) {
        Lookup::Disk(file) => return info_of(&file),
        Lookup::Search(names) => names,
    };
    with_search(|search, save, assets| Some(search.find(save, assets, &names)?.info))
}

/// Returns whether `path` names a file.
pub fn is_file(path: impl AsRef<Path>) -> bool {
    info(path).is_some_and(|info| info.kind == FileType::File)
}

/// Returns whether `path` names a directory.
pub fn is_directory(path: impl AsRef<Path>) -> bool {
    info(path).is_some_and(|info| info.kind == FileType::Directory)
}

/// Lists the files and directories inside `path`, by name and sorted.
///
/// As in LÖVE, the save directory, the asset directory and anything
/// mounted are looked at together, so the list has what the game shipped
/// with and what it has saved. An empty path lists the top level. A
/// directory that does not exist gives an empty list.
pub fn directory_items(path: impl AsRef<Path>) -> Vec<String> {
    let path = path.as_ref();
    if path.is_absolute() {
        let mut names = BTreeSet::new();
        search::list_directory(path, &mut names);
        return names.into_iter().collect();
    }
    let Some(names) = names(path) else {
        return Vec::new();
    };
    with_search(|search, save, assets| search.list(save, assets, &names))
}

/// Makes the files in a zip archive, or a directory, readable as if they
/// were the game's own, the equivalent of `love.filesystem.mount`.
///
/// `archive` is looked up in the save directory and then the asset
/// directory, or used as it is if absolute. Its contents appear under
/// `mount_point`, or at the top level if that is empty. They are looked
/// at before the game's other files, so an archive can replace a file the
/// game shipped with; [`mount_after`] puts them after instead. Mounting
/// an archive that is already mounted moves it.
///
/// ```no_run
/// use smitten::filesystem;
///
/// filesystem::mount("level-pack.zip", "levels").expect("the pack is there");
/// let first = filesystem::read_string("levels/1.txt").expect("it has a first level");
/// ```
pub fn mount(archive: impl AsRef<Path>, mount_point: impl AsRef<Path>) -> io::Result<()> {
    add_mount(archive.as_ref(), mount_point.as_ref(), false)
}

/// Like [`mount`], but the archive is looked at after the game's other
/// files, so it only adds files and never replaces one.
pub fn mount_after(archive: impl AsRef<Path>, mount_point: impl AsRef<Path>) -> io::Result<()> {
    add_mount(archive.as_ref(), mount_point.as_ref(), true)
}

/// The archive or directory on disk that `archive` names.
fn mounted_file(archive: &Path) -> io::Result<PathBuf> {
    if archive.is_absolute() {
        return Ok(archive.to_path_buf());
    }
    confined(archive)?;
    context::with(|ctx| {
        if let Ok(saves) = save_directory_in(ctx)
            && saves.join(archive).exists()
        {
            return Ok(saves.join(archive));
        }
        Ok(ctx.fs.asset_directory.join(archive))
    })
}

fn add_mount(archive: &Path, mount_point: &Path, last: bool) -> io::Result<()> {
    let point = names(mount_point).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a relative mount point", mount_point.display()),
        )
    })?;
    let file = mounted_file(archive)?;
    context::with(|ctx| ctx.fs.mounts.add(file, point, last))
}

/// Stops reading from an archive given to [`mount`], the equivalent of
/// `love.filesystem.unmount`. Returns whether it was mounted.
pub fn unmount(archive: impl AsRef<Path>) -> bool {
    let Ok(file) = mounted_file(archive.as_ref()) else {
        return false;
    };
    context::with(|ctx| ctx.fs.mounts.remove(&file))
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

/// Where the system keeps programs' per-user data, the equivalent of
/// `love.filesystem.getAppdataDirectory`: `%APPDATA%` on Windows,
/// `~/Library/Application Support` on macOS, and `$XDG_DATA_HOME` or
/// `~/.local/share` elsewhere. The save directory is inside it.
///
/// `None` if the system does not say.
pub fn appdata_directory() -> Option<PathBuf> {
    appdata_directory_from(
        |name| std::env::var_os(name),
        cfg!(windows),
        cfg!(target_os = "macos"),
    )
}

fn appdata_directory_from(
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
    windows: bool,
    macos: bool,
) -> Option<PathBuf> {
    let set = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if windows {
        set("APPDATA")
    } else if macos {
        Some(set("HOME")?.join("Library/Application Support"))
    } else {
        set("XDG_DATA_HOME").or_else(|| Some(set("HOME")?.join(".local/share")))
    }
}

/// The user's home directory, the equivalent of
/// `love.filesystem.getUserDirectory`. `None` if the system does not say.
pub fn user_directory() -> Option<PathBuf> {
    std::env::home_dir().filter(|path| !path.as_os_str().is_empty())
}

/// The directory the game was started from, the equivalent of
/// `love.filesystem.getWorkingDirectory`. Relative paths given to
/// `std::fs` are taken from here; smitten's own functions do not use it.
pub fn working_directory() -> io::Result<PathBuf> {
    std::env::current_dir()
}

/// Where the game itself is, the equivalent of
/// `love.filesystem.getSource`: under `cargo run`, the folder holding
/// the game's `Cargo.toml`; otherwise the executable.
pub fn source() -> PathBuf {
    match std::env::var_os("CARGO_MANIFEST_DIR") {
        Some(folder) => PathBuf::from(folder),
        None => std::env::current_exe().unwrap_or_default(),
    }
}

/// The directory that holds [`source`], the equivalent of
/// `love.filesystem.getSourceBaseDirectory`. For a released game this is
/// the folder the executable is in, which is the place to look for files
/// the player puts next to it.
pub fn source_base_directory() -> PathBuf {
    source().parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Whether the game is running as a released executable rather than
/// under `cargo run`, the equivalent of `love.filesystem.isFused`.
///
/// A smitten game is always one executable, so this tells the two ways
/// of running it apart instead: it is `false` when `cargo run` started
/// the game, and `true` otherwise. The save directory is the same either
/// way.
pub fn is_fused() -> bool {
    std::env::var_os("CARGO_MANIFEST_DIR").is_none()
}

/// The directory or archive a read of `path` would come from, the
/// equivalent of `love.filesystem.getRealDirectory`: the save directory,
/// the asset directory, or a file given to [`mount`].
///
/// `None` if nothing is at `path`, or if `path` is absolute or could
/// leave the game's files, since such a path is not looked up.
pub fn real_directory(path: impl AsRef<Path>) -> Option<PathBuf> {
    let names = match context::with(|ctx| lookup(ctx, path.as_ref())) {
        Lookup::Disk(_) => return None,
        Lookup::Search(names) => names,
    };
    with_search(|search, save, assets| {
        let found = search.find(save, assets, &names)?;
        Some(search.real_directory(&found))
    })
}

/// Turns following symbolic links on or off, the equivalent of
/// `love.filesystem.setSymlinksEnabled`. They are on to begin with.
///
/// While they are off, a link in the save directory, the asset directory
/// or a mounted directory is reported by [`info`] as
/// [`FileType::Symlink`] and cannot be read, and nothing behind a linked
/// directory is found. This keeps a game from reading outside its files
/// through a link someone has put there. Absolute paths, and writing,
/// are not affected.
pub fn set_symlinks_enabled(enabled: bool) {
    context::with(|ctx| ctx.fs.mounts.follow_symlinks = enabled);
}

/// Whether symbolic links are followed, the equivalent of
/// `love.filesystem.areSymlinksEnabled`. See [`set_symlinks_enabled`].
pub fn are_symlinks_enabled() -> bool {
    context::with(|ctx| ctx.fs.mounts.follow_symlinks)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp folder, removed on drop.
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new(name: &str) -> Self {
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

    fn find(assets: &Path, saves: &Path, path: &str) -> Option<search::Found> {
        let names = names(Path::new(path)).expect("a relative path");
        SearchPath::default().find(Some(saves), assets, &names)
    }

    #[test]
    fn a_file_only_in_assets_is_read_from_assets() {
        let assets = TempDir::new("assets-only");
        let saves = TempDir::new("assets-only-saves");
        std::fs::write(assets.0.join("level.txt"), "shipped").unwrap();

        let found = find(&assets.0, &saves.0, "level.txt").unwrap();
        assert_eq!(
            found.location,
            search::Location::Disk(assets.0.join("level.txt"))
        );
    }

    #[test]
    fn a_saved_file_overrides_the_shipped_one() {
        let assets = TempDir::new("override");
        let saves = TempDir::new("override-saves");
        std::fs::write(assets.0.join("level.txt"), "shipped").unwrap();
        std::fs::write(saves.0.join("level.txt"), "edited").unwrap();

        let found = find(&assets.0, &saves.0, "./level.txt").unwrap();
        let bytes = SearchPath::default().read(&found).unwrap();
        assert_eq!(bytes, b"edited");
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let assets = TempDir::new("missing");
        let saves = TempDir::new("missing-saves");
        assert!(find(&assets.0, &saves.0, "nope.png").is_none());
    }

    #[test]
    fn names_reject_paths_that_could_escape() {
        assert_eq!(names(Path::new("a/./b")).unwrap(), ["a", "b"]);
        assert!(names(Path::new("")).unwrap().is_empty());
        assert!(names(Path::new(".")).unwrap().is_empty());
        assert!(names(Path::new("a/../b")).is_none());
        assert!(names(&std::env::temp_dir()).is_none());
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

        let search = SearchPath::default();
        let list =
            |path: &str| search.list(Some(&saves.0), &assets.0, &names(Path::new(path)).unwrap());
        assert_eq!(list("levels"), ["a.txt", "b.txt", "custom.txt"]);
        assert_eq!(list(""), ["levels"]);
        assert!(list("nowhere").is_empty());
    }

    #[test]
    fn the_appdata_directory_follows_each_system() {
        let vars = |name: &str| match name {
            "APPDATA" => Some(r"C:\Users\ada\AppData\Roaming".into()),
            "HOME" => Some("/home/ada".into()),
            _ => None,
        };
        assert_eq!(
            appdata_directory_from(vars, true, false).unwrap(),
            PathBuf::from(r"C:\Users\ada\AppData\Roaming")
        );
        assert_eq!(
            appdata_directory_from(vars, false, true).unwrap(),
            PathBuf::from("/home/ada/Library/Application Support")
        );
        assert_eq!(
            appdata_directory_from(vars, false, false).unwrap(),
            PathBuf::from("/home/ada/.local/share")
        );
        let xdg = |name: &str| (name == "XDG_DATA_HOME").then(|| "/data".into());
        assert_eq!(
            appdata_directory_from(xdg, false, false).unwrap(),
            PathBuf::from("/data")
        );
        assert!(appdata_directory_from(|_| None, false, false).is_none());
    }

    #[test]
    fn the_source_holds_the_game() {
        // Tests run under cargo, which sets CARGO_MANIFEST_DIR.
        assert!(!is_fused());
        assert!(source().join("Cargo.toml").is_file());
        assert_eq!(source_base_directory(), source().parent().unwrap());
        assert!(working_directory().unwrap().is_absolute());
    }

    #[test]
    fn the_real_directory_is_where_a_file_was_found() {
        let assets = TempDir::new("real-assets");
        let saves = TempDir::new("real-saves");
        std::fs::write(assets.0.join("level.txt"), "").unwrap();
        std::fs::write(saves.0.join("save.txt"), "").unwrap();

        let search = SearchPath::default();
        let real = |path: &str| {
            let names = names(Path::new(path)).unwrap();
            let found = search.find(Some(&saves.0), &assets.0, &names)?;
            Some(search.real_directory(&found))
        };
        assert_eq!(real("level.txt").unwrap(), assets.0);
        assert_eq!(real("save.txt").unwrap(), saves.0);
        assert!(real("missing.txt").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_can_be_turned_off() {
        let assets = TempDir::new("links-assets");
        let outside = TempDir::new("links-outside");
        std::fs::write(outside.0.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.0.join("secret.txt"), assets.0.join("link.txt"))
            .unwrap();
        std::os::unix::fs::symlink(&outside.0, assets.0.join("linked")).unwrap();

        let mut search = SearchPath::default();
        let file = names(Path::new("link.txt")).unwrap();
        let behind = names(Path::new("linked/secret.txt")).unwrap();
        let linked = names(Path::new("linked")).unwrap();

        let found = search.find(None, &assets.0, &file).unwrap();
        assert_eq!(found.info.kind, FileType::File);
        assert_eq!(search.read(&found).unwrap(), b"secret");
        assert!(search.find(None, &assets.0, &behind).is_some());

        search.follow_symlinks = false;
        let found = search.find(None, &assets.0, &file).unwrap();
        assert_eq!(found.info.kind, FileType::Symlink);
        assert_eq!(
            search.read(&found).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(search.find(None, &assets.0, &behind).is_none());
        let found = search.find(None, &assets.0, &linked).unwrap();
        assert_eq!(found.info.kind, FileType::Symlink);
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
}
