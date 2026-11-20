use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use zip::write::SimpleFileOptions;

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
    /// The package's `[package.metadata.smitten]` table, or null.
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

    Ok(Project {
        folder: folder_of(package),
        package: name,
        target_directory: PathBuf::from(metadata["target_directory"].as_str().unwrap_or("target")),
        executable,
        is_example,
        version: package["version"].as_str().unwrap_or("0.0.0").to_string(),
        settings: package["metadata"]["smitten"].clone(),
    })
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

    let file_name = format!("{}{}", project.executable, std::env::consts::EXE_SUFFIX);
    let mut built = project.target_directory.join("release");
    if project.is_example {
        built.push("examples");
    }
    built.push(&file_name);

    let out = options
        .out
        .clone()
        .unwrap_or_else(|| project.target_directory.join("package"));
    if let Some(bundle) = bundle {
        return package_app(&project, &options, &built, &out, bundle);
    }
    let folder = out.join(&project.executable);
    let describe = |action: &str, path: &Path, error: io::Error| {
        format!("could not {action} {}: {error}", path.display())
    };

    // Start from an empty folder so files from an earlier package that
    // the game no longer has are not shipped.
    if folder.exists() {
        fs::remove_dir_all(&folder).map_err(|error| describe("clear", &folder, error))?;
    }
    fs::create_dir_all(&folder).map_err(|error| describe("create", &folder, error))?;
    fs::copy(&built, folder.join(&file_name)).map_err(|error| describe("copy", &built, error))?;

    let assets = project.folder.join("assets");
    let mut asset_count = 0;
    if assets.is_dir() {
        asset_count = copy_folder(&assets, &folder.join("assets"))
            .map_err(|error| describe("copy", &assets, error))?;
    }

    let zip_path = out.join(format!(
        "{}-{}-{}.zip",
        project.executable,
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    zip_folder(&folder, &project.executable, &zip_path)
        .map_err(|error| describe("write", &zip_path, error))?;

    Ok(format!(
        "Packaged {} with {asset_count} asset file{}.\n  Folder: {}\n  Zip:    {}",
        project.executable,
        if asset_count == 1 { "" } else { "s" },
        folder.display(),
        zip_path.display()
    ))
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
    info: macos::BundleInfo,
) -> Result<String, String> {
    let app_name = format!("{}.app", info.name);
    let app = out.join(&app_name);
    if app.exists() {
        fs::remove_dir_all(&app)
            .map_err(|error| format!("could not clear {}: {error}", app.display()))?;
    }
    let assets = project.folder.join("assets");
    let asset_count = macos::write_bundle(
        &app,
        built,
        assets.is_dir().then_some(assets.as_path()),
        &info,
        &icon_png(project, options)?,
    )?;
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
        "Packaged {} with {asset_count} asset file{}, {signed}.\n  App: {}\n  Zip: {}",
        info.name,
        if asset_count == 1 { "" } else { "s" },
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

/// The files under `folder`, as paths relative to it, in a stable order.
fn files_in(folder: &Path) -> io::Result<Vec<PathBuf>> {
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

/// Copies everything under `from` into `to`. Returns how many files.
pub(crate) fn copy_folder(from: &Path, to: &Path) -> io::Result<usize> {
    let files = files_in(from)?;
    for file in &files {
        let destination = to.join(file);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(from.join(file), destination)?;
    }
    Ok(files.len())
}

/// Writes `folder` to a zip file in which everything sits inside a single
/// top-level folder called `name`, so unzipping never scatters files.
fn zip_folder(folder: &Path, name: &str, zip_path: &Path) -> io::Result<()> {
    let mut zip = zip::ZipWriter::new(File::create(zip_path)?);
    for file in files_in(folder)? {
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
    fn a_folder_is_copied_and_zipped_under_one_top_level_name() {
        let scratch =
            std::env::temp_dir().join(format!("cargo-smitten-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        let source = scratch.join("source");
        fs::create_dir_all(source.join("sounds")).unwrap();
        fs::write(source.join("a.txt"), "a").unwrap();
        fs::write(source.join("sounds/b.txt"), "b").unwrap();

        let copy = scratch.join("copy");
        assert_eq!(copy_folder(&source, &copy).unwrap(), 2);
        assert_eq!(fs::read_to_string(copy.join("sounds/b.txt")).unwrap(), "b");

        let zip_path = scratch.join("out.zip");
        zip_folder(&copy, "game", &zip_path).unwrap();
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
