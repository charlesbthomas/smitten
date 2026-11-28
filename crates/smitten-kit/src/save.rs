//! Save slots: named, versioned saves in the game's save directory.
//!
//! Each slot is one JSON file holding the game's own serde type and the
//! version it was saved at. A save is written to a temporary file first
//! and then moved over the old one, so a crash or power cut mid-save
//! leaves the last good save in place. A slot that is missing, broken, or
//! from another version loads as `None`.
//!
//! ```no_run
//! use serde::{Deserialize, Serialize};
//! use smitten_kit::save::SaveSlots;
//!
//! #[derive(Serialize, Deserialize, Default)]
//! struct Progress {
//!     level: u32,
//! }
//!
//! # fn main() -> std::io::Result<()> {
//! // Version 1 of the save format, in <save directory>/saves/.
//! let slots = SaveSlots::new("saves", 1)?;
//! let progress: Progress = slots.load("slot1").unwrap_or_default();
//! slots.save("slot1", &progress)?;
//! for name in slots.list() {
//!     println!("{name}");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! The text format alone is in [`smitten_kit_core::save`](crate::save::data),
//! for a rules crate or a test with no window.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use smitten_kit_core::save as data;
pub use smitten_kit_core::save::Value;

/// A folder of save slots. See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveSlots {
    folder: PathBuf,
    version: u32,
}

/// Whether `name` can name a slot: letters, digits, `-` and `_`, at
/// most 64 of them, so it is a safe file name everywhere.
pub fn is_valid_slot_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn bad_name(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("`{name}` is not a slot name (use letters, digits, - and _)"),
    )
}

impl SaveSlots {
    /// Slots in `folder` inside smitten's save directory, for saves at
    /// format `version`. Fails if the save directory cannot be found.
    pub fn new(folder: impl AsRef<Path>, version: u32) -> io::Result<SaveSlots> {
        let root = smitten::filesystem::save_directory()?;
        Ok(SaveSlots::in_directory(root.join(folder), version))
    }

    /// Slots in `folder` anywhere on disk, as a test or a tool uses.
    pub fn in_directory(folder: impl Into<PathBuf>, version: u32) -> SaveSlots {
        SaveSlots {
            folder: folder.into(),
            version,
        }
    }

    /// The folder holding the slots.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The save format version.
    pub fn version(&self) -> u32 {
        self.version
    }

    fn path(&self, slot: &str) -> io::Result<PathBuf> {
        if !is_valid_slot_name(slot) {
            return Err(bad_name(slot));
        }
        Ok(self.folder.join(format!("{slot}.json")))
    }

    /// Writes `data` to `slot`, replacing what was there only once the
    /// new save is written in full.
    pub fn save<T: Serialize + ?Sized>(&self, slot: &str, data: &T) -> io::Result<()> {
        self.save_text(slot, &data::encode(self.version, data))
    }

    fn save_text(&self, slot: &str, text: &str) -> io::Result<()> {
        let path = self.path(slot)?;
        std::fs::create_dir_all(&self.folder)?;
        let temporary = self.folder.join(format!("{slot}.json.tmp"));
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, &path)
    }

    /// The data in `slot`, or `None` when it is empty, broken, or from
    /// another version.
    pub fn load<T: DeserializeOwned>(&self, slot: &str) -> Option<T> {
        data::decode(
            self.version,
            &std::fs::read_to_string(self.path(slot).ok()?).ok()?,
        )
    }

    /// Like [`load`](SaveSlots::load), carrying a save from an older
    /// version forward with `migrate`, as
    /// [`decode_migrating`](data::decode_migrating) does.
    pub fn load_migrating<T: DeserializeOwned>(
        &self,
        slot: &str,
        migrate: impl FnMut(u32, Value) -> Option<Value>,
    ) -> Option<T> {
        let text = std::fs::read_to_string(self.path(slot).ok()?).ok()?;
        data::decode_migrating(self.version, &text, migrate)
    }

    /// Whether `slot` holds a file (not checking it loads).
    pub fn exists(&self, slot: &str) -> bool {
        self.path(slot).is_ok_and(|path| path.is_file())
    }

    /// Empties `slot`. Emptying an empty slot is fine.
    pub fn delete(&self, slot: &str) -> io::Result<()> {
        match std::fs::remove_file(self.path(slot)?) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    /// The names of the slots holding files, sorted.
    pub fn list(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.folder) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                let slot = name.strip_suffix(".json")?;
                is_valid_slot_name(slot).then(|| slot.to_owned())
            })
            .collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("smitten-kit-save-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn save_load_list_delete() {
        let folder = scratch("slots");
        let slots = SaveSlots::in_directory(&folder, 3);
        assert_eq!(slots.load::<u32>("a"), None);
        assert!(slots.list().is_empty());

        slots.save("b", &20_u32).unwrap();
        slots.save("a", &10_u32).unwrap();
        slots.save("a", &11_u32).unwrap();
        assert_eq!(slots.load::<u32>("a"), Some(11));
        assert_eq!(slots.list(), ["a", "b"]);
        assert!(!folder.join("a.json.tmp").exists());

        // Another version of the game does not read it.
        assert_eq!(SaveSlots::in_directory(&folder, 4).load::<u32>("a"), None);
        // A broken file reads as empty.
        std::fs::write(folder.join("b.json"), "{ not json").unwrap();
        assert_eq!(slots.load::<u32>("b"), None);
        assert!(slots.exists("b"));

        slots.delete("b").unwrap();
        slots.delete("b").unwrap();
        assert_eq!(slots.list(), ["a"]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn slot_names_stay_in_the_folder() {
        let slots = SaveSlots::in_directory(scratch("names"), 1);
        assert!(slots.save("../escape", &1).is_err());
        assert!(slots.save("", &1).is_err());
        assert_eq!(slots.load::<i32>("a/b"), None);
        assert!(is_valid_slot_name("autosave_2-b"));
    }

    #[test]
    fn migrates_old_slots() {
        let folder = scratch("migrate");
        SaveSlots::in_directory(&folder, 1).save("s", &5).unwrap();
        let v2 = SaveSlots::in_directory(&folder, 2);
        let got: Option<i64> = v2.load_migrating("s", |_, d| Some((d.as_i64()? + 1).into()));
        assert_eq!(got, Some(6));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
