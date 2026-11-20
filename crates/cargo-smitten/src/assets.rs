//! Which of a game's asset files and extras go into a package, and
//! putting them there: copied beside the executable, or zipped for the
//! executable to carry inside it.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// Files no game means to ship, left out unless an `include` pattern
/// names them: hidden files such as `.DS_Store` and `.gitkeep`, the files
/// Windows Explorer leaves behind, and editor backups.
pub const DEFAULT_EXCLUDES: &[&str] = &[".*", "Thumbs.db", "desktop.ini", "*~", "*.swp"];

/// What to ship from the asset folder, from `[package.metadata.smitten]`
/// and the command line.
#[derive(Debug, PartialEq)]
pub struct Settings {
    /// The folder to ship from.
    pub folder: PathBuf,
    /// Whether the folder was named rather than the default, so that its
    /// being missing is a mistake.
    pub named: bool,
    /// Patterns for the files to ship, or `None` to ship everything. An
    /// empty list ships nothing.
    pub include: Option<Vec<String>>,
    /// Patterns for files to leave out, after `include`.
    pub exclude: Vec<String>,
    /// Whether to build the files into the executable.
    pub embed: bool,
}

/// The files chosen to ship, relative to the folder they are in.
#[derive(Debug, Default, PartialEq)]
pub struct Selection {
    pub folder: PathBuf,
    pub files: Vec<PathBuf>,
    /// How many files in the folder were left out.
    pub left_out: usize,
    /// `include` and `exclude` patterns that matched no file, which is
    /// usually a typo.
    pub unused_patterns: Vec<String>,
}

impl Selection {
    /// What the package summary says about the patterns, if anything.
    pub fn notes(&self) -> String {
        let mut notes = String::new();
        for pattern in &self.unused_patterns {
            notes.push_str(&format!(
                "\nWarning: the asset pattern `{pattern}` matched no file."
            ));
        }
        notes
    }

    /// Each chosen file on disk, and its path inside the asset folder.
    pub fn entries(&self) -> Vec<(PathBuf, PathBuf)> {
        self.files
            .iter()
            .map(|file| (self.folder.join(file), file.clone()))
            .collect()
    }

    /// "12 asset files", with how many were left out.
    pub fn describe(&self) -> String {
        let count = self.files.len();
        let mut text = format!("{count} asset file{}", if count == 1 { "" } else { "s" });
        if self.left_out > 0 {
            text.push_str(&format!(" ({} left out)", self.left_out));
        }
        text
    }
}

/// Chooses the files to ship from `settings.folder`.
pub fn select(settings: &Settings) -> Result<Selection, String> {
    if !settings.folder.is_dir() {
        if settings.named {
            return Err(format!(
                "the asset folder {} does not exist",
                settings.folder.display()
            ));
        }
        return Ok(Selection {
            folder: settings.folder.clone(),
            ..Selection::default()
        });
    }
    let all = files_in(&settings.folder)
        .map_err(|error| format!("could not read {}: {error}", settings.folder.display()))?;
    let include = settings.include.as_deref().unwrap_or_default();
    let mut include_used = vec![false; include.len()];
    let mut exclude_used = vec![false; settings.exclude.len()];
    let mut files = Vec::new();
    for file in &all {
        let parts: Vec<String> = file
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        let mut included = settings.include.is_none();
        let mut named = false;
        for (index, pattern) in include.iter().enumerate() {
            if pattern_matches(pattern, &parts) {
                include_used[index] = true;
                included = true;
                named = true;
            }
        }
        if !included {
            continue;
        }
        if !named
            && DEFAULT_EXCLUDES
                .iter()
                .any(|pattern| pattern_matches(pattern, &parts))
        {
            continue;
        }
        let mut excluded = false;
        for (index, pattern) in settings.exclude.iter().enumerate() {
            if pattern_matches(pattern, &parts) {
                exclude_used[index] = true;
                excluded = true;
            }
        }
        if !excluded {
            files.push(file.clone());
        }
    }
    let unused = |patterns: &[String], used: &[bool]| -> Vec<String> {
        patterns
            .iter()
            .zip(used)
            .filter(|(_, used)| !**used)
            .map(|(pattern, _)| pattern.clone())
            .collect()
    };
    let mut unused_patterns = unused(include, &include_used);
    unused_patterns.extend(unused(&settings.exclude, &exclude_used));
    Ok(Selection {
        folder: settings.folder.clone(),
        left_out: all.len() - files.len(),
        files,
        unused_patterns,
    })
}

/// Whether `pattern` picks out the file whose path inside the asset
/// folder is `parts`.
///
/// A pattern without a slash, such as `*.psd` or `raw`, matches a file or
/// folder of that name at any depth, and a folder takes everything in it
/// along. A pattern with a slash, such as `music/*.ogg`, matches paths
/// from the top of the asset folder, again with a folder's contents. `*`
/// matches within one name, `**` across folders, `?` one character and
/// `[abc]` or `[a-z]` one of a set, `[!abc]` one not in it.
pub fn pattern_matches(pattern: &str, parts: &[String]) -> bool {
    let pattern = pattern.trim_end_matches('/');
    let anchored = pattern.trim_start_matches('/');
    if anchored.is_empty() {
        return false;
    }
    if pattern.contains('/') {
        let pattern: Vec<char> = anchored.chars().collect();
        (1..=parts.len()).any(|depth| {
            let path: Vec<char> = parts[..depth].join("/").chars().collect();
            glob(&pattern, &path)
        })
    } else {
        let pattern: Vec<char> = pattern.chars().collect();
        parts.iter().any(|part| {
            let name: Vec<char> = part.chars().collect();
            glob(&pattern, &name)
        })
    }
}

/// Matches a whole path against a glob.
fn glob(pattern: &[char], text: &[char]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some('*') if pattern.get(1) == Some(&'*') => {
            // `**/` also matches no folders at all.
            let mut rest = &pattern[2..];
            if rest.first() == Some(&'/') && glob(&rest[1..], text) {
                return true;
            }
            rest = rest.strip_prefix(&['/']).unwrap_or(rest);
            (0..=text.len()).any(|skip| glob(rest, &text[skip..]))
        }
        Some('*') => {
            let rest = &pattern[1..];
            for skip in 0..=text.len() {
                if glob(rest, &text[skip..]) {
                    return true;
                }
                if text.get(skip) == Some(&'/') {
                    break;
                }
            }
            false
        }
        Some('?') => text.first().is_some_and(|&c| c != '/') && glob(&pattern[1..], &text[1..]),
        Some('[') => match (class(&pattern[1..]), text.first()) {
            (Some((matches, length)), Some(&c)) if c != '/' => {
                matches(c) && glob(&pattern[1 + length..], &text[1..])
            }
            // An unclosed bracket is an ordinary character.
            (None, Some('[')) => glob(&pattern[1..], &text[1..]),
            _ => false,
        },
        Some(&c) => text.first() == Some(&c) && glob(&pattern[1..], &text[1..]),
    }
}

/// Reads a `[...]` set from just after its `[`: a test for a character,
/// and how much of the pattern the set took including its `]`.
#[allow(clippy::type_complexity)]
fn class(pattern: &[char]) -> Option<(Box<dyn Fn(char) -> bool + '_>, usize)> {
    let negated = matches!(pattern.first(), Some('!' | '^'));
    let start = usize::from(negated);
    // A `]` straight after the opening is part of the set.
    let end = start + 1 + pattern.get(start + 1..)?.iter().position(|&c| c == ']')?;
    let set = &pattern[start..end];
    let test = move |c: char| {
        let mut index = 0;
        let mut found = false;
        while index < set.len() {
            if index + 2 < set.len() && set[index + 1] == '-' {
                found |= (set[index]..=set[index + 2]).contains(&c);
                index += 3;
            } else {
                found |= set[index] == c;
                index += 1;
            }
        }
        found != negated
    };
    Some((Box::new(test), end + 1))
}

/// The files under `folder`, as paths relative to it, in a stable order.
pub fn files_in(folder: &Path) -> io::Result<Vec<PathBuf>> {
    fn walk(root: &Path, folder: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(folder)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, files)?;
            } else if let Ok(relative) = path.strip_prefix(root) {
                files.push(relative.to_path_buf());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(folder, folder, &mut files)?;
    files.sort();
    Ok(files)
}

/// Kinds of file that are compressed already, which a zip stores as they
/// are rather than spend time failing to shrink.
const COMPRESSED: &[&str] = &[
    "png", "jpg", "jpeg", "ogg", "ogv", "mp3", "flac", "zip", "gz", "webp",
];

/// A file or folder from `extra` to ship beside the assets, and the path
/// it ships under.
#[derive(Debug, Clone, PartialEq)]
pub struct Extra {
    pub source: PathBuf,
    pub name: PathBuf,
}

/// The extra files to ship: each file on disk, and its path beside the
/// assets folder.
#[derive(Debug, Default, PartialEq)]
pub struct Extras {
    pub files: Vec<(PathBuf, PathBuf)>,
}

impl Extras {
    /// "3 extra files".
    pub fn describe(&self) -> String {
        let count = self.files.len();
        format!("{count} extra file{}", if count == 1 { "" } else { "s" })
    }
}

/// Gathers the files of `extras`. A folder brings everything in it but
/// the files [`DEFAULT_EXCLUDES`] leaves out. `reserved` holds names the
/// package already uses beside the assets, which an extra cannot take.
pub fn select_extras(extras: &[Extra], reserved: &[String]) -> Result<Extras, String> {
    let mut chosen: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for extra in extras {
        let first = extra
            .name
            .components()
            .next()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
        if reserved.contains(&first) {
            return Err(format!(
                "the extra `{}` cannot be shipped: the package already has `{first}` there",
                extra.name.display()
            ));
        }
        let metadata = fs::metadata(&extra.source).map_err(|error| {
            format!(
                "the extra path {} cannot be shipped: {error}",
                extra.source.display()
            )
        })?;
        let mut add = |source: PathBuf, name: PathBuf| match chosen.get(&name) {
            Some(earlier) if *earlier != source => Err(format!(
                "two extras both ship as {}: {} and {}",
                name.display(),
                earlier.display(),
                source.display()
            )),
            _ => {
                chosen.insert(name, source);
                Ok(())
            }
        };
        if metadata.is_dir() {
            let files = files_in(&extra.source)
                .map_err(|error| format!("could not read {}: {error}", extra.source.display()))?;
            for file in files {
                let parts: Vec<String> = file
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                if DEFAULT_EXCLUDES
                    .iter()
                    .any(|pattern| pattern_matches(pattern, &parts))
                {
                    continue;
                }
                add(extra.source.join(&file), extra.name.join(&file))?;
            }
        } else {
            add(extra.source.clone(), extra.name.clone())?;
        }
    }
    Ok(Extras {
        files: chosen
            .into_iter()
            .map(|(name, source)| (source, name))
            .collect(),
    })
}

/// Copies files, given as each one's path on disk and its path inside
/// `to`, into `to`.
pub fn copy_entries(entries: &[(PathBuf, PathBuf)], to: &Path) -> io::Result<()> {
    for (source, name) in entries {
        let destination = to.join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    Ok(())
}

/// Writes files, given as each one's path on disk and its path inside the
/// zip, into a zip at `zip_path`, for the executable to build in.
pub fn zip(entries: &[(PathBuf, PathBuf)], zip_path: &Path) -> io::Result<()> {
    let mut zip = zip::ZipWriter::new(File::create(zip_path)?);
    for (source, file) in entries {
        let compressed = file
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| COMPRESSED.contains(&extension.to_ascii_lowercase().as_str()));
        let options = SimpleFileOptions::default().compression_method(if compressed {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        });
        let name: Vec<String> = file
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        zip.start_file(name.join("/"), options)?;
        zip.write_all(&fs::read(source)?)?;
    }
    zip.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, path: &str) -> bool {
        let parts: Vec<String> = path.split('/').map(str::to_string).collect();
        pattern_matches(pattern, &parts)
    }

    #[test]
    fn a_name_pattern_matches_at_any_depth_and_takes_a_folder_along() {
        assert!(matches("*.psd", "hero.psd"));
        assert!(matches("*.psd", "art/raw/hero.psd"));
        assert!(!matches("*.psd", "hero.png"));
        assert!(matches("raw", "art/raw/hero.png"));
        assert!(matches("raw/", "raw/hero.png"));
        assert!(!matches("raw", "art/rawest/hero.png"));
        assert!(matches(".*", "sounds/.DS_Store"));
        assert!(matches("*~", "level.txt~"));
        assert!(matches("level-?.txt", "levels/level-1.txt"));
        assert!(!matches("level-?.txt", "levels/level-10.txt"));
    }

    #[test]
    fn a_path_pattern_matches_from_the_top() {
        assert!(matches("music/*.ogg", "music/theme.ogg"));
        assert!(!matches("music/*.ogg", "music/old/theme.ogg"));
        assert!(!matches("music/*.ogg", "extra/music/theme.ogg"));
        assert!(matches("music/**/*.ogg", "music/old/theme.ogg"));
        assert!(matches("music/**/*.ogg", "music/theme.ogg"));
        assert!(matches("**/*.ogg", "theme.ogg"));
        assert!(matches("art/raw", "art/raw/a/b.png"));
        assert!(matches("/levels", "levels/1.txt"));
        assert!(matches("**", "a/b/c"));
    }

    #[test]
    fn sets_match_one_character() {
        assert!(matches("level-[0-9].txt", "level-4.txt"));
        assert!(!matches("level-[0-9].txt", "level-x.txt"));
        assert!(matches("level-[!0-9].txt", "level-x.txt"));
        assert!(matches("[ab].txt", "b.txt"));
        assert!(matches("a[.txt", "a[.txt"));
    }

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str, files: &[&str]) -> Scratch {
        let folder = std::env::temp_dir().join(format!(
            "cargo-smitten-assets-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        for file in files {
            let path = folder.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, file).unwrap();
        }
        Scratch(folder)
    }

    fn settings(folder: &Path, include: Option<&[&str]>, exclude: &[&str]) -> Settings {
        Settings {
            folder: folder.to_path_buf(),
            named: false,
            include: include.map(|list| list.iter().map(|p| p.to_string()).collect()),
            exclude: exclude.iter().map(|p| p.to_string()).collect(),
            embed: false,
        }
    }

    fn chosen(selection: &Selection) -> Vec<String> {
        selection
            .files
            .iter()
            .map(|file| file.to_string_lossy().replace('\\', "/"))
            .collect()
    }

    #[test]
    fn hidden_files_and_excluded_patterns_are_left_out() {
        let dir = scratch(
            "exclude",
            &[
                "hero.png",
                "hero.psd",
                ".DS_Store",
                "sounds/.gitkeep",
                "sounds/blip.wav",
                "raw/sketch.png",
            ],
        );
        let selection = select(&settings(&dir.0, None, &["*.psd", "raw", "*.bak"])).unwrap();
        assert_eq!(chosen(&selection), ["hero.png", "sounds/blip.wav"]);
        assert_eq!(selection.left_out, 4);
        assert_eq!(selection.unused_patterns, ["*.bak"]);
        assert_eq!(selection.describe(), "2 asset files (4 left out)");
        assert!(selection.notes().contains("`*.bak` matched no file"));
    }

    #[test]
    fn include_picks_files_and_can_name_a_hidden_one() {
        let dir = scratch(
            "include",
            &[
                "pong.png",
                "shardfall-ship.png",
                "shardfall/.config",
                "blip.wav",
            ],
        );
        let selection = select(&settings(&dir.0, Some(&["shardfall*", "blip.wav"]), &[])).unwrap();
        assert_eq!(
            chosen(&selection),
            ["blip.wav", "shardfall/.config", "shardfall-ship.png"]
        );
        let selection = select(&settings(&dir.0, Some(&["shardfall*"]), &["shardfall"])).unwrap();
        assert_eq!(chosen(&selection), ["shardfall-ship.png"]);
        let nothing = select(&settings(&dir.0, Some(&[]), &[])).unwrap();
        assert!(nothing.files.is_empty());
        assert_eq!(nothing.left_out, 4);
    }

    #[test]
    fn a_missing_folder_is_empty_unless_it_was_named() {
        let missing = std::env::temp_dir().join("cargo-smitten-no-such-assets");
        let mut wanted = settings(&missing, None, &[]);
        assert_eq!(select(&wanted).unwrap().files.len(), 0);
        wanted.named = true;
        assert!(select(&wanted).unwrap_err().contains("does not exist"));
    }

    #[test]
    fn a_missing_extra_is_an_error_that_names_it() {
        let dir = scratch("extra-missing", &["build.txt"]);
        let missing = dir.0.join("data");
        let error = select_extras(
            &[Extra {
                source: missing.clone(),
                name: PathBuf::from("data"),
            }],
            &[],
        )
        .unwrap_err();
        assert!(error.contains(&missing.display().to_string()), "{error}");
    }

    #[test]
    fn extras_cannot_take_reserved_names_or_each_others_paths() {
        let dir = scratch(
            "extra-clash",
            &["a/build.txt", "b/build.txt", "assets/x.png"],
        );
        let extra = |source: &str, name: &str| Extra {
            source: dir.0.join(source),
            name: PathBuf::from(name),
        };
        let reserved = ["assets".to_string()];
        assert!(
            select_extras(&[extra("assets", "assets")], &reserved)
                .unwrap_err()
                .contains("already has `assets`")
        );
        assert!(
            select_extras(
                &[
                    extra("a/build.txt", "build.txt"),
                    extra("b/build.txt", "build.txt")
                ],
                &reserved
            )
            .unwrap_err()
            .contains("both ship as build.txt")
        );
        // The same file listed twice ships once.
        let twice = select_extras(
            &[extra("a", "a"), extra("a/build.txt", "a/build.txt")],
            &reserved,
        )
        .unwrap();
        assert_eq!(twice.files.len(), 1);
    }

    #[test]
    fn chosen_files_are_copied_and_zipped_at_the_top_level() {
        let dir = scratch("zip", &["a.txt", "sounds/b.ogg", "skip.psd"]);
        let selection = select(&settings(&dir.0.join("."), None, &["*.psd"])).unwrap();
        let copy_to = dir.0.join("copy");
        copy_entries(&selection.entries(), &copy_to).unwrap();
        assert_eq!(
            fs::read_to_string(copy_to.join("sounds/b.ogg")).unwrap(),
            "sounds/b.ogg"
        );
        assert!(!copy_to.join("skip.psd").exists());

        let zip_path = dir.0.join("assets.zip");
        zip(&selection.entries(), &zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "sounds/b.ogg"]);
        assert_eq!(
            archive.by_name("sounds/b.ogg").unwrap().compression(),
            CompressionMethod::Stored
        );
        assert_eq!(
            archive.by_name("a.txt").unwrap().compression(),
            CompressionMethod::Deflated
        );
    }
}
