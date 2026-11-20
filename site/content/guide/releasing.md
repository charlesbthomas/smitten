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
| `--assets <folder>` | The folder of assets to ship. The default is `assets`. |
| `--include <pattern>` | Ship only the asset files that match. Repeatable. |
| `--exclude <pattern>` | Leave out the asset files that match. Repeatable. |
| `--embed-assets` | Build the assets into the executable. |

To try it on one of Smitten's examples without installing it, run this from
the Smitten repository:

```sh
cargo run -p cargo-smitten -- package -p smitten-examples --example pong
```

A package is for the platform it was built on, so build on Windows for
Windows players, on Linux for Linux players and on a Mac for Mac players.

### Choosing the assets

By default every file in the `assets` folder next to `Cargo.toml` is
shipped, apart from hidden files such as `.DS_Store` and `.gitkeep`,
`Thumbs.db`, `desktop.ini` and editor backups ending in `~` or `.swp`.
The game's `Cargo.toml` can change that:

```toml
[package.metadata.smitten]
assets = "art"                    # the folder to ship, relative to Cargo.toml
include = ["*.png", "music/**"]   # ship only these; leave out to ship everything
exclude = ["*.psd", "raw"]        # then leave these out
embed-assets = true               # build the assets into the executable
```

A pattern without a slash matches a file or folder of that name anywhere,
so `*.psd` leaves out every Photoshop file and `raw` every folder named
`raw`, with what is in it. A pattern with a slash, such as `music/*.ogg`,
matches from the top of the asset folder. `*` matches within a name, `**`
across folders, `?` one character and `[a-z]` one of a set. An `include`
pattern can name a hidden file to ship it anyway, and `include = []` ships
no assets at all. The summary says how many files were left out, and
warns about a pattern that matched nothing, which is usually a typo.

Under `cargo run` the game reads its assets from the folder `assets` names,
so renaming the folder works in development too. Excluded files are still
there while developing, so a game can use a file that will not ship: run
the package once to see.

When one package holds several games, each can have its own settings,
which replace the package's for that executable:

```toml
[package.metadata.smitten.example.shardfall]
include = ["shardfall-*"]

[package.metadata.smitten.bin.editor]
assets = "editor-assets"
```

`--assets`, `--include` and `--exclude` do the same from the command line.
`--include` replaces the patterns in `Cargo.toml`, and `--exclude` adds to
them.

### Building the assets in

With `embed-assets = true`, or `--embed-assets`, the chosen files are
zipped and built into the executable, and the package has no `assets`
folder: on Windows and Linux it is the executable alone, and on a Mac an
app with nothing in `Contents/Resources` but its icon. The game reads the
built-in files first, then the save directory and the asset directory as
usual, so `filesystem::read`, image and sound loading and the rest work
the same. `filesystem::has_embedded_assets()` says whether a game was
built this way.

Building in keeps players from browsing or replacing the game's files
casually. It is not protection: the zip is inside the executable as it is,
and anything a game draws or plays can be captured. Packaging rebuilds
Smitten with the zip in it, which takes a little longer than a plain
release build; a game with hundreds of megabytes of assets makes a very
large executable and a slow build, and is better off shipping them beside
it.

## macOS: the app bundle

On a Mac, `cargo smitten package` makes an app instead of a folder:
`target/package/Name.app`, with the executable, an icon and the game's
assets inside, and a zip of it to share. The assets go in the app's
`Contents/Resources`, unless they are built in, and Smitten finds them
there.

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
