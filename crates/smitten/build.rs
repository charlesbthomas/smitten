//! Builds a game's assets into it when `cargo smitten package` asks, by
//! setting `SMITTEN_EMBED_ASSETS` to the path of a zip of them. Without it
//! nothing is embedded and the game reads its assets folder.

use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-env-changed=SMITTEN_EMBED_ASSETS");
    println!("cargo::rustc-check-cfg=cfg(smitten_embedded_assets)");
    let Some(zip) = std::env::var_os("SMITTEN_EMBED_ASSETS").filter(|path| !path.is_empty()) else {
        return;
    };
    let zip = PathBuf::from(zip);
    let zip = zip.canonicalize().unwrap_or_else(|error| {
        panic!(
            "SMITTEN_EMBED_ASSETS names {}, which cannot be read: {error}",
            zip.display()
        )
    });
    let Some(path) = zip.to_str() else {
        panic!(
            "SMITTEN_EMBED_ASSETS must be a path in Unicode, not {}",
            zip.display()
        );
    };
    println!("cargo::rerun-if-changed={path}");
    println!("cargo::rustc-cfg=smitten_embedded_assets");
    println!("cargo::rustc-env=SMITTEN_EMBEDDED_ASSETS={path}");
}
