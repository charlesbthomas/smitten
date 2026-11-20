use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use zip::write::SimpleFileOptions;

use crate::assets;
use crate::macos;

/// What to package, from the command line.
#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub package: Option<String>,
    pub target: Target,
    pub out: Option<PathBuf>,
    /// macOS: the name the app shows, over the game's metadata.
    pub name: Option<String>,
    /// macOS: the bundle identifier, over the game's metadata.
    pub identifier: Option<String>,
    /// macOS: a square PNG to make the app's icon from.
    pub icon: Option<PathBuf>,
    /// macOS: how to sign the app.
    pub signing: macos::Signing,
    /// The folder to ship assets from, over the game's metadata.
    pub assets: Option<PathBuf>,
    /// Patterns for the asset files to ship, in place of the metadata's.
    pub include: Vec<String>,
    /// Patterns for asset files to leave out, on top of the metadata's.
    pub exclude: Vec<String>,
    /// Build the assets into the executable instead of beside it.
    pub embed_assets: bool,
    /// Files and folders to ship beside the assets, on top of the
    /// metadata's `extra`.
    pub extra: Vec<PathBuf>,
}

/// Which executable of the package to ship.
#[derive(Debug, Default, PartialEq)]
pub enum Target {
    /// The package's only binary.
    #[default]
    DefaultBin,
    Bin(String),
    Example(String),
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut options = Options::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || {
                args.next()
                    .cloned()
                    .ok_or_else(|| format!("`{arg}` needs a value"))
            };
            match arg.as_str() {
                "-p" | "--package" => options.package = Some(value()?),
                "--bin" => options.target = Target::Bin(value()?),
                "--example" => options.target = Target::Example(value()?),
                "--out" => options.out = Some(PathBuf::from(value()?)),
                "--name" => options.name = Some(value()?),
                "--identifier" => options.identifier = Some(value()?),
                "--icon" => options.icon = Some(PathBuf::from(value()?)),
                "--sign" => options.signing.identity = Some(value()?),
                "--entitlements" => options.signing.entitlements = Some(PathBuf::from(value()?)),
                "--notarize" => options.signing.notary_profile = Some(value()?),
                "--assets" => options.assets = Some(PathBuf::from(value()?)),
                "--include" => options.include.push(value()?),
                "--exclude" => options.exclude.push(value()?),
                "--embed-assets" => options.embed_assets = true,
                "--extra" => options.extra.push(PathBuf::from(value()?)),
                other => return Err(format!("unknown option `{other}`")),
            }
        }
        if options.signing.notary_profile.is_some() && options.signing.identity.is_none() {
            return Err(
                "`--notarize` needs a Developer ID to sign with: add `--sign <identity>`"
                    .to_string(),
            );
        }
        Ok(options)
    }

    /// Fills signing options left out from the environment, so CI can
    /// keep them in secrets: `SMITTEN_SIGN_IDENTITY` and
    /// `SMITTEN_NOTARY_PROFILE`.
    fn with_environment(mut self, var: impl Fn(&str) -> Option<String>) -> Options {
        let signing = &mut self.signing;
        signing.identity = signing
            .identity
            .take()
            .or_else(|| var("SMITTEN_SIGN_IDENTITY"));
        signing.notary_profile = signing
            .notary_profile
            .take()
            .or_else(|| var("SMITTEN_NOTARY_PROFILE"));
        self
    }
}

/// The parts of `cargo metadata` that packaging needs.
#[derive(Debug, PartialEq)]
struct Project {
    package: String,
    /// The folder holding the package's `Cargo.toml`.
    folder: PathBuf,
    target_directory: PathBuf,
    /// The name of the executable to ship, without an extension.
    executable: String,
    is_example: bool,
    version: String,
    /// The package's `[package.metadata.smitten]` table, with the
    /// `bin.<name>` or `example.<name>` table for the executable laid over
    /// it, or null.
    settings: serde_json::Value,
}

fn cargo() -> Command {
    // Cargo sets this for its subcommands, so the same cargo is used.
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn read_metadata() -> Result<serde_json::Value, String> {
    let output = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|error| format!("could not run cargo: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("could not read cargo's description of the project: {error}"))
}

/// Works out which package and executable `options` mean. `current` is
/// the folder the command was run from.
fn find_project(
    metadata: &serde_json::Value,
    options: &Options,
    current: &Path,
) -> Result<Project, String> {
    let packages = metadata["packages"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let folder_of = |package: &serde_json::Value| {
        Path::new(package["manifest_path"].as_str().unwrap_or_default())
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    };

    let package = match &options.package {
        Some(name) => packages
            .iter()
            .find(|package| package["name"] == name.as_str())
            .ok_or_else(|| format!("there is no package named `{name}` here"))?,
        // Without a name, use the package whose folder we are in, or the
        // only package there is.
        None => packages
            .iter()
            .find(|package| folder_of(package) == current)
            .or(match packages {
                [only] => Some(only),
                _ => None,
            })
            .ok_or("this folder is not a package; name one with `-p <name>`")?,
    };
    let name = package["name"].as_str().unwrap_or_default().to_string();

    let targets_of_kind = |kind: &str| -> Vec<&str> {
        package["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|target| {
                target["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|k| k == kind))
            })
            .filter_map(|target| target["name"].as_str())
            .collect()
    };
    let pick = |wanted: &str, kind: &str, label: &str| {
        let available = targets_of_kind(kind);
        if available.contains(&wanted) {
            Ok(wanted.to_string())
        } else {
            Err(format!(
                "`{name}` has no {label} named `{wanted}` (it has: {})",
                if available.is_empty() {
                    "none".to_string()
                } else {
                    available.join(", ")
                }
            ))
        }
    };
    let (executable, is_example) = match &options.target {
        Target::Bin(wanted) => (pick(wanted, "bin", "binary")?, false),
        Target::Example(wanted) => (pick(wanted, "example", "example")?, true),
        Target::DefaultBin => match targets_of_kind("bin").as_slice() {
            [only] => (only.to_string(), false),
            [] => {
                return Err(format!(
                    "`{name}` has no binary to package; choose an example with `--example <name>`"
                ));
            }
            several => {
                return Err(format!(
                    "`{name}` has several binaries; choose one with `--bin <name>` ({})",
                    several.join(", ")
                ));
            }
        },
    };

    let settings = settings_for(
        &package["metadata"]["smitten"],
        if is_example { "example" } else { "bin" },
        &executable,
    )?;
    Ok(Project {
        folder: folder_of(package),
        package: name,
        target_directory: PathBuf::from(metadata["target_directory"].as_str().unwrap_or("target")),
        executable,
        is_example,
        version: package["version"].as_str().unwrap_or("0.0.0").to_string(),
        settings,
    })
}

/// The settings for one executable: `[package.metadata.smitten]`, with
/// what `[package.metadata.smitten.<kind>.<executable>]` sets in place of
/// the same keys there. The `bin` and `example` tables themselves are
/// left out.
fn settings_for(
    table: &serde_json::Value,
    kind: &str,
    executable: &str,
) -> Result<serde_json::Value, String> {
    let serde_json::Value::Object(table) = table else {
        return match table {
            serde_json::Value::Null => Ok(serde_json::Value::Null),
            _ => Err("[package.metadata.smitten] must be a table".to_string()),
        };
    };
    let mut settings = table.clone();
    settings.remove("bin");
    settings.remove("example");
    match &table.get(kind).unwrap_or(&serde_json::Value::Null)[executable] {
        serde_json::Value::Null => {}
        serde_json::Value::Object(overrides) => settings.extend(overrides.clone()),
        _ => {
            return Err(format!(
                "[package.metadata.smitten.{kind}.{executable}] must be a table"
            ));
        }
    }
    Ok(serde_json::Value::Object(settings))
}

/// Which asset files to ship and how: from the command line, then the
/// game's metadata, then the `assets` folder next to its `Cargo.toml`.
fn asset_settings(project: &Project, options: &Options) -> Result<assets::Settings, String> {
    let settings = &project.settings;
    let patterns = |key: &str| -> Result<Option<Vec<String>>, String> {
        let bad = || format!("`{key}` in [package.metadata.smitten] must be a list of patterns");
        match &settings[key] {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::Array(list) => list
                .iter()
                .map(|pattern| pattern.as_str().map(str::to_string).ok_or_else(bad))
                .collect::<Result<_, _>>()
                .map(Some),
            _ => Err(bad()),
        }
    };
    let folder = match (&options.assets, &settings["assets"]) {
        (Some(folder), _) => Some(folder.clone()),
        (None, serde_json::Value::Null) => None,
        (None, serde_json::Value::String(folder)) => Some(project.folder.join(folder)),
        _ => {
            return Err("`assets` in [package.metadata.smitten] must be a folder name".to_string());
        }
    };
    let embed = match &settings["embed-assets"] {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(embed) => *embed,
        _ => {
            return Err(
                "`embed-assets` in [package.metadata.smitten] must be true or false".to_string(),
            );
        }
    };
    let include = if options.include.is_empty() {
        patterns("include")?
    } else {
        Some(options.include.clone())
    };
    let mut exclude = patterns("exclude")?.unwrap_or_default();
    exclude.extend(options.exclude.iter().cloned());
    Ok(assets::Settings {
        named: folder.is_some(),
        folder: folder.unwrap_or_else(|| project.folder.join("assets")),
        include,
        exclude,
        embed: embed || options.embed_assets,
    })
}

/// The files and folders to ship beside the assets: `extra` in the
/// metadata, relative to `Cargo.toml` and shipped under the same path,
/// then `--extra` paths, shipped under their own names.
fn extra_list(project: &Project, options: &Options) -> Result<Vec<assets::Extra>, String> {
    let bad = || "`extra` in [package.metadata.smitten] must be a list of paths".to_string();
    let listed: Vec<&str> = match &project.settings["extra"] {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(list) => list
            .iter()
            .map(|path| path.as_str().ok_or_else(bad))
            .collect::<Result<_, _>>()?,
        _ => return Err(bad()),
    };
    let mut extras = Vec::new();
    for path in listed {
        // Shipped under the same path, so the game finds it at the same
        // place under `cargo run` and when released.
        let name: PathBuf = Path::new(path)
            .components()
            .map(|part| match part {
                std::path::Component::Normal(name) => Ok(name),
                std::path::Component::CurDir => Ok(std::ffi::OsStr::new("")),
                _ => Err(format!(
                    "the extra `{path}` must be a path inside the package, relative to Cargo.toml"
                )),
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|name| !name.is_empty())
            .collect();
        if name.as_os_str().is_empty() {
            return Err(format!("the extra `{path}` names no file or folder"));
        }
        extras.push(assets::Extra {
            source: project.folder.join(&name),
            name,
        });
    }
    for path in &options.extra {
        let name = path
            .file_name()
            .ok_or_else(|| format!("--extra {} names no file or folder", path.display()))?;
        extras.push(assets::Extra {
            source: path.clone(),
            name: PathBuf::from(name),
        });
    }
    Ok(extras)
}

/// The files that go beside the executable, or in a Mac app's Resources,
/// as each one's path on disk and its path there: the assets in their
/// folder, and the extras under their own names.
fn loose_entries(
    selection: &assets::Selection,
    extras: &assets::Extras,
) -> Vec<(PathBuf, PathBuf)> {
    selection
        .entries()
        .into_iter()
        .map(|(source, name)| (source, Path::new("assets").join(name)))
        .chain(extras.files.iter().cloned())
        .collect()
}

/// The files to build into the executable, as each one's path on disk and
/// its path in the zip. The assets sit at the top of the zip and the
/// extras beside them under their own names, so a read of `data/x.json`
/// or `build.txt` finds them as it would beside the executable.
fn built_in_entries(
    selection: &assets::Selection,
    extras: &assets::Extras,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut entries = selection.entries();
    for (source, name) in &extras.files {
        if selection.files.contains(name) {
            return Err(format!(
                "the extra {} has the same path as an asset, so they cannot both be built in",
                name.display()
            ));
        }
        entries.push((source.clone(), name.clone()));
    }
    Ok(entries)
}

/// Zips the files to build into the executable, each given as its path on
/// disk and its path inside the zip, returning the zip's path, or `None`
/// when there is nothing to embed.
fn embedded_zip(
    project: &Project,
    entries: &[(PathBuf, PathBuf)],
) -> Result<Option<PathBuf>, String> {
    if entries.is_empty() {
        return Ok(None);
    }
    let folder = project.target_directory.join("smitten-embed");
    fs::create_dir_all(&folder)
        .map_err(|error| format!("could not create {}: {error}", folder.display()))?;
    let zip_path = folder.join(format!("{}-assets.zip", project.executable));
    assets::zip(entries, &zip_path)
        .map_err(|error| format!("could not write {}: {error}", zip_path.display()))?;
    // The build reads the zip by this path from inside another crate, so
    // it must not depend on the folder the build runs in.
    let zip_path = zip_path.canonicalize().unwrap_or(zip_path);
    Ok(Some(zip_path))
}

/// Checks that the built executable carries the zip: a game built against
/// a Smitten from before embedding existed builds fine but ignores it.
fn check_embedded(built: &Path, zip_path: &Path) -> Result<(), String> {
    let zip = fs::read(zip_path)
        .map_err(|error| format!("could not read {}: {error}", zip_path.display()))?;
    let executable =
        fs::read(built).map_err(|error| format!("could not read {}: {error}", built.display()))?;
    // The end of a zip holds its sizes and offsets, which no other bytes
    // in the executable are likely to repeat.
    let tail = &zip[zip.len().saturating_sub(64)..];
    if executable.windows(tail.len()).any(|window| window == tail) {
        Ok(())
    } else {
        Err(format!(
            "the assets were not built into {}. The game may use a version of \
             Smitten from before `embed-assets`; update it, or package without \
             embedding.",
            built.display()
        ))
    }
}

/// Explains the most likely cause of a failed build when CMake cannot be
/// found: packaging builds SDL from source, which needs it.
fn cmake_hint() -> &'static str {
    let found = Command::new("cmake")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if found {
        ""
    } else if cfg!(windows) {
        "\nCMake was not found, and packaging needs it to build SDL into the game. \
         Install CMake, or run this from \"Developer PowerShell for VS 2022\", which \
         has the copy that comes with Visual Studio."
    } else {
        "\nCMake was not found, and packaging needs it to build SDL into the game. \
         Install CMake and try again."
    }
}

pub fn run(options: Options) -> Result<String, String> {
    let options = options.with_environment(|name| std::env::var(name).ok());
    if options.signing != macos::Signing::default() && !cfg!(target_os = "macos") {
        return Err("signing and notarizing are for macOS apps; package on a Mac".to_string());
    }
    let current = std::env::current_dir().map_err(|error| error.to_string())?;
    let project = find_project(&read_metadata()?, &options, &current)?;
    let bundle = if cfg!(target_os = "macos") {
        Some(bundle_info(&project, &options)?)
    } else {
        None
    };
    let asset_settings = asset_settings(&project, &options)?;
    let selection = assets::select(&asset_settings)?;
    let file_name = format!("{}{}", project.executable, std::env::consts::EXE_SUFFIX);
    // Names the package already has beside the assets folder.
    let reserved = if bundle.is_some() {
        vec!["assets".to_string(), "AppIcon.icns".to_string()]
    } else {
        vec!["assets".to_string(), file_name.clone()]
    };
    let extras = assets::select_extras(&extra_list(&project, &options)?, &reserved)?;
    let embedded = if asset_settings.embed {
        embedded_zip(&project, &built_in_entries(&selection, &extras)?)?
    } else {
        None
    };

    // Build with SDL inside the executable, so there is no separate
    // library for players to be missing.
    let mut build = cargo();
    build.args(["build", "--release", "-p", &project.package]);
    build.args(if project.is_example {
        ["--example", &project.executable]
    } else {
        ["--bin", &project.executable]
    });
    build.args(["--features", "smitten/sdl-static"]);
    // Smitten's build script builds this zip into the game when it is set,
    // and only then.
    match &embedded {
        Some(zip_path) => build.env("SMITTEN_EMBED_ASSETS", zip_path),
        None => build.env_remove("SMITTEN_EMBED_ASSETS"),
    };
    if let Some(bundle) = &bundle {
        // Build Rust and SDL for the oldest macOS the bundle claims, so the
        // claim holds. CMake and the cc crate read this too.
        if std::env::var_os("MACOSX_DEPLOYMENT_TARGET").is_none() {
            build.env("MACOSX_DEPLOYMENT_TARGET", &bundle.minimum_macos);
        }
    }
    let status = build
        .status()
        .map_err(|error| format!("could not run cargo: {error}"))?;
    if !status.success() {
        return Err(format!("the release build failed{}", cmake_hint()));
    }

    let mut built = project.target_directory.join("release");
    if project.is_example {
        built.push("examples");
    }
    built.push(&file_name);
    if let Some(zip_path) = &embedded {
        check_embedded(&built, zip_path)?;
    }
    let loose = if asset_settings.embed {
        Vec::new()
    } else {
        loose_entries(&selection, &extras)
    };
    let mut where_assets = selection.describe();
    if !extras.files.is_empty() {
        where_assets.push_str(&format!(" and {}", extras.describe()));
    }
    if asset_settings.embed {
        where_assets.push_str(" built in");
    }

    let out = options
        .out
        .clone()
        .unwrap_or_else(|| project.target_directory.join("package"));
    if let Some(bundle) = bundle {
        let summary = package_app(
            &project,
            &options,
            &built,
            &out,
            &loose,
            &where_assets,
            bundle,
        )?;
        return Ok(summary + &selection.notes());
    }
    let folder = out.join(&project.executable);
    write_folder(&folder, &built, &file_name, &loose)?;

    let zip_path = out.join(format!(
        "{}-{}-{}.zip",
        project.executable,
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    zip_folder(&folder, &project.executable, &zip_path)
        .map_err(|error| format!("could not write {}: {error}", zip_path.display()))?;

    Ok(format!(
        "Packaged {} with {where_assets}.\n  Folder: {}\n  Zip:    {}{}",
        project.executable,
        folder.display(),
        zip_path.display(),
        selection.notes()
    ))
}

/// Writes the Windows or Linux package folder: the executable as
/// `file_name`, and `loose`, the files that go beside it, each given as its
/// path on disk and its path inside the folder.
fn write_folder(
    folder: &Path,
    built: &Path,
    file_name: &str,
    loose: &[(PathBuf, PathBuf)],
) -> Result<(), String> {
    let describe = |action: &str, path: &Path, error: io::Error| {
        format!("could not {action} {}: {error}", path.display())
    };
    // Start from an empty folder so files from an earlier package that
    // the game no longer has are not shipped.
    if folder.exists() {
        fs::remove_dir_all(folder).map_err(|error| describe("clear", folder, error))?;
    }
    fs::create_dir_all(folder).map_err(|error| describe("create", folder, error))?;
    fs::copy(built, folder.join(file_name)).map_err(|error| describe("copy", built, error))?;
    assets::copy_entries(loose, folder).map_err(|error| describe("copy files into", folder, error))
}

/// What the macOS bundle is called and says about itself: from the command
/// line, then `[package.metadata.smitten]`, then made up from the
/// executable's name.
fn bundle_info(project: &Project, options: &Options) -> Result<macos::BundleInfo, String> {
    let setting = |key: &str| -> Result<Option<String>, String> {
        match &project.settings[key] {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(value) => Ok(Some(value.clone())),
            _ => Err(format!(
                "`{key}` in [package.metadata.smitten] must be a string"
            )),
        }
    };
    let name = match options.name.clone().or(setting("name")?) {
        Some(name) if name.contains(['/', ':']) || name.trim().is_empty() => {
            return Err(format!("`{name}` cannot be an app name"));
        }
        Some(name) => name,
        None => macos::display_name(&project.executable),
    };
    Ok(macos::BundleInfo {
        name,
        identifier: options
            .identifier
            .clone()
            .or(setting("identifier")?)
            .unwrap_or_else(|| macos::placeholder_identifier(&project.executable)),
        executable: project.executable.clone(),
        version: project.version.clone(),
        minimum_macos: setting("minimum-macos")?
            .unwrap_or_else(|| macos::DEFAULT_MINIMUM_MACOS.to_string()),
    })
}

/// The icon PNG: from the command line, then `icon` in
/// `[package.metadata.smitten]` relative to the package, then Smitten's.
fn icon_png(project: &Project, options: &Options) -> Result<Vec<u8>, String> {
    let path = match (&options.icon, project.settings["icon"].as_str()) {
        (Some(path), _) => path.clone(),
        (None, Some(path)) => project.folder.join(path),
        (None, None) => return Ok(macos::DEFAULT_ICON.to_vec()),
    };
    fs::read(&path).map_err(|error| format!("could not read the icon {}: {error}", path.display()))
}

/// Puts the built game into `Name.app` in `out`, signs it, zips it and,
/// when asked, notarizes it.
fn package_app(
    project: &Project,
    options: &Options,
    built: &Path,
    out: &Path,
    loose: &[(PathBuf, PathBuf)],
    where_assets: &str,
    info: macos::BundleInfo,
) -> Result<String, String> {
    let app_name = format!("{}.app", info.name);
    let app = out.join(&app_name);
    if app.exists() {
        fs::remove_dir_all(&app)
            .map_err(|error| format!("could not clear {}: {error}", app.display()))?;
    }
    macos::write_bundle(&app, built, loose, &info, &icon_png(project, options)?)?;
    macos::sign(&app, &options.signing, out)?;

    let zip_path = out.join(format!(
        "{}-{}-{}.zip",
        project.executable,
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    let write_zip = || {
        zip_folder(&app, &app_name, &zip_path)
            .map_err(|error| format!("could not write {}: {error}", zip_path.display()))
    };
    write_zip()?;
    let signed = match (&options.signing.identity, &options.signing.notary_profile) {
        (Some(identity), Some(profile)) => {
            macos::notarize(&zip_path, &app, profile)?;
            // Zip again so the download carries the stapled ticket.
            write_zip()?;
            format!("signed as {identity} and notarized")
        }
        (Some(identity), None) => format!("signed as {identity}, not notarized"),
        (None, _) => {
            "signed ad hoc; players open it the first time with right-click, Open".to_string()
        }
    };

    let mut summary = format!(
        "Packaged {} with {where_assets}, {signed}.\n  App: {}\n  Zip: {}",
        info.name,
        app.display(),
        zip_path.display()
    );
    if info.identifier.starts_with("com.example.") {
        summary.push_str(&format!(
            "\nThe bundle identifier is a placeholder, {}. Set your own with \
             `identifier` in [package.metadata.smitten] before sharing the game.",
            info.identifier
        ));
    }
    Ok(summary)
}

/// Writes `folder` to a zip file in which everything sits inside a single
/// top-level folder called `name`, so unzipping never scatters files.
fn zip_folder(folder: &Path, name: &str, zip_path: &Path) -> io::Result<()> {
    let mut zip = zip::ZipWriter::new(File::create(zip_path)?);
    for file in assets::files_in(folder)? {
        let source = folder.join(&file);
        let options = SimpleFileOptions::default();
        // Keep the executable runnable after unzipping on Linux and macOS.
        #[cfg(unix)]
        let options = {
            use std::os::unix::fs::PermissionsExt;
            options.unix_permissions(fs::metadata(&source)?.permissions().mode())
        };
        // Zip paths always use forward slashes.
        let inner: Vec<String> = std::iter::once(name.to_string())
            .chain(
                file.components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned()),
            )
            .collect();
        zip.start_file(inner.join("/"), options)?;
        zip.write_all(&fs::read(&source)?)?;
    }
    zip.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    fn metadata() -> serde_json::Value {
        serde_json::json!({
            "target_directory": "/work/target",
            "packages": [
                {
                    "name": "game",
                    "manifest_path": "/work/game/Cargo.toml",
                    "targets": [
                        { "name": "game", "kind": ["bin"] },
                        { "name": "demo", "kind": ["example"] },
                    ],
                },
                {
                    "name": "tools",
                    "manifest_path": "/work/tools/Cargo.toml",
                    "targets": [
                        { "name": "one", "kind": ["bin"] },
                        { "name": "two", "kind": ["bin"] },
                    ],
                },
                {
                    "name": "library",
                    "manifest_path": "/work/library/Cargo.toml",
                    "targets": [{ "name": "library", "kind": ["lib"] }],
                },
            ],
        })
    }

    #[test]
    fn options_are_read_from_the_command_line() {
        let options =
            Options::parse(&args(&["-p", "game", "--example", "demo", "--out", "dist"])).unwrap();
        assert_eq!(
            options,
            Options {
                package: Some("game".to_string()),
                target: Target::Example("demo".to_string()),
                out: Some(PathBuf::from("dist")),
                ..Options::default()
            }
        );
        assert_eq!(Options::parse(&[]).unwrap(), Options::default());
    }

    #[test]
    fn bad_options_are_reported() {
        assert!(
            Options::parse(&args(&["--bin"]))
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(
            Options::parse(&args(&["--fast"]))
                .unwrap_err()
                .contains("unknown option")
        );
    }

    #[test]
    fn the_package_in_the_current_folder_is_the_default() {
        let project =
            find_project(&metadata(), &Options::default(), Path::new("/work/game")).unwrap();
        assert_eq!(
            project,
            Project {
                package: "game".to_string(),
                folder: PathBuf::from("/work/game"),
                target_directory: PathBuf::from("/work/target"),
                executable: "game".to_string(),
                is_example: false,
                version: "0.0.0".to_string(),
                settings: serde_json::Value::Null,
            }
        );
    }

    #[test]
    fn an_example_can_be_chosen() {
        let options = Options::parse(&args(&["-p", "game", "--example", "demo"])).unwrap();
        let project = find_project(&metadata(), &options, Path::new("/work")).unwrap();
        assert_eq!(project.executable, "demo");
        assert!(project.is_example);
    }

    #[test]
    fn unclear_requests_say_what_to_add() {
        let find = |list: &[&str], folder: &str| {
            let options = Options::parse(&args(list)).unwrap();
            find_project(&metadata(), &options, Path::new(folder)).unwrap_err()
        };
        assert!(find(&[], "/work").contains("-p <name>"));
        assert!(find(&["-p", "missing"], "/work").contains("no package named `missing`"));
        assert!(find(&["-p", "tools"], "/work").contains("--bin <name>` (one, two)"));
        assert!(find(&["-p", "library"], "/work").contains("no binary to package"));
        assert!(find(&["-p", "game", "--example", "nope"], "/work").contains("it has: demo"));
    }

    #[test]
    fn macos_options_are_read_and_signing_can_come_from_the_environment() {
        let options = Options::parse(&args(&[
            "--name",
            "Pong",
            "--identifier",
            "com.ada.pong",
            "--icon",
            "art/icon.png",
            "--sign",
            "Developer ID Application: Ada",
        ]))
        .unwrap();
        assert_eq!(options.name.as_deref(), Some("Pong"));
        assert_eq!(options.identifier.as_deref(), Some("com.ada.pong"));
        assert_eq!(options.icon, Some(PathBuf::from("art/icon.png")));
        let env = |name: &str| match name {
            "SMITTEN_SIGN_IDENTITY" => Some("From CI".to_string()),
            "SMITTEN_NOTARY_PROFILE" => Some("notary".to_string()),
            _ => None,
        };
        let options = options.with_environment(env);
        // The command line wins over the environment.
        assert_eq!(
            options.signing.identity.as_deref(),
            Some("Developer ID Application: Ada")
        );
        assert_eq!(options.signing.notary_profile.as_deref(), Some("notary"));
        let from_env = Options::default().with_environment(env);
        assert_eq!(from_env.signing.identity.as_deref(), Some("From CI"));

        assert!(
            Options::parse(&args(&["--notarize", "notary"]))
                .unwrap_err()
                .contains("--sign")
        );
    }

    #[test]
    fn the_app_is_described_by_metadata_or_made_up() {
        let mut project =
            find_project(&metadata(), &Options::default(), Path::new("/work/game")).unwrap();
        let info = bundle_info(&project, &Options::default()).unwrap();
        assert_eq!(info.name, "Game");
        assert_eq!(info.identifier, "com.example.game");
        assert_eq!(info.version, "0.0.0");
        assert_eq!(info.minimum_macos, macos::DEFAULT_MINIMUM_MACOS);

        project.settings = serde_json::json!({
            "name": "Great Game",
            "identifier": "com.ada.game",
            "minimum-macos": "12.0",
        });
        let info = bundle_info(&project, &Options::default()).unwrap();
        assert_eq!(info.name, "Great Game");
        assert_eq!(info.identifier, "com.ada.game");
        assert_eq!(info.minimum_macos, "12.0");
        assert_eq!(info.executable, "game");

        let renamed = Options::parse(&args(&["--name", "Other"])).unwrap();
        assert_eq!(bundle_info(&project, &renamed).unwrap().name, "Other");
        let bad = Options::parse(&args(&["--name", "a/b"])).unwrap();
        assert!(bundle_info(&project, &bad).is_err());
        project.settings = serde_json::json!({ "name": 3 });
        assert!(
            bundle_info(&project, &Options::default())
                .unwrap_err()
                .contains("must be a string")
        );
    }

    #[test]
    fn asset_options_are_read_from_the_command_line() {
        let options = Options::parse(&args(&[
            "--assets",
            "art",
            "--include",
            "*.png",
            "--exclude",
            "raw",
            "--exclude",
            "*.psd",
            "--embed-assets",
        ]))
        .unwrap();
        assert_eq!(options.assets, Some(PathBuf::from("art")));
        assert_eq!(options.include, ["*.png"]);
        assert_eq!(options.exclude, ["raw", "*.psd"]);
        assert!(options.embed_assets);
    }

    #[test]
    fn a_table_for_the_executable_is_laid_over_the_package_settings() {
        let table = serde_json::json!({
            "name": "Smitten Examples",
            "exclude": ["*.psd"],
            "example": {
                "shardfall": { "include": ["shardfall-*"], "name": "Shardfall" },
            },
            "bin": { "shardfall": { "name": "Not this one" } },
        });
        let settings = settings_for(&table, "example", "shardfall").unwrap();
        assert_eq!(
            settings,
            serde_json::json!({
                "name": "Shardfall",
                "exclude": ["*.psd"],
                "include": ["shardfall-*"],
            })
        );
        let settings = settings_for(&table, "example", "pong").unwrap();
        assert_eq!(settings["name"], "Smitten Examples");
        assert!(settings.get("example").is_none());
        assert_eq!(
            settings_for(&serde_json::Value::Null, "bin", "game").unwrap(),
            serde_json::Value::Null
        );
        let bad = serde_json::json!({ "bin": { "game": 3 } });
        assert!(
            settings_for(&bad, "bin", "game")
                .unwrap_err()
                .contains("must be a table")
        );
    }

    #[test]
    fn asset_settings_come_from_the_command_line_then_metadata() {
        let mut project =
            find_project(&metadata(), &Options::default(), Path::new("/work/game")).unwrap();
        let plain = asset_settings(&project, &Options::default()).unwrap();
        assert_eq!(
            plain,
            assets::Settings {
                folder: PathBuf::from("/work/game/assets"),
                named: false,
                include: None,
                exclude: Vec::new(),
                embed: false,
            }
        );

        project.settings = serde_json::json!({
            "assets": "art",
            "include": ["*.png", "*.ogg"],
            "exclude": ["raw"],
            "embed-assets": true,
        });
        let from_metadata = asset_settings(&project, &Options::default()).unwrap();
        assert_eq!(from_metadata.folder, PathBuf::from("/work/game/art"));
        assert!(from_metadata.named);
        assert_eq!(
            from_metadata.include.as_deref(),
            Some(&["*.png".to_string(), "*.ogg".to_string()][..])
        );
        assert!(from_metadata.embed);

        let options = Options::parse(&args(&[
            "--assets",
            "other",
            "--include",
            "*.wav",
            "--exclude",
            "x",
        ]))
        .unwrap();
        let overridden = asset_settings(&project, &options).unwrap();
        assert_eq!(overridden.folder, PathBuf::from("other"));
        assert_eq!(
            overridden.include.as_deref(),
            Some(&["*.wav".to_string()][..])
        );
        assert_eq!(overridden.exclude, ["raw", "x"]);

        project.settings = serde_json::json!({ "include": "*.png" });
        assert!(
            asset_settings(&project, &Options::default())
                .unwrap_err()
                .contains("list of patterns")
        );
        project.settings = serde_json::json!({ "embed-assets": "yes" });
        assert!(
            asset_settings(&project, &Options::default())
                .unwrap_err()
                .contains("true or false")
        );
    }

    #[test]
    fn extras_come_from_metadata_then_the_command_line() {
        let mut project =
            find_project(&metadata(), &Options::default(), Path::new("/work/game")).unwrap();
        project.settings = serde_json::json!({ "extra": ["data", "./config/build.txt"] });
        let options = Options::parse(&args(&["--extra", "dist/demo.txt"])).unwrap();
        assert_eq!(options.extra, [PathBuf::from("dist/demo.txt")]);
        let extras = extra_list(&project, &options).unwrap();
        assert_eq!(
            extras,
            [
                assets::Extra {
                    source: PathBuf::from("/work/game/data"),
                    name: PathBuf::from("data"),
                },
                assets::Extra {
                    source: PathBuf::from("/work/game/config/build.txt"),
                    name: PathBuf::from("config/build.txt"),
                },
                assets::Extra {
                    source: PathBuf::from("dist/demo.txt"),
                    name: PathBuf::from("demo.txt"),
                },
            ]
        );

        project.settings = serde_json::json!({ "extra": ["../shared"] });
        assert!(
            extra_list(&project, &Options::default())
                .unwrap_err()
                .contains("inside the package")
        );
        project.settings = serde_json::json!({ "extra": "data" });
        assert!(
            extra_list(&project, &Options::default())
                .unwrap_err()
                .contains("list of paths")
        );
    }

    #[test]
    fn extras_ship_beside_the_assets_in_the_folder_and_zip_or_are_built_in() {
        let scratch =
            std::env::temp_dir().join(format!("cargo-smitten-extras-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let game = scratch.join("game");
        for (path, contents) in [
            ("assets/ship.png", "ship"),
            ("data/ships.json", "[]"),
            ("data/.DS_Store", "junk"),
            ("build.txt", "v1.2 abc123"),
            ("demo.txt", "demo"),
        ] {
            let path = game.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        fs::write(scratch.join("built"), "binary").unwrap();

        let mut project =
            find_project(&metadata(), &Options::default(), Path::new("/work/game")).unwrap();
        project.folder = game.clone();
        project.settings = serde_json::json!({ "extra": ["data", "build.txt"] });
        let demo = game.join("demo.txt");
        let options = Options::parse(&args(&["--extra", demo.to_str().unwrap()])).unwrap();
        let selection = assets::select(&asset_settings(&project, &options).unwrap()).unwrap();
        let extras = assets::select_extras(
            &extra_list(&project, &options).unwrap(),
            &["assets".to_string(), "game".to_string()],
        )
        .unwrap();
        assert_eq!(extras.describe(), "3 extra files");

        let folder = scratch.join("package/game");
        write_folder(
            &folder,
            &scratch.join("built"),
            "game",
            &loose_entries(&selection, &extras),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(folder.join("build.txt")).unwrap(),
            "v1.2 abc123"
        );
        let zip_path = scratch.join("package/game.zip");
        zip_folder(&folder, "game", &zip_path).unwrap();
        let archive = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "game/assets/ship.png",
                "game/build.txt",
                "game/data/ships.json",
                "game/demo.txt",
                "game/game",
            ]
        );

        let built_in: Vec<String> = built_in_entries(&selection, &extras)
            .unwrap()
            .into_iter()
            .map(|(_, name)| name.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(
            built_in,
            ["ship.png", "build.txt", "data/ships.json", "demo.txt"]
        );

        // An extra cannot take an asset's place in the built-in zip.
        let clash = assets::select_extras(
            &[assets::Extra {
                source: game.join("assets/ship.png"),
                name: PathBuf::from("ship.png"),
            }],
            &[],
        )
        .unwrap();
        assert!(
            built_in_entries(&selection, &clash)
                .unwrap_err()
                .contains("same path as an asset")
        );

        fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn a_folder_is_zipped_under_one_top_level_name() {
        let scratch =
            std::env::temp_dir().join(format!("cargo-smitten-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let source = scratch.join("source");
        fs::create_dir_all(source.join("sounds")).unwrap();
        fs::write(source.join("a.txt"), "a").unwrap();
        fs::write(source.join("sounds/b.txt"), "b").unwrap();

        let zip_path = scratch.join("out.zip");
        zip_folder(&source, "game", &zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
        names.sort();
        assert_eq!(names, ["game/a.txt", "game/sounds/b.txt"]);
        let mut contents = String::new();
        io::Read::read_to_string(
            &mut archive.by_name("game/sounds/b.txt").unwrap(),
            &mut contents,
        )
        .unwrap();
        assert_eq!(contents, "b");

        fs::remove_dir_all(&scratch).unwrap();
    }
}
