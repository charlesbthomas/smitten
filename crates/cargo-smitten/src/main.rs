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
  -h, --help             Show this help

The game is built in release mode with SDL linked into the executable.
The result is a folder holding the executable and the game's assets
folder, and a zip of that folder ready to share.";

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
