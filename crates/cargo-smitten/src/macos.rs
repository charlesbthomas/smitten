//! Turning a game into a macOS app bundle: `Name.app`, with an
//! `Info.plist`, an icon, the executable and the game's assets, and then
//! signing it.
//!
//! Without a Developer ID the bundle is signed ad hoc, which is what
//! Apple Silicon needs to run it at all. Players then open it the first
//! time with right-click, Open (or Open Anyway in System Settings). With a
//! Developer ID, `--sign` signs it with the hardened runtime and
//! `--notarize` sends it to Apple, so it opens like any other app.

use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::process::Command;

use image::imageops::FilterType;
use image::{ImageFormat, RgbaImage};

/// Smitten's own icon, used when a game has none.
pub const DEFAULT_ICON: &[u8] = include_bytes!("../assets/icon.png");

/// The oldest macOS a bundle says it runs on unless the game says
/// otherwise. Every Mac with Apple Silicon, and Intel Macs from about
/// 2013, run it.
pub const DEFAULT_MINIMUM_MACOS: &str = "11.0";

/// What goes into a bundle's `Info.plist`.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleInfo {
    /// The name Finder and the Dock show.
    pub name: String,
    /// The reverse-DNS name macOS tells apps apart by, such as
    /// `com.example.pong`.
    pub identifier: String,
    /// The name of the executable in `Contents/MacOS`.
    pub executable: String,
    pub version: String,
    pub minimum_macos: String,
}

/// The name to show for an executable that has none of its own:
/// `spatial_audio` becomes `Spatial Audio`.
pub fn display_name(executable: &str) -> String {
    executable
        .split(['_', '-'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// A bundle identifier for a game that has not chosen one. It works, but
/// a game should set its own before it is shared, since macOS keeps
/// settings and permissions by it.
pub fn placeholder_identifier(executable: &str) -> String {
    let tail: String = executable
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("com.example.{tail}")
}

/// Escapes text for an XML plist.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The bundle's `Info.plist`.
pub fn info_plist(info: &BundleInfo) -> String {
    let strings = [
        ("CFBundleDevelopmentRegion", "en"),
        ("CFBundleDisplayName", &info.name),
        ("CFBundleExecutable", &info.executable),
        ("CFBundleIconFile", "AppIcon"),
        ("CFBundleIdentifier", &info.identifier),
        ("CFBundleInfoDictionaryVersion", "6.0"),
        ("CFBundleName", &info.name),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleShortVersionString", &info.version),
        ("CFBundleVersion", &info.version),
        ("LSApplicationCategoryType", "public.app-category.games"),
        ("LSMinimumSystemVersion", &info.minimum_macos),
        // Shown if the game records sound; macOS asks the player first.
        (
            "NSMicrophoneUsageDescription",
            "The game records sound from the microphone.",
        ),
    ];
    let mut body = String::new();
    for (key, value) in strings {
        body.push_str(&format!(
            "\t<key>{key}</key>\n\t<string>{}</string>\n",
            escape(value)
        ));
    }
    // Draw at the screen's full resolution on Retina displays, and let a
    // Mac with two GPUs stay on the one it is using.
    for key in [
        "NSHighResolutionCapable",
        "NSSupportsAutomaticGraphicsSwitching",
    ] {
        body.push_str(&format!("\t<key>{key}</key>\n\t<true/>\n"));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n"
    )
}

/// The entitlements a game signed with the hardened runtime needs: only
/// the microphone, for games that record sound.
pub const ENTITLEMENTS: &str = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
\t<key>com.apple.security.device.audio-input</key>
\t<true/>
</dict>
</plist>
";

/// The icon sizes in pixels an `.icns` file holds, by their four-letter
/// types, as Apple's `iconutil` writes them: 16 to 512 points, each at 1x
/// and 2x. The 2x types (`ic11` to `ic14`, and `ic10` for 1024) hold
/// twice the pixels of their point size.
const ICNS_SIZES: [(&[u8; 4], u32); 10] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"icp6", 64),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic13", 256),
    (b"ic09", 512),
    (b"ic14", 512),
];

/// Turns a square PNG into an `.icns` file. Each size is scaled from the
/// PNG: smoothly when shrinking, and keeping hard pixel edges when
/// growing, so pixel art stays crisp.
pub fn icns(png: &[u8]) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory_with_format(png, ImageFormat::Png)
        .map_err(|error| format!("the icon is not a PNG image: {error}"))?
        .to_rgba8();
    if image.width() != image.height() {
        return Err(format!(
            "the icon must be square, but it is {}x{}",
            image.width(),
            image.height()
        ));
    }
    let mut entries: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    let mut sizes: Vec<u32> = ICNS_SIZES.iter().map(|&(_, size)| size).collect();
    sizes.push(1024);
    let mut encoded = std::collections::HashMap::new();
    for size in sizes {
        let filter = if size > image.width() {
            FilterType::Nearest
        } else {
            FilterType::Lanczos3
        };
        let scaled: RgbaImage = image::imageops::resize(&image, size, size, filter);
        let mut bytes = Vec::new();
        scaled
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .map_err(|error| format!("could not encode the icon: {error}"))?;
        encoded.insert(size, bytes);
    }
    for &(kind, pixels) in &ICNS_SIZES {
        entries.push((kind, encoded[&pixels].clone()));
    }
    entries.push((b"ic10", encoded[&1024].clone()));

    let total: usize = 8 + entries
        .iter()
        .map(|(_, data)| 8 + data.len())
        .sum::<usize>();
    let mut file = Vec::with_capacity(total);
    file.extend_from_slice(b"icns");
    file.extend_from_slice(&(total as u32).to_be_bytes());
    for (kind, data) in entries {
        file.extend_from_slice(kind);
        file.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
        file.extend_from_slice(&data);
    }
    Ok(file)
}

/// Writes the bundle at `app`, a folder ending in `.app`, from the built
/// executable, the icon PNG, and the files to ship beside it, each given
/// as its path on disk and its path inside `Contents/Resources` (the
/// assets under `assets/`, and the extras).
pub fn write_bundle(
    app: &Path,
    executable: &Path,
    resources_files: &[(PathBuf, PathBuf)],
    info: &BundleInfo,
    icon_png: &[u8],
) -> Result<(), String> {
    let describe = |action: &str, path: &Path, error: io::Error| {
        format!("could not {action} {}: {error}", path.display())
    };
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    for folder in [&macos, &resources] {
        fs::create_dir_all(folder).map_err(|error| describe("create", folder, error))?;
    }
    let plist = contents.join("Info.plist");
    fs::write(&plist, info_plist(info)).map_err(|error| describe("write", &plist, error))?;
    // Marks the folder as an application, as Xcode does.
    let pkginfo = contents.join("PkgInfo");
    fs::write(&pkginfo, "APPL????").map_err(|error| describe("write", &pkginfo, error))?;
    let icon = resources.join("AppIcon.icns");
    fs::write(&icon, icns(icon_png)?).map_err(|error| describe("write", &icon, error))?;
    let destination = macos.join(&info.executable);
    fs::copy(executable, &destination).map_err(|error| describe("copy", executable, error))?;
    // Assets go in Resources rather than next to the executable: macOS
    // expects only code in Contents/MacOS, and signing checks that.
    // Smitten looks for them there when it runs from a bundle.
    crate::assets::copy_entries(resources_files, &resources)
        .map_err(|error| describe("copy files into", &resources, error))
}

/// How to sign a bundle.
#[derive(Debug, Default, PartialEq)]
pub struct Signing {
    /// A signing identity from the keychain, such as
    /// `Developer ID Application: Ada Lovelace (TEAMID1234)`. Without one
    /// the bundle is signed ad hoc.
    pub identity: Option<String>,
    /// An entitlements file to sign with in place of [`ENTITLEMENTS`].
    pub entitlements: Option<PathBuf>,
    /// A `notarytool` keychain profile, made with
    /// `xcrun notarytool store-credentials`, to notarize with.
    pub notary_profile: Option<String>,
}

fn run(command: &mut Command, what: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("could not run {what}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what} failed: {}{}",
            String::from_utf8_lossy(&output.stderr).trim(),
            String::from_utf8_lossy(&output.stdout).trim()
        ))
    }
}

/// Signs the bundle: ad hoc without an identity, or with the identity,
/// the hardened runtime and a secure timestamp, as notarizing requires.
/// `scratch` is a folder to write the default entitlements file to.
pub fn sign(app: &Path, signing: &Signing, scratch: &Path) -> Result<(), String> {
    let mut codesign = Command::new("codesign");
    codesign.args(["--force", "--sign"]);
    match &signing.identity {
        None => {
            codesign.arg("-");
        }
        Some(identity) => {
            let entitlements = match &signing.entitlements {
                Some(path) => path.clone(),
                None => {
                    let path = scratch.join("entitlements.plist");
                    fs::write(&path, ENTITLEMENTS)
                        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
                    path
                }
            };
            codesign
                .arg(identity)
                .args(["--options", "runtime", "--timestamp", "--entitlements"])
                .arg(entitlements);
        }
    }
    run(codesign.arg(app), "codesign")?;
    run(
        Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(app),
        "codesign --verify",
    )
}

/// Sends `zip`, holding the signed bundle, to Apple's notary service and
/// waits for the answer, then staples the ticket to the bundle so it
/// opens without a network connection.
pub fn notarize(zip: &Path, app: &Path, profile: &str) -> Result<(), String> {
    run(
        Command::new("xcrun")
            .args(["notarytool", "submit"])
            .arg(zip)
            .args(["--keychain-profile", profile, "--wait"]),
        "notarytool",
    )?;
    run(
        Command::new("xcrun").args(["stapler", "staple"]).arg(app),
        "stapler",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> BundleInfo {
        BundleInfo {
            name: "Pong & Friends".to_string(),
            identifier: "com.example.pong".to_string(),
            executable: "pong".to_string(),
            version: "1.2.0".to_string(),
            minimum_macos: DEFAULT_MINIMUM_MACOS.to_string(),
        }
    }

    #[test]
    fn names_come_from_the_executable() {
        assert_eq!(display_name("pong"), "Pong");
        assert_eq!(display_name("spatial_audio"), "Spatial Audio");
        assert_eq!(display_name("my-game"), "My Game");
        assert_eq!(
            placeholder_identifier("spatial_audio"),
            "com.example.spatial-audio"
        );
    }

    #[test]
    fn the_plist_names_the_executable_and_icon() {
        let plist = info_plist(&info());
        for expected in [
            "<key>CFBundleExecutable</key>\n\t<string>pong</string>",
            "<key>CFBundleIdentifier</key>\n\t<string>com.example.pong</string>",
            "<key>CFBundleName</key>\n\t<string>Pong &amp; Friends</string>",
            "<key>CFBundleIconFile</key>\n\t<string>AppIcon</string>",
            "<key>CFBundleShortVersionString</key>\n\t<string>1.2.0</string>",
            "<key>LSMinimumSystemVersion</key>\n\t<string>11.0</string>",
            "<key>NSHighResolutionCapable</key>\n\t<true/>",
        ] {
            assert!(plist.contains(expected), "{expected} in {plist}");
        }
        assert!(plist.starts_with("<?xml"));
        assert!(plist.ends_with("</plist>\n"));
    }

    /// The entries of an `.icns` file, by type, with the size of each
    /// PNG inside.
    fn icns_entries(file: &[u8]) -> Vec<(String, u32)> {
        assert_eq!(&file[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(file[4..8].try_into().unwrap()) as usize,
            file.len()
        );
        let mut entries = Vec::new();
        let mut at = 8;
        while at < file.len() {
            let kind = String::from_utf8(file[at..at + 4].to_vec()).unwrap();
            let length = u32::from_be_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
            let png = image::load_from_memory(&file[at + 8..at + length]).unwrap();
            assert_eq!(png.width(), png.height());
            entries.push((kind, png.width()));
            at += length;
        }
        entries
    }

    #[test]
    fn the_default_icon_makes_every_size() {
        let entries = icns_entries(&icns(DEFAULT_ICON).unwrap());
        let find = |kind: &str| entries.iter().find(|(k, _)| k == kind).unwrap().1;
        assert_eq!(entries.len(), 11);
        assert_eq!(find("icp4"), 16);
        assert_eq!(find("ic11"), 32);
        assert_eq!(find("ic07"), 128);
        assert_eq!(find("ic14"), 512);
        assert_eq!(find("ic10"), 1024);
    }

    #[test]
    fn a_small_pixel_art_icon_is_scaled_up() {
        let mut bytes = Vec::new();
        RgbaImage::from_pixel(16, 16, image::Rgba([200, 40, 60, 255]))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        let entries = icns_entries(&icns(&bytes).unwrap());
        assert!(entries.contains(&("ic10".to_string(), 1024)));
    }

    #[test]
    fn an_icon_that_is_not_square_or_not_a_png_is_refused() {
        let mut bytes = Vec::new();
        RgbaImage::new(32, 16)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        assert!(icns(&bytes).unwrap_err().contains("square"));
        assert!(icns(b"not a png").unwrap_err().contains("not a PNG"));
    }

    #[test]
    fn a_bundle_holds_the_executable_icon_and_assets() {
        let scratch =
            std::env::temp_dir().join(format!("cargo-smitten-bundle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        fs::create_dir_all(scratch.join("assets/sounds")).unwrap();
        fs::write(scratch.join("assets/sounds/blip.wav"), "blip").unwrap();
        fs::write(scratch.join("pong"), "binary").unwrap();

        let app = scratch.join("Pong.app");
        fs::write(scratch.join("build.txt"), "v1").unwrap();
        let files = [
            (
                scratch.join("assets/sounds/blip.wav"),
                PathBuf::from("assets/sounds/blip.wav"),
            ),
            (scratch.join("build.txt"), PathBuf::from("build.txt")),
        ];
        write_bundle(&app, &scratch.join("pong"), &files, &info(), DEFAULT_ICON).unwrap();
        let contents = app.join("Contents");
        assert_eq!(
            fs::read_to_string(contents.join("MacOS/pong")).unwrap(),
            "binary"
        );
        assert_eq!(
            fs::read_to_string(contents.join("Resources/assets/sounds/blip.wav")).unwrap(),
            "blip"
        );
        assert_eq!(
            fs::read_to_string(contents.join("Resources/build.txt")).unwrap(),
            "v1"
        );
        assert!(
            fs::read_to_string(contents.join("Info.plist"))
                .unwrap()
                .contains("com.example.pong")
        );
        assert_eq!(
            fs::read_to_string(contents.join("PkgInfo")).unwrap(),
            "APPL????"
        );
        assert_eq!(
            &fs::read(contents.join("Resources/AppIcon.icns")).unwrap()[..4],
            b"icns"
        );

        fs::remove_dir_all(&scratch).unwrap();
    }
}
