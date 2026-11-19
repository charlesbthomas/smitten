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
Windows players and on Linux for Linux players.

## Windows: no console window

On Windows, add this to the top of the game's `main.rs` so it doesn't open
a console window alongside the game:

```rust
#![windows_subsystem = "windows"]
```

A game built this way has nowhere to print. Set `Config::console` to `true`
to open a console for its output anyway, which helps while testing a
release build.
