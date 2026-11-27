//! A stack of screens: title, play, pause, game over.
//!
//! Each screen is a [`Scene`]. A [`SceneStack`] runs them: the top scene
//! gets the input, overlays such as a pause menu draw over the scene
//! beneath, and moving between scenes is a [`Transition`] that a scene
//! returns from [`Scene::update`] or [`Scene::event`]. A scene never
//! touches the stack directly, so it can never pull itself out from under
//! its own feet.
//!
//! Scenes share one context, `C`, a type of the game's own for whatever
//! outlives a single screen: fonts, sounds, settings, the best score, the
//! input bindings.
//!
//! Settings that belong to one screen, such as a wider letterbox for a
//! map or the music that plays under it, go in [`Scene::enter`] and
//! [`Scene::resume`]: those run whenever the scene comes (back) on top.
//!
//! A scene below the top can move the stack too, as a level that keeps
//! playing under a popup ends the game when the player dies: its move
//! first closes the scenes above it. When several scenes ask on the same
//! frame, the highest one wins.
//!
//! [`run`] opens the window and runs a stack with no [`smitten::Game`]
//! impl needed. A game that keeps its own `Game` can own a `SceneStack`
//! and call it from its callbacks instead.
//!
//! ```no_run
//! use smitten_kit::prelude::*;
//!
//! struct Shared {
//!     best: u32,
//! }
//!
//! impl scene::Context for Shared {}
//!
//! struct Title;
//! struct Playing {
//!     score: u32,
//! }
//! struct Paused;
//!
//! impl Scene<Shared> for Title {
//!     fn event(&mut self, _: &mut Shared, event: &Event) -> Transition<Shared> {
//!         match event {
//!             Event::KeyPressed { key: Key::Return, .. } => {
//!                 Transition::switch(Playing { score: 0 }).fade(0.3, Color::BLACK)
//!             }
//!             Event::KeyPressed { key: Key::Escape, .. } => Transition::Quit,
//!             _ => Transition::None,
//!         }
//!     }
//! }
//!
//! impl Scene<Shared> for Playing {
//!     fn update(&mut self, ctx: &mut Shared, _dt: f32) -> Transition<Shared> {
//!         ctx.best = ctx.best.max(self.score);
//!         Transition::None
//!     }
//!
//!     fn event(&mut self, _: &mut Shared, event: &Event) -> Transition<Shared> {
//!         match event {
//!             Event::KeyPressed { key: Key::Escape, .. } => Transition::push(Paused),
//!             _ => Transition::None,
//!         }
//!     }
//! }
//!
//! impl Scene<Shared> for Paused {
//!     // The game stays on screen, frozen, under the menu.
//!     fn is_overlay(&self) -> bool {
//!         true
//!     }
//!
//!     fn event(&mut self, _: &mut Shared, event: &Event) -> Transition<Shared> {
//!         match event {
//!             Event::KeyPressed { key: Key::Escape, .. } => Transition::Pop,
//!             Event::KeyPressed { key: Key::Q, .. } => Transition::reset(Title),
//!             _ => Transition::None,
//!         }
//!     }
//! }
//!
//! fn main() {
//!     scene::run(Config::default(), || Ok((Shared { best: 0 }, Title)));
//! }
//! ```

use std::fmt;

use smitten::event::Event;
use smitten::{Color, Config, graphics};

/// One screen of the game. Every method has a default that does nothing.
pub trait Scene<C>: 'static {
    /// A name for logs, debug overlays and tests. Empty by default.
    fn name(&self) -> &str {
        ""
    }

    /// The scene was put on the stack.
    fn enter(&mut self, _ctx: &mut C) {}

    /// The scene is leaving the stack, popped or switched away.
    fn exit(&mut self, _ctx: &mut C) {}

    /// Another scene was pushed on top of this one.
    fn pause(&mut self, _ctx: &mut C) {}

    /// The scene above this one was popped, and this one is on top again.
    fn resume(&mut self, _ctx: &mut C) {}

    /// Called every frame while the scene is on top (or under a scene
    /// whose [`updates_below`](Scene::updates_below) is true). A move
    /// returned from a scene below the top first closes the scenes above
    /// it.
    fn update(&mut self, _ctx: &mut C, _dt: f32) -> Transition<C> {
        Transition::None
    }

    /// Called every frame while the scene can be seen.
    fn draw(&mut self, _ctx: &mut C) {}

    /// Called with each input and window event while the scene is on top.
    /// Return [`Transition::Pass`] to hand the event to the scene below.
    fn event(&mut self, _ctx: &mut C, _event: &Event) -> Transition<C> {
        Transition::None
    }

    /// Whether the scene below should be drawn first, as under a pause
    /// menu or a dialog.
    fn is_overlay(&self) -> bool {
        false
    }

    /// Whether the scene below should keep updating, as under a popup
    /// that does not stop play.
    fn updates_below(&self) -> bool {
        false
    }
}

/// A move between scenes, returned from [`Scene::update`] and
/// [`Scene::event`].
pub enum Transition<C> {
    /// Stay on this scene.
    None,
    /// From [`Scene::event`]: not handled here, so offer the event to the
    /// scene below. Elsewhere it is the same as `None`.
    Pass,
    /// Put a scene on top of this one, which pauses.
    Push(Box<dyn Scene<C>>),
    /// Take this scene off; the one below resumes. Popping the last scene
    /// quits.
    Pop,
    /// Replace this scene.
    Switch(Box<dyn Scene<C>>),
    /// Take every scene off and start over from this one.
    Reset(Box<dyn Scene<C>>),
    /// End the game.
    Quit,
    /// Fade out to `color` over `seconds`, make the move, then fade back
    /// in over `seconds`.
    Fade {
        /// The move to make once the screen is covered.
        then: Box<Transition<C>>,
        /// Seconds for each half of the fade.
        seconds: f32,
        /// The color faded through.
        color: Color,
    },
}

impl<C: 'static> Transition<C> {
    /// Puts `scene` on top of this one.
    pub fn push(scene: impl Scene<C>) -> Self {
        Transition::Push(Box::new(scene))
    }

    /// Replaces this scene with `scene`.
    pub fn switch(scene: impl Scene<C>) -> Self {
        Transition::Switch(Box::new(scene))
    }

    /// Takes every scene off and starts over from `scene`.
    pub fn reset(scene: impl Scene<C>) -> Self {
        Transition::Reset(Box::new(scene))
    }

    /// The same move, made behind a fade through `color`.
    pub fn fade(self, seconds: f32, color: Color) -> Self {
        Transition::Fade {
            then: Box::new(self),
            seconds,
            color,
        }
    }

    /// Whether this is `None` or `Pass`.
    pub fn is_none(&self) -> bool {
        matches!(self, Transition::None | Transition::Pass)
    }
}

impl<C> fmt::Debug for Transition<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Transition::None => f.write_str("None"),
            Transition::Pass => f.write_str("Pass"),
            Transition::Push(_) => f.write_str("Push(..)"),
            Transition::Pop => f.write_str("Pop"),
            Transition::Switch(_) => f.write_str("Switch(..)"),
            Transition::Reset(_) => f.write_str("Reset(..)"),
            Transition::Quit => f.write_str("Quit"),
            Transition::Fade {
                then,
                seconds,
                color,
            } => f
                .debug_struct("Fade")
                .field("then", then)
                .field("seconds", seconds)
                .field("color", color)
                .finish(),
        }
    }
}

struct Fade<C> {
    then: Option<Transition<C>>,
    /// The stack height the move applies at: scenes above are closed
    /// first.
    from: usize,
    seconds: f32,
    color: Color,
    /// From 0.0 (clear) to 1.0 (covered) and back.
    elapsed: f32,
}

impl<C> Fade<C> {
    fn cover(&self) -> f32 {
        if self.seconds <= 0.0 {
            return 0.0;
        }
        let t = self.elapsed / self.seconds;
        if t <= 1.0 { t } else { (2.0 - t).max(0.0) }
    }
}

/// The scenes being run, bottom to top. See the [module docs](self).
pub struct SceneStack<C> {
    scenes: Vec<Box<dyn Scene<C>>>,
    fade: Option<Fade<C>>,
    quit: bool,
}

impl<C> fmt::Debug for SceneStack<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneStack")
            .field("len", &self.scenes.len())
            .field("fading", &self.fade.is_some())
            .field("quit", &self.quit)
            .finish()
    }
}

impl<C: 'static> SceneStack<C> {
    /// A stack holding `first`, which enters at once.
    pub fn new(first: impl Scene<C>, ctx: &mut C) -> Self {
        let mut stack = SceneStack {
            scenes: Vec::new(),
            fade: None,
            quit: false,
        };
        stack.apply(Transition::push(first), ctx);
        stack
    }

    /// How many scenes are on the stack.
    pub fn len(&self) -> usize {
        self.scenes.len()
    }

    /// Whether the stack is empty, as after the last scene popped.
    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty()
    }

    /// Whether a scene asked to quit, or the last scene popped.
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Whether a fade is playing. Input is held back until it ends.
    pub fn is_fading(&self) -> bool {
        self.fade.is_some()
    }

    /// The scene on top, the one getting input.
    pub fn top(&self) -> Option<&dyn Scene<C>> {
        self.scenes.last().map(|scene| &**scene)
    }

    /// The scenes, bottom to top.
    pub fn iter(&self) -> impl Iterator<Item = &dyn Scene<C>> {
        self.scenes.iter().map(|scene| &**scene)
    }

    /// The scenes' [names](Scene::name), bottom to top, as
    /// `"title > play > pause"` in a debug overlay.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.scenes.iter().map(|scene| scene.name())
    }

    /// Makes `transition` now, as if the top scene had returned it.
    pub fn apply(&mut self, transition: Transition<C>, ctx: &mut C) {
        let top = self.scenes.len().saturating_sub(1);
        self.apply_at(top, transition, ctx);
    }

    /// Makes `transition` as if the scene at `index` had returned it:
    /// the scenes above it are closed first and it resumes, unless the
    /// move is one that does nothing.
    fn apply_at(&mut self, index: usize, transition: Transition<C>, ctx: &mut C) {
        if transition.is_none() {
            return;
        }
        if !matches!(transition, Transition::Fade { .. } | Transition::Quit)
            && index + 1 < self.scenes.len()
        {
            while self.scenes.len() > index + 1 {
                if let Some(mut top) = self.scenes.pop() {
                    top.exit(ctx);
                }
            }
            if let Some(top) = self.scenes.last_mut() {
                top.resume(ctx);
            }
        }
        match transition {
            Transition::None | Transition::Pass => {}
            Transition::Push(mut scene) => {
                if let Some(top) = self.scenes.last_mut() {
                    top.pause(ctx);
                }
                scene.enter(ctx);
                self.scenes.push(scene);
            }
            Transition::Pop => {
                if let Some(mut top) = self.scenes.pop() {
                    top.exit(ctx);
                }
                match self.scenes.last_mut() {
                    Some(top) => top.resume(ctx),
                    None => self.quit = true,
                }
            }
            Transition::Switch(mut scene) => {
                if let Some(mut top) = self.scenes.pop() {
                    top.exit(ctx);
                }
                scene.enter(ctx);
                self.scenes.push(scene);
            }
            Transition::Reset(mut scene) => {
                while let Some(mut top) = self.scenes.pop() {
                    top.exit(ctx);
                }
                scene.enter(ctx);
                self.scenes.push(scene);
            }
            Transition::Quit => self.quit = true,
            Transition::Fade {
                then,
                seconds,
                color,
            } => {
                if seconds <= 0.0 {
                    self.apply_at(index, *then, ctx);
                } else if self.fade.is_none() {
                    self.fade = Some(Fade {
                        then: Some(*then),
                        from: index,
                        seconds,
                        color,
                        elapsed: 0.0,
                    });
                }
                // A fade asked for during another fade is dropped: the
                // screen is already on its way somewhere.
            }
        }
    }

    /// Updates the top scene, and the ones below it that it lets update,
    /// then makes the move the highest of them asked for.
    ///
    /// While a fade plays, scenes keep updating so they stay animated,
    /// but the moves they return are ignored.
    pub fn update(&mut self, ctx: &mut C, dt: f32) {
        if self.scenes.is_empty() {
            return;
        }
        let mut lowest = self.scenes.len() - 1;
        while lowest > 0 && self.scenes[lowest].updates_below() {
            lowest -= 1;
        }
        let mut wanted = (0, Transition::None);
        for (index, scene) in self.scenes.iter_mut().enumerate().skip(lowest) {
            let transition = scene.update(ctx, dt);
            // The highest scene that asks to move wins.
            if !transition.is_none() {
                wanted = (index, transition);
            }
        }

        if let Some(fade) = &mut self.fade {
            let was_covered = fade.elapsed >= fade.seconds;
            fade.elapsed += dt;
            if !was_covered
                && fade.elapsed >= fade.seconds
                && let Some(then) = fade.then.take()
            {
                let from = fade.from.min(self.scenes.len().saturating_sub(1));
                self.apply_at(from, then, ctx);
            }
            if let Some(fade) = &self.fade
                && fade.elapsed >= fade.seconds * 2.0
            {
                self.fade = None;
            }
            return;
        }
        self.apply_at(wanted.0, wanted.1, ctx);
    }

    /// Hands `event` to the top scene, and down the stack for as long as
    /// scenes return [`Transition::Pass`]. Held back during a fade.
    pub fn event(&mut self, ctx: &mut C, event: &Event) {
        if self.fade.is_some() {
            return;
        }
        let mut wanted = (0, Transition::None);
        for (index, scene) in self.scenes.iter_mut().enumerate().rev() {
            match scene.event(ctx, event) {
                Transition::Pass => continue,
                transition => {
                    wanted = (index, transition);
                    break;
                }
            }
        }
        self.apply_at(wanted.0, wanted.1, ctx);
    }

    /// Draws the top scene, after the scenes it overlays, then the fade.
    pub fn draw(&mut self, ctx: &mut C) {
        if let Some(top) = self.scenes.len().checked_sub(1) {
            let mut lowest = top;
            while lowest > 0 && self.scenes[lowest].is_overlay() {
                lowest -= 1;
            }
            for scene in &mut self.scenes[lowest..] {
                scene.draw(ctx);
            }
        }
        if let Some(fade) = &self.fade {
            let cover = fade.cover();
            if cover > 0.0 {
                let (w, h) = graphics::dimensions();
                graphics::push_all();
                graphics::origin();
                graphics::set_color(fade.color.with_alpha(fade.color.a * cover));
                graphics::rectangle(graphics::DrawMode::Fill, (0.0, 0.0), (w as f32, h as f32));
                graphics::pop();
            }
        }
    }
}

/// The shared state of a game run with [`run`]. Each hook is called
/// before (or for `draw`, after) the scenes get the same call, for work
/// that belongs to the whole game: feeding
/// [`Actions`](crate::input::Actions), updating music, drawing a debug
/// overlay.
///
/// ```
/// use smitten::event::Event;
/// use smitten_kit::input::Actions;
/// use smitten_kit::scene::Context;
///
/// # #[derive(Clone, Copy, PartialEq, Eq, Hash)] enum Action { Jump }
/// struct Shared {
///     actions: Actions<Action>,
/// }
///
/// impl Context for Shared {
///     fn event(&mut self, event: &Event) {
///         self.actions.handle(event);
///     }
///
///     fn update(&mut self, dt: f32) {
///         self.actions.update(dt);
///     }
/// }
/// ```
pub trait Context: 'static {
    /// Called with each event before the scenes see it.
    fn event(&mut self, _event: &Event) {}

    /// Called each frame before the scenes update.
    fn update(&mut self, _dt: f32) {}

    /// Called each frame after the scenes draw, in screen space.
    fn draw(&mut self) {}

    /// The color the screen is cleared to each frame.
    fn background(&self) -> Color {
        Color::BLACK
    }

    /// Called when the player closes the window. Return true to keep
    /// running, as when asking to save first.
    fn quit(&mut self) -> bool {
        false
    }
}

/// Opens the window and runs a scene stack until it quits.
///
/// `load` runs once the window exists and builds the shared context and
/// the first scene. An error from it is shown on smitten's error screen,
/// as an error from [`smitten::Game::load`] is.
///
/// Each frame, the context and then the scenes get the frame's events,
/// the context and then the scenes update, and the scenes and then the
/// context draw.
pub fn run<C: Context, S: Scene<C>>(
    config: Config,
    load: impl FnOnce() -> smitten::Result<(C, S)>,
) {
    smitten::run_loop(config, move |main| {
        let (mut ctx, first) = match load() {
            Ok(loaded) => loaded,
            Err(error) => std::panic::panic_any(format!("{error:#}")),
        };
        let mut stack = SceneStack::new(first, &mut ctx);
        // Leave the loading time out of the first frame.
        main.step();
        loop {
            for event in main.poll() {
                if matches!(event, Event::Quit) {
                    if ctx.quit() {
                        continue;
                    }
                    return;
                }
                ctx.event(&event);
                stack.event(&mut ctx, &event);
            }
            if stack.should_quit() {
                return;
            }
            let dt = main.step();
            ctx.update(dt);
            stack.update(&mut ctx, dt);
            if stack.should_quit() {
                return;
            }
            if main.is_active() {
                main.clear(ctx.background());
                stack.draw(&mut ctx);
                ctx.draw();
                main.present();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records what each scene was told, in order.
    #[derive(Default)]
    struct Log(Vec<String>);

    struct Named {
        name: &'static str,
        overlay: bool,
        passes: bool,
        below: bool,
        on_update: Option<fn() -> Transition<Log>>,
    }

    fn named(name: &'static str) -> Named {
        Named {
            name,
            overlay: false,
            passes: false,
            below: false,
            on_update: None,
        }
    }

    impl Scene<Log> for Named {
        fn name(&self) -> &str {
            self.name
        }
        fn updates_below(&self) -> bool {
            self.below
        }
        fn enter(&mut self, ctx: &mut Log) {
            ctx.0.push(format!("{} enter", self.name));
        }
        fn exit(&mut self, ctx: &mut Log) {
            ctx.0.push(format!("{} exit", self.name));
        }
        fn pause(&mut self, ctx: &mut Log) {
            ctx.0.push(format!("{} pause", self.name));
        }
        fn resume(&mut self, ctx: &mut Log) {
            ctx.0.push(format!("{} resume", self.name));
        }
        fn update(&mut self, ctx: &mut Log, _dt: f32) -> Transition<Log> {
            ctx.0.push(format!("{} update", self.name));
            self.on_update.map_or(Transition::None, |f| f())
        }
        fn event(&mut self, ctx: &mut Log, _event: &Event) -> Transition<Log> {
            ctx.0.push(format!("{} event", self.name));
            if self.passes {
                Transition::Pass
            } else {
                Transition::None
            }
        }
        fn is_overlay(&self) -> bool {
            self.overlay
        }
    }

    fn take(log: &mut Log) -> Vec<String> {
        std::mem::take(&mut log.0)
    }

    #[test]
    fn push_pop_switch_reset_call_the_hooks() {
        let mut log = Log::default();
        let mut stack = SceneStack::new(named("title"), &mut log);
        assert_eq!(take(&mut log), ["title enter"]);

        stack.apply(Transition::switch(named("play")), &mut log);
        assert_eq!(take(&mut log), ["title exit", "play enter"]);

        stack.apply(Transition::push(named("pause")), &mut log);
        assert_eq!(take(&mut log), ["play pause", "pause enter"]);
        assert_eq!(stack.len(), 2);

        stack.apply(Transition::Pop, &mut log);
        assert_eq!(take(&mut log), ["pause exit", "play resume"]);

        stack.apply(Transition::push(named("menu")), &mut log);
        take(&mut log);
        stack.apply(Transition::reset(named("title")), &mut log);
        assert_eq!(take(&mut log), ["menu exit", "play exit", "title enter"]);
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn popping_the_last_scene_quits() {
        let mut log = Log::default();
        let mut stack = SceneStack::new(named("only"), &mut log);
        assert!(!stack.should_quit());
        stack.apply(Transition::Pop, &mut log);
        assert!(stack.should_quit());
        assert!(stack.is_empty());
    }

    #[test]
    fn only_the_top_updates_and_gets_events() {
        let mut log = Log::default();
        let mut stack = SceneStack::new(named("play"), &mut log);
        stack.apply(Transition::push(named("pause")), &mut log);
        take(&mut log);
        stack.update(&mut log, 0.1);
        stack.event(&mut log, &Event::Focus(true));
        assert_eq!(take(&mut log), ["pause update", "pause event"]);
    }

    #[test]
    fn pass_hands_the_event_down() {
        let mut log = Log::default();
        let mut stack = SceneStack::new(named("play"), &mut log);
        stack.apply(
            Transition::push(Named {
                passes: true,
                ..named("hud")
            }),
            &mut log,
        );
        take(&mut log);
        stack.event(&mut log, &Event::Focus(true));
        assert_eq!(take(&mut log), ["hud event", "play event"]);
    }

    #[test]
    fn transitions_from_update_apply() {
        let mut log = Log::default();
        let mut stack = SceneStack::new(
            Named {
                on_update: Some(|| Transition::switch(named("next"))),
                ..named("first")
            },
            &mut log,
        );
        take(&mut log);
        stack.update(&mut log, 0.1);
        assert_eq!(take(&mut log), ["first update", "first exit", "next enter"]);
    }

    #[test]
    fn a_move_from_below_closes_the_scenes_above() {
        let mut log = Log::default();
        let mut stack = SceneStack::new(
            Named {
                on_update: Some(|| Transition::switch(named("over"))),
                ..named("play")
            },
            &mut log,
        );
        stack.apply(
            Transition::push(Named {
                below: true,
                ..named("popup")
            }),
            &mut log,
        );
        assert_eq!(stack.names().collect::<Vec<_>>(), ["play", "popup"]);
        take(&mut log);
        stack.update(&mut log, 0.1);
        assert_eq!(
            take(&mut log),
            [
                "play update",
                "popup update",
                "popup exit",
                "play resume",
                "play exit",
                "over enter"
            ]
        );
        assert_eq!(stack.top().map(|s| s.name()), Some("over"));
    }

    #[test]
    fn the_highest_move_wins() {
        let mut log = Log::default();
        let mut stack = SceneStack::new(
            Named {
                on_update: Some(|| Transition::Quit),
                ..named("play")
            },
            &mut log,
        );
        stack.apply(
            Transition::push(Named {
                below: true,
                on_update: Some(|| Transition::Pop),
                ..named("popup")
            }),
            &mut log,
        );
        stack.update(&mut log, 0.1);
        assert!(!stack.should_quit());
        assert_eq!(stack.names().collect::<Vec<_>>(), ["play"]);
    }

    #[test]
    fn a_fade_switches_once_covered_and_holds_input() {
        let mut log = Log::default();
        let mut stack = SceneStack::new(named("title"), &mut log);
        stack.apply(
            Transition::switch(named("play")).fade(0.5, Color::BLACK),
            &mut log,
        );
        take(&mut log);

        stack.update(&mut log, 0.25);
        stack.event(&mut log, &Event::Focus(true));
        assert_eq!(take(&mut log), ["title update"], "no switch yet, no input");

        stack.update(&mut log, 0.25);
        assert_eq!(take(&mut log), ["title update", "title exit", "play enter"]);
        assert!(stack.is_fading(), "fading back in");

        stack.update(&mut log, 0.5);
        assert!(!stack.is_fading());
        take(&mut log);
        stack.event(&mut log, &Event::Focus(true));
        assert_eq!(take(&mut log), ["play event"]);
    }
}
