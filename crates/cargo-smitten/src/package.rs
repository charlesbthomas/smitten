use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use zip::write::SimpleFileOptions;

/// What to package, from the command line.
#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub package: Option<String>,
    pub target: Target,
    pub out: Option<PathBuf>,
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
                other => return Err(format!("unknown option `{other}`")),
            }
        }
        Ok(options)
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
    let current = std::env::current_dir().map_err(|error| error.to_string())?;
    let project = find_project(&read_metadata()?, &options, &current)?;

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
        .unwrap_or_else(|| project.target_directory.join("package"));
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
fn copy_folder(from: &Path, to: &Path) -> io::Result<usize> {
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
