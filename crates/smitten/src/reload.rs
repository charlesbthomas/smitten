//! Reloads images, shaders, fonts and sounds when their files change on
//! disk.
//!
//! While a game is being developed, saving an image in a paint program or
//! a shader in an editor should show up in the running game. Assets loaded
//! from a file register themselves here, and the game loop asks a few
//! times a second whether any of their files have changed.

use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::SystemTime;

use crate::audio::Sound;
use crate::context;
use crate::graphics::reloading::{
    FontInner, ImageInner, ShaderInner, reload_font, reload_image, reload_shader,
};

/// Seconds between looks at the files. Often enough to feel immediate,
/// rarely enough to cost nothing.
const CHECK_INTERVAL: f64 = 0.25;

/// Something that was loaded from a file. The asset is not kept alive by
/// being watched.
pub(crate) enum Asset {
    Image(Weak<ImageInner>),
    Shader(Weak<ShaderInner>),
    Font(Weak<FontInner>),
    Sound(Sound),
}

/// A strong handle held while an asset is reloaded.
enum Live {
    Image(Rc<ImageInner>),
    Shader(Rc<ShaderInner>),
    Font(Rc<FontInner>),
    Sound(Sound),
}

impl Asset {
    fn upgrade(&self) -> Option<Live> {
        match self {
            Asset::Image(weak) => weak.upgrade().map(Live::Image),
            Asset::Shader(weak) => weak.upgrade().map(Live::Shader),
            Asset::Font(weak) => weak.upgrade().map(Live::Font),
            Asset::Sound(sound) => sound.in_use().then(|| Live::Sound(sound.clone())),
        }
    }
}

struct Watched {
    path: PathBuf,
    /// The file's modification time when it was last loaded.
    modified: Option<SystemTime>,
    asset: Asset,
}

#[derive(Default)]
pub(crate) struct ReloadState {
    enabled: bool,
    watched: Vec<Watched>,
    next_check: f64,
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

impl ReloadState {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    /// Remembers that `asset` came from the file at `path`.
    pub(crate) fn watch(&mut self, path: PathBuf, asset: Asset) {
        // Watching costs a little memory per asset, so it is skipped
        // entirely in games that have reloading off.
        if self.enabled {
            let modified = modified(&path);
            self.watched.push(Watched {
                path,
                modified,
                asset,
            });
        }
    }

    /// Returns whether it is time to look at the files again.
    fn due(&mut self, now: f64) -> bool {
        if !self.enabled || now < self.next_check {
            return false;
        }
        self.next_check = now + CHECK_INTERVAL;
        true
    }
}

/// Reloads every watched asset whose file has changed. Called once a
/// frame by the game loop, between frames.
pub(crate) fn check() {
    // Assets are kept alive until the framework state is released: if the
    // game dropped its last handle meanwhile, freeing the asset needs that
    // state too.
    let mut alive = Vec::new();
    context::with(|ctx| {
        let now = ctx.backend.ticks_ns() as f64 / 1e9;
        if !ctx.reload.due(now) {
            return;
        }
        let context::Context {
            reload,
            backend,
            gfx,
            ..
        } = ctx;
        reload.watched.retain_mut(|watched| {
            // Forget assets the game no longer uses.
            let Some(live) = watched.asset.upgrade() else {
                return false;
            };
            let current = modified(&watched.path);
            if current.is_some() && current != watched.modified {
                watched.modified = current;
                let result = std::fs::read(&watched.path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| match &live {
                        Live::Image(image) => reload_image(image, backend.as_mut(), &bytes),
                        Live::Shader(shader) => {
                            reload_shader(shader, backend.as_mut(), gfx, &bytes)
                        }
                        Live::Font(font) => reload_font(font, backend.as_mut(), &bytes),
                        Live::Sound(sound) => sound.reload(bytes),
                    });
                // A file caught half-saved, or a shader with a mistake in
                // it, keeps the version that was working.
                match result {
                    Ok(()) => println!("smitten: reloaded {}", watched.path.display()),
                    Err(message) => eprintln!(
                        "smitten: could not reload {}:\n{message}",
                        watched.path.display()
                    ),
                }
            }
            alive.push(live);
            true
        });
    });
    drop(alive);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_looked_at_a_few_times_a_second() {
        let mut state = ReloadState::new(true);
        assert!(state.due(10.0));
        assert!(!state.due(10.1));
        assert!(state.due(10.0 + CHECK_INTERVAL));
    }

    #[test]
    fn nothing_is_watched_or_checked_when_reloading_is_off() {
        let mut state = ReloadState::new(false);
        state.watch(PathBuf::from("heart.png"), Asset::Image(Weak::new()));
        assert!(state.watched.is_empty());
        assert!(!state.due(10.0));
    }

    #[test]
    fn a_watched_file_remembers_when_it_was_last_changed() {
        let mut state = ReloadState::new(true);
        // This source file certainly exists and has a modification time.
        let path = PathBuf::from(file!());
        let path = if path.exists() {
            path
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/reload.rs")
        };
        state.watch(path.clone(), Asset::Image(Weak::new()));
        assert_eq!(state.watched[0].modified, modified(&path));
        assert!(state.watched[0].modified.is_some());

        state.watch(PathBuf::from("no/such/file.png"), Asset::Image(Weak::new()));
        assert_eq!(state.watched[1].modified, None);
    }
}
