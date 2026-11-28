//! Save data with a version number, as JSON text.
//!
//! [`encode`] wraps the game's own serde type with the version it was
//! saved at; [`decode`] gives it back only when the text is whole and
//! from the same version, and `None` otherwise, so a broken or old save
//! starts a new game instead of crashing. A game that wants to carry old
//! saves forward reads them with [`decode_migrating`].
//!
//! `smitten-kit`'s `save` module writes these to save slots on disk.
//!
//! ```
//! use serde::{Deserialize, Serialize};
//! use smitten_kit_core::save::{decode, encode};
//!
//! #[derive(Serialize, Deserialize, Debug, PartialEq)]
//! struct Progress {
//!     level: u32,
//!     coins: u32,
//! }
//!
//! let text = encode(2, &Progress { level: 3, coins: 40 });
//! assert_eq!(decode::<Progress>(2, &text), Some(Progress { level: 3, coins: 40 }));
//! assert_eq!(decode::<Progress>(3, &text), None); // saved by an older build
//! assert_eq!(decode::<Progress>(2, "{\"version\": 2, \"da"), None); // cut short
//! ```

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use serde_json::Value;

/// `data` as text, marked with `version`.
///
/// # Panics
///
/// When `data` cannot be written as JSON, which only happens for a type
/// whose `Serialize` impl fails or a map with keys that are not strings.
pub fn encode<T: Serialize + ?Sized>(version: u32, data: &T) -> String {
    let wrapped = serde_json::json!({
        "version": version,
        "data": serde_json::to_value(data).expect("save data writes as JSON"),
    });
    serde_json::to_string_pretty(&wrapped).expect("JSON values always write")
}

/// The version and raw data of saved text, or `None` when it is not
/// text [`encode`] wrote.
pub fn read(text: &str) -> Option<(u32, Value)> {
    let mut value: Value = serde_json::from_str(text).ok()?;
    let version = u32::try_from(value.get("version")?.as_u64()?).ok()?;
    let data = value.get_mut("data")?.take();
    Some((version, data))
}

/// The data saved in `text` if it was saved at `version`, or `None` when
/// it is broken, from another version, or not a `T`.
pub fn decode<T: DeserializeOwned>(version: u32, text: &str) -> Option<T> {
    let (saved, data) = read(text)?;
    if saved != version {
        return None;
    }
    serde_json::from_value(data).ok()
}

/// Like [`decode`], but a save from an older version is passed to
/// `migrate` with its version, to be turned into the current shape one
/// version at a time; `migrate` returns `None` for a version it cannot
/// carry forward. A save from a newer version gives `None`.
///
/// ```
/// use serde::Deserialize;
/// use smitten_kit_core::save::{Value, decode_migrating};
///
/// #[derive(Deserialize, Debug, PartialEq)]
/// struct Progress {
///     level: u32,
///     coins: u32,
/// }
///
/// // Version 1 had no coins.
/// let old = r#"{ "version": 1, "data": { "level": 4 } }"#;
/// let progress: Option<Progress> = decode_migrating(2, old, |version, mut data: Value| {
///     match version {
///         1 => data["coins"] = 0.into(),
///         _ => return None,
///     }
///     Some(data)
/// });
/// assert_eq!(progress, Some(Progress { level: 4, coins: 0 }));
/// ```
pub fn decode_migrating<T: DeserializeOwned>(
    version: u32,
    text: &str,
    mut migrate: impl FnMut(u32, Value) -> Option<Value>,
) -> Option<T> {
    let (mut saved, mut data) = read(text)?;
    if saved > version {
        return None;
    }
    while saved < version {
        data = migrate(saved, data)?;
        saved += 1;
    }
    serde_json::from_value(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_what_it_wrote() {
        let text = encode(7, &vec![1, 2, 3]);
        assert_eq!(read(&text).map(|(v, _)| v), Some(7));
        assert_eq!(decode::<Vec<i32>>(7, &text), Some(vec![1, 2, 3]));
        assert_eq!(decode::<String>(7, &text), None);
        assert_eq!(decode::<Vec<i32>>(7, "[1, 2, 3]"), None);
        assert_eq!(decode::<Vec<i32>>(7, ""), None);
    }

    #[test]
    fn newer_saves_are_not_migrated() {
        let text = encode(5, &1);
        let mut called = false;
        let got: Option<i32> = decode_migrating(4, &text, |_, d| {
            called = true;
            Some(d)
        });
        assert_eq!(got, None);
        assert!(!called);
        // Two steps forward, one at a time.
        let text = encode(1, &10);
        let mut steps = Vec::new();
        let got: Option<i64> = decode_migrating(3, &text, |v, d| {
            steps.push(v);
            Some((d.as_i64()? * 2).into())
        });
        assert_eq!(got, Some(40));
        assert_eq!(steps, [1, 2]);
    }
}
