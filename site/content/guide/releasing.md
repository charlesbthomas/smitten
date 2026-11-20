+++
title = "Hot reload and releasing"
description = "Reloading assets while the game runs, and packaging a game for players."
weight = 12
+++

## Hot reload

Under `cargo run`, an image, shader, font or sound loaded from a file is
loaded again when that file is saved, so edits show up in the running game
without restarting it. A file with a mistake in it prints the error, and the
game keeps using the last version that worked. A sound that is playing
starts over with the new version.

Hot reload is on in debug builds and off in release builds. Set
`Config::hot_reload` to choose for yourself.

## Packaging a game

`cargo smitten package` builds a game in release mode and gathers what a
player needs: the executable, with SDL linked into it, and the game's
`assets` folder. It writes a folder and a zip of that folder to
`target/package`.

Install the command from a clone of the Smitten repository, then run it in
your game's folder:

```sh
cargo install --path crates/cargo-smitten
cd my-game
cargo smitten package
```

| Option | What it does |
|---|---|
| `-p`, `--package <name>` | The package to build, if not the one in this folder. |
| `--bin <name>` | The binary to package, if the package has several. |
| `--example <name>` | Package an example instead of a binary. |
| `--out <folder>` | Where to put the result. The default is `target/package`. |

To try it on one of Smitten's examples without installing it, run this from
the Smitten repository:

```sh
cargo run -p cargo-smitten -- package -p smitten-examples --example pong
```

A package is for the platform it was built on, so build on Windows for
Windows players, on Linux for Linux players and on a Mac for Mac players.

## macOS: the app bundle

On a Mac, `cargo smitten package` makes an app instead of a folder:
`target/package/Name.app`, with the executable, an icon and the game's
assets inside, and a zip of it to share. The assets go in the app's
`Contents/Resources`, and Smitten finds them there.

The app is named after the executable and gets Smitten's heart icon unless
the game says otherwise in its `Cargo.toml`:

```toml
[package.metadata.smitten]
name = "Pong"                    # the name Finder and the Dock show
identifier = "com.yourname.pong" # how macOS tells apps apart
icon = "art/icon.png"            # a square PNG, ideally 1024x1024
minimum-macos = "11.0"           # the oldest macOS it runs on, the default
```

Set `identifier` before sharing the game; without it the app gets a
placeholder, `com.example.<executable>`. `--name`, `--identifier` and
`--icon` do the same from the command line. A small pixel-art icon is
scaled up with hard edges, so it stays crisp.

### Signing and notarizing

Without a paid Apple Developer account the app is signed ad hoc, which is
enough for it to run, including on Apple Silicon. macOS still warns the
first time a downloaded copy is opened, because Apple has not checked it:
players open it with right-click, Open, or with Open Anyway in System
Settings, Privacy & Security.

With a Developer ID certificate in your keychain, sign it properly and
have Apple notarize it, so it opens like any other app:

```sh
# Once: store the notary service's credentials in the keychain.
xcrun notarytool store-credentials notary --apple-id you@example.com --team-id TEAMID1234

cargo smitten package --sign "Developer ID Application: Your Name (TEAMID1234)" --notarize notary
```

`--sign` signs with the hardened runtime and the microphone entitlement,
for games that record sound; `--entitlements <file>` replaces those.
`--notarize` uploads the zip, waits for Apple's answer, staples the ticket
to the app and zips it again. In CI, `SMITTEN_SIGN_IDENTITY` and
`SMITTEN_NOTARY_PROFILE` stand in for the two options.

## Windows: no console window

On Windows, add this to the top of the game's `main.rs` so it doesn't open
a console window alongside the game:

```rust
#![windows_subsystem = "windows"]
```

A game built this way has nowhere to print. Set `Config::console` to `true`
to open a console for its output anyway, which helps while testing a
release build.
