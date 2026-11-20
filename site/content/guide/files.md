+++
title = "Files, saves and data"
description = "Where assets and save files live, mounting archives, and the data module."
weight = 9
+++

## Where files come from

The {{ api(path="filesystem/index.html", text="filesystem") }} module reads from two places and writes to one:

- The **asset directory** holds the files shipped with the game. Under
  `cargo run` it is the `assets` folder next to the game's `Cargo.toml`, or
  the folder `assets` names in its `[package.metadata.smitten]`. In a
  released game it is the `assets` folder next to the executable, or in a
  Mac app's `Contents/Resources`. A game packaged with
  `cargo smitten package --embed-assets` carries its assets inside the
  executable, and reads those first.
- The **save directory** is a per-user folder the game may write to, named
  after the game's identity. Set the identity with `Config::identity`; it
  defaults to the executable's name.

Relative paths are looked up in the save directory first and then in the
asset directory, so a saved file can override a shipped one.
`Config::append_identity` reverses the order. Image, font, shader and sound
loaders use the same lookup, and writing always goes to the save
directory.

## Saving and loading

```rust
use smitten::filesystem;

// Save.
if let Err(error) = filesystem::write("progress.txt", self.level.to_string()) {
    eprintln!("could not save: {error}");
}

// Load, falling back to the first level.
let level: u32 = filesystem::read_string("progress.txt")
    .ok()
    .and_then(|text| text.trim().parse().ok())
    .unwrap_or(1);
```

`read`, `read_string` and `lines` read files. `write`, `append`,
`create_directory` and `remove` change the save directory. `exists`,
`info` and `directory_items` look around. Errors are ordinary
`std::io::Error`s.

`filesystem::save_directory()` says where saves actually go on this
computer. For a file too large to read at once, or one written over time,
`filesystem::open_file(path, FileMode::Read)` gives a
{{ api(path="filesystem/struct.File.html", text="File") }} to read or write a
piece at a time.

## Mounting archives

`filesystem::mount(path, mount_point)` makes the files inside a zip archive
or a directory readable like the game's own, for downloadable level packs
or mods. A folder the player drops on the window arrives through the
`directory_dropped` callback, ready to mount:

```rust
fn directory_dropped(&mut self, path: &Path, _x: f32, _y: f32) {
    if filesystem::mount(path, "mods").is_ok() {
        self.mods = filesystem::directory_items("mods");
    }
}
```

Mounted files cannot be written.

## The data module

{{ api(path="data/index.html", text="smitten::data") }}
compresses, encodes, hashes and packs bytes. Like `math`, it
works on any thread and without a running game.

```rust
use smitten::data::{self, CompressedDataFormat, EncodeFormat, HashFunction};

let save = b"level=3;coins=120";
let packed = data::compress(CompressedDataFormat::Zlib, save, None);
assert_eq!(data::decompress(&packed).unwrap(), save);

let text = data::encode(EncodeFormat::Base64, b"hello");
assert_eq!(text, "aGVsbG8=");

let digest = data::hash(HashFunction::Sha256, b"abc");
println!("{}", data::encode(EncodeFormat::Hex, &digest));
```

Zlib, gzip, deflate and LZ4 compression, Base64 and hex encoding, MD5 and
the SHA family of hashes are supported. `data::pack` and `unpack` read and
write binary data described by format strings, in the style of Lua's
`string.pack`.

## Threads

The framework modules only work on the main thread, but `data`, `math` and
`thread` work anywhere. Start threads with `std::thread::spawn`. Rust's own
channels are often all you need;
{{ api(path="thread/struct.Channel.html", text="thread::Channel") }} adds
channels any thread can both send and receive on, and waiting until a
message is read:

```rust
use smitten::thread::Channel;

let jobs = Channel::new();
let results = Channel::new();
{
    let (jobs, results) = (jobs.clone(), results.clone());
    std::thread::spawn(move || {
        while let Some(n) = jobs.demand(None) {
            results.push(n * n);
        }
    });
}
jobs.push(3);
assert_eq!(results.demand(None), Some(9));
```
