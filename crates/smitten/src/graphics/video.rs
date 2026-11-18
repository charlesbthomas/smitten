//! Playing a video and drawing it, the equivalent of LÖVE's `Video`.

use std::cell::{Cell, Ref, RefCell};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use smitten_core::Filter;

use super::Image;
use crate::audio::{Source, SourceType};
use crate::context;
use crate::timer;
use crate::video::{VideoError, VideoStream};

/// A video that plays and can be drawn like an image.
///
/// Load one with [`Video::load`](crate::graphics::Video::load), call
/// [`Video::play`], and draw it each frame with
/// [`draw_video`](crate::graphics::draw_video). The picture moves on as
/// it is drawn. A video with a sound track plays it through
/// [`Video::source`], and the picture follows the sound so the two stay
/// together.
///
/// Like a LÖVE video, it stops on its last frame at the end. Call
/// [`Video::rewind`] to play it again.
pub struct Video {
    stream: RefCell<VideoStream>,
    image: Image,
    source: Option<Source>,
    playing: Cell<bool>,
    /// The position when the clock last started or was set.
    position: Cell<f64>,
    /// [`timer::time`] when it started, while playing.
    started: Cell<Option<f64>>,
}

impl Video {
    /// Creates a video from the contents of an Ogg Theora file. With
    /// `audio`, its sound track, if it has one, plays along with it.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>, audio: bool) -> Result<Video, VideoError> {
        let bytes = bytes.into();
        let stream = VideoStream::from_bytes(bytes.clone())?;
        let (width, height) = stream.dimensions();
        let image = Image::from_rgba(width, height, stream.pixels())
            .map_err(|error| VideoError::Upload(error.to_string()))?;
        // A video with no sound track is fine; it just has no source.
        let source = audio
            .then(|| Source::from_bytes(bytes, SourceType::Stream).ok())
            .flatten();
        Ok(Video {
            stream: RefCell::new(stream),
            image,
            source,
            playing: Cell::new(false),
            position: Cell::new(0.0),
            started: Cell::new(None),
        })
    }

    /// Starts or resumes playing.
    pub fn play(&self) {
        if self.playing.get() {
            return;
        }
        if self.position.get() >= self.duration() {
            // At the end there is nothing to play until a rewind.
            return;
        }
        self.playing.set(true);
        self.started.set(Some(timer::time()));
        if let Some(source) = &self.source {
            source.seek(self.position.get());
            source.play();
        }
    }

    /// Stops playing, keeping the position.
    pub fn pause(&self) {
        self.position.set(self.tell());
        self.playing.set(false);
        self.started.set(None);
        if let Some(source) = &self.source {
            source.pause();
        }
    }

    /// Whether the video is playing. It stops by itself at the end.
    pub fn is_playing(&self) -> bool {
        self.update_clock();
        self.playing.get()
    }

    /// Moves to `seconds` from the start.
    pub fn seek(&self, seconds: f64) {
        let seconds = seconds.clamp(0.0, self.duration());
        self.position.set(seconds);
        if self.playing.get() {
            self.started.set(Some(timer::time()));
        }
        if let Some(source) = &self.source {
            source.seek(seconds);
        }
    }

    /// Goes back to the start.
    pub fn rewind(&self) {
        self.seek(0.0);
    }

    /// The position in seconds from the start.
    pub fn tell(&self) -> f64 {
        let elapsed = self
            .started
            .get()
            .map_or(0.0, |started| timer::time() - started);
        (self.position.get() + elapsed).min(self.duration())
    }

    /// The length in seconds: of the picture, or of the sound if that
    /// goes on longer, holding the last frame.
    pub fn duration(&self) -> f64 {
        let picture = self.stream.borrow().duration();
        let sound = self.source.as_ref().and_then(Source::duration);
        sound.map_or(picture, |sound| sound.max(picture))
    }

    /// The sound track, if the video has one and was loaded with it.
    /// Change its volume or position it as any other source, but play,
    /// pause and seek the video instead of the source.
    pub fn source(&self) -> Option<&Source> {
        self.source.as_ref()
    }

    /// Gives the video a different sound track, or none, the equivalent
    /// of `Video:setSource`. The new source jumps to the video's position,
    /// and plays if the video is playing, and from then on the picture
    /// follows it. The old source is paused and handed back, so it can be
    /// played on its own.
    ///
    /// Any source works, such as a dub in another language. The video's
    /// length becomes the longer of the picture and the new sound.
    pub fn set_source(&mut self, source: Option<Source>) -> Option<Source> {
        let position = self.tell();
        let old = std::mem::replace(&mut self.source, source);
        if let Some(old) = &old {
            old.pause();
        }
        let position = position.min(self.duration());
        self.position.set(position);
        if self.playing.get() {
            self.started.set(Some(timer::time()));
        }
        if let Some(source) = &self.source {
            source.seek(position);
            if self.playing.get() {
                source.play();
            } else {
                source.pause();
            }
        }
        old
    }

    /// The decoder the picture comes from, the equivalent of
    /// `Video:getStream`. It reads the file's size, frame rate and length,
    /// and where the picture is up to. Play, pause and seek the video
    /// instead of the stream.
    ///
    /// Let go of it before drawing the video or asking for its image,
    /// which move the stream on and panic while it is held.
    pub fn stream(&self) -> Ref<'_, VideoStream> {
        self.stream.borrow()
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.image.height()
    }

    /// Size in pixels as `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        self.image.dimensions()
    }

    /// How the picture is sampled when scaled or rotated.
    pub fn filter(&self) -> Filter {
        self.image.filter()
    }

    /// Sets how the picture is sampled when scaled or rotated.
    pub fn set_filter(&self, filter: Filter) {
        self.image.set_filter(filter);
    }

    /// The current picture, as an image that can be drawn with a
    /// [`Quad`](super::Quad), a shader or a mesh. It is brought up to date
    /// first. The image changes as the video plays, so there is no need
    /// to fetch it again.
    pub fn image(&self) -> &Image {
        self.update();
        &self.image
    }

    /// Follows the sound when there is some, and notices the end.
    fn update_clock(&self) {
        if !self.playing.get() {
            return;
        }
        if let Some(source) = &self.source
            && source.is_playing()
        {
            // The sound plays at its own pace, so the picture keeps up
            // with it rather than with the game's clock.
            self.position.set(source.tell());
            self.started.set(Some(timer::time()));
        }
        if self.tell() >= self.duration() {
            self.position.set(self.duration());
            self.playing.set(false);
            self.started.set(None);
        }
    }

    /// Shows the frame for the current position.
    pub(crate) fn update(&self) {
        self.update_clock();
        let position = self.tell();
        let mut stream = self.stream.borrow_mut();
        if stream.seek(position) {
            let (width, height) = stream.dimensions();
            let result = context::with(|ctx| {
                ctx.backend.update_texture(
                    self.image.texture(),
                    0,
                    0,
                    width,
                    height,
                    stream.pixels(),
                )
            });
            if let Err(message) = result {
                eprintln!("smitten: could not show a video frame: {message}");
            }
        }
    }
}

impl fmt::Debug for Video {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Video")
            .field("dimensions", &self.dimensions())
            .field("playing", &self.playing.get())
            .field("position", &self.tell())
            .field("has_sound", &self.source.is_some())
            .finish()
    }
}

impl Video {
    /// Loads a video and its sound track, the equivalent of
    /// `love.graphics.newVideo`. Videos are Ogg Theora files; see
    /// [`crate::video`] for how to make one.
    ///
    /// A relative path is looked up as [`crate::filesystem`] describes.
    pub fn load(path: impl AsRef<Path>) -> Result<Video, VideoError> {
        Video::from_bytes(crate::video::read(path.as_ref())?, true)
    }
}
