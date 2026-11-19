//! Keyboard state: which keys are held, the layout, key repeat and text
//! input.

use crate::Key;
use crate::context;
use glam::Vec2;

/// Returns whether `key` is currently held.
pub fn is_down(key: Key) -> bool {
    context::with(|ctx| ctx.input.keys.contains(&key))
}

/// Returns whether any of `keys` is currently held.
pub fn is_any_down(keys: &[Key]) -> bool {
    context::with(|ctx| keys.iter().any(|key| ctx.input.keys.contains(key)))
}

/// Reads two sets of keys as one direction: -1.0 if any of `negative`
/// is held, 1.0 if any of `positive` is, and 0.0 for neither or both.
///
/// ```no_run
/// # use smitten::{Key, keyboard};
/// let steer = keyboard::axis(&[Key::Left, Key::A], &[Key::Right, Key::D]);
/// ```
pub fn axis(negative: &[Key], positive: &[Key]) -> f32 {
    f32::from(i8::from(is_any_down(positive)) - i8::from(is_any_down(negative)))
}

/// The direction the arrow keys and W, A, S, D are asking for, with
/// right and down positive as on screen. It is never longer than 1, so
/// walking diagonally is no faster than walking straight.
pub fn direction() -> Vec2 {
    let x = axis(&[Key::Left, Key::A], &[Key::Right, Key::D]);
    let y = axis(&[Key::Up, Key::W], &[Key::Down, Key::S]);
    crate::input::limit_length(x, y).into()
}

/// Which key the player's keyboard layout puts at the position `key` names.
///
/// [`Key`] always names a position, by what a US layout has there, which is
/// often called a scancode. That is what movement keys want: WASD stays in
/// the same place on a French keyboard. Use this to show the player what is
/// printed on the key, so on an AZERTY layout `layout_key(Key::Q)` is
/// `Some(Key::A)`.
///
/// Returns `None` if the layout puts a key there that Smitten has no
/// name for, such as `é`.
pub fn layout_key(key: Key) -> Option<Key> {
    context::with(|ctx| ctx.backend.layout_key(key))
}

/// Where the player's keyboard layout puts `key`, named by the key a US
/// layout has in that position. The reverse of [`layout_key`].
///
/// Returns `None` if the layout has no such key.
pub fn layout_position(key: Key) -> Option<Key> {
    context::with(|ctx| ctx.backend.layout_position(key))
}

/// Returns whether the key labelled `key` on the player's layout is held,
/// wherever the layout puts it. [`is_down`] goes by the key's position
/// instead.
///
/// Use it for shortcuts named after a letter, such as Ctrl+Z, which
/// should follow the letter rather than the position.
pub fn is_layout_down(key: Key) -> bool {
    layout_position(key).is_some_and(is_down)
}

/// Turns repeated [`Game::key_pressed`](crate::Game::key_pressed) calls on
/// or off while a key is held.
///
/// Repeats are off when the game starts, so a held key calls `key_pressed`
/// once. They never affect [`Game::text_input`](crate::Game::text_input),
/// which always repeats as the system does.
pub fn set_key_repeat(enabled: bool) {
    context::with(|ctx| ctx.input.key_repeat = enabled);
}

/// Returns whether held keys repeat [`Game::key_pressed`](crate::Game::key_pressed).
pub fn has_key_repeat() -> bool {
    context::with(|ctx| ctx.input.key_repeat)
}

/// Starts or stops reporting typed text to
/// [`Game::text_input`](crate::Game::text_input). On a phone or tablet this
/// shows or hides the screen keyboard.
///
/// Text input is on when the game starts on desktop systems. A game can
/// turn it off until a text box is selected.
pub fn set_text_input(enabled: bool) {
    context::with(|ctx| {
        if let Err(message) = ctx.backend.set_text_input(enabled) {
            eprintln!("smitten: could not change text input: {message}");
        }
    });
}

/// Starts text input and says where the text box is, in the same units as
/// [`mouse::position`](crate::mouse::position). Input methods put their
/// window beside it, and screen keyboards try not to cover it.
pub fn set_text_input_area(position: impl Into<Vec2>, size: impl Into<Vec2>) {
    let Vec2 { x, y } = position.into();
    let Vec2 {
        x: width,
        y: height,
    } = size.into();
    context::with(|ctx| {
        let view = ctx.gfx.screen.view();
        let area = [
            view.x + x * view.scale,
            view.y + y * view.scale,
            width * view.scale,
            height * view.scale,
        ];
        let result = ctx
            .backend
            .set_text_input_area(Some(area))
            .and_then(|()| ctx.backend.set_text_input(true));
        if let Err(message) = result {
            eprintln!("smitten: could not change text input: {message}");
        }
    });
}

/// Returns whether typed text is being reported.
pub fn has_text_input() -> bool {
    context::with(|ctx| ctx.backend.text_input_active())
}

/// Returns whether the platform has an on-screen keyboard.
pub fn has_screen_keyboard() -> bool {
    context::with(|ctx| ctx.backend.has_screen_keyboard())
}
