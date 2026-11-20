//! `cargo smitten package` builds a Smitten game for release and gathers
//! everything a player needs into one folder and one zip file.
//!
//! Run it from the game's folder:
//!
//! ```text
//! cargo smitten package
//! ```
//!
//! or from this workspace without installing it:
//!
//! ```text
//! cargo run -p cargo-smitten -- package -p smitten-examples --example pong
//! ```

mod assets;
mod macos;
mod package;

use std::process::ExitCode;

const USAGE: &str = "\
Packages a Smitten game for release.

Usage: cargo smitten package [options]

Options:
  -p, --package <name>   The package to build, if not the one in this folder
      --bin <name>       The binary to package, if the package has several
      --example <name>   Package an example instead of a binary
      --out <folder>     Where to put the result (default: target/package)
      --assets <folder>  The folder of assets to ship (default: assets)
      --include <glob>   Ship only the asset files that match; repeatable
      --exclude <glob>   Leave out the asset files that match; repeatable
      --embed-assets     Build the assets into the executable
  -h, --help             Show this help

On macOS:
      --name <name>          The app's name (default: from the executable)
      --identifier <id>      The bundle identifier, such as com.you.game
      --icon <png>           A square PNG for the app's icon
      --sign <identity>      Sign with a Developer ID instead of ad hoc
      --entitlements <file>  Sign with these entitlements
      --notarize <profile>   Notarize with this notarytool keychain profile

The game is built in release mode with SDL linked into the executable.
The result is a folder holding the executable and the game's assets
folder, and a zip of that folder ready to share. On macOS it is an app,
Name.app, and a zip of it. With --embed-assets the assets are inside the
executable instead, and there is no assets folder.

`assets`, `include`, `exclude`, `embed-assets`, and on macOS `name`,
`identifier`, `icon` and `minimum-macos` can be set in
[package.metadata.smitten] in Cargo.toml, and for one executable in
[package.metadata.smitten.bin.<name>] or [...smitten.example.<name>].";

fn main() -> ExitCode {
    // Run as `cargo smitten ...`, cargo passes `smitten` as the first
    // argument.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "smitten") {
        args.remove(0);
    }

    match args.first().map(String::as_str) {
        Some("package") => match package::Options::parse(&args[1..]).and_then(package::run) {
            Ok(summary) => {
                println!("{summary}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
        Some("-h" | "--help") | None => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown command `{other}`\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
