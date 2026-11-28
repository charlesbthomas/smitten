//! An immediate-mode menu for title screens, pause menus and settings:
//! buttons, sliders and toggles in a column, worked with a keyboard, a
//! gamepad (through [`Actions`]) or the mouse.
//!
//! Each frame the menu is built again from the game's own state, and each
//! widget call answers whether it was used, so there is no widget tree to
//! keep in sync with the settings it shows:
//!
//! ```no_run
//! use smitten_kit::prelude::*;
//! use smitten_kit::ui::{Ui, UiActions, UiInput};
//!
//! #[derive(Clone, Copy, PartialEq, Eq, Hash)]
//! enum Menu {
//!     Up,
//!     Down,
//!     Left,
//!     Right,
//!     Confirm,
//!     Back,
//! }
//!
//! const NAV: UiActions<Menu> = UiActions {
//!     up: Menu::Up,
//!     down: Menu::Down,
//!     left: Menu::Left,
//!     right: Menu::Right,
//!     confirm: Menu::Confirm,
//!     back: Menu::Back,
//! };
//!
//! struct Settings {
//!     volume: f32,
//!     fullscreen: bool,
//! }
//!
//! fn settings_menu(ui: &mut Ui, actions: &Actions<Menu>, settings: &mut Settings) -> bool {
//!     ui.begin(
//!         Rect::from_position_size((40.0, 40.0), (300.0, 200.0)),
//!         UiInput::from_actions(actions, &NAV),
//!     );
//!     ui.label("Settings");
//!     ui.slider("Volume", &mut settings.volume, 0.0..=1.0, 0.1);
//!     ui.toggle("Fullscreen", &mut settings.fullscreen);
//!     let done = ui.button("Back") || ui.back();
//!     ui.end();
//!     ui.draw();
//!     done
//! }
//! ```
//!
//! Building the menu needs no window: [`Ui::widgets`] lists what was
//! built, where, and which has focus, so menu logic can be tested.

use std::ops::RangeInclusive;

use smitten::graphics::{self, Align, DrawMode};
use smitten::{Color, Vec2};

use crate::input::Actions;
use crate::math::Rect;

/// Which actions move through a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UiActions<A> {
    /// Focus the widget above.
    pub up: A,
    /// Focus the widget below.
    pub down: A,
    /// Turn a slider down, or flip a toggle.
    pub left: A,
    /// Turn a slider up, or flip a toggle.
    pub right: A,
    /// Press the focused button or flip the focused toggle.
    pub confirm: A,
    /// Leave the menu; read it with [`Ui::back`].
    pub back: A,
}

/// One frame's input to a menu.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct UiInput {
    /// Move focus up.
    pub up: bool,
    /// Move focus down.
    pub down: bool,
    /// Turn down or flip.
    pub left: bool,
    /// Turn up or flip.
    pub right: bool,
    /// Press the focused widget.
    pub confirm: bool,
    /// Leave the menu.
    pub back: bool,
    /// Where the mouse is, in the same space as the menu's area, or
    /// `None` to leave the mouse out.
    pub pointer: Option<Vec2>,
    /// The mouse button went down this frame.
    pub pointer_pressed: bool,
    /// The mouse button is held.
    pub pointer_down: bool,
}

impl UiInput {
    /// The actions pressed this frame.
    pub fn from_actions<A: Copy + Eq + std::hash::Hash>(
        actions: &Actions<A>,
        map: &UiActions<A>,
    ) -> UiInput {
        UiInput {
            up: actions.is_pressed(map.up),
            down: actions.is_pressed(map.down),
            left: actions.is_pressed(map.left),
            right: actions.is_pressed(map.right),
            confirm: actions.is_pressed(map.confirm),
            back: actions.is_pressed(map.back),
            ..UiInput::default()
        }
    }

    /// The same input with the mouse at `position`, its button `pressed`
    /// this frame and `down` now.
    #[must_use]
    pub fn with_pointer(mut self, position: impl Into<Vec2>, pressed: bool, down: bool) -> UiInput {
        self.pointer = Some(position.into());
        self.pointer_pressed = pressed;
        self.pointer_down = down;
        self
    }
}

/// The colors and sizes a menu is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiStyle {
    /// The height of each row.
    pub row_height: f32,
    /// The gap between rows.
    pub spacing: f32,
    /// Text.
    pub text: Color,
    /// Labels and unfocused slider tracks.
    pub dim: Color,
    /// The focused row's highlight and slider handle.
    pub accent: Color,
}

impl Default for UiStyle {
    fn default() -> Self {
        UiStyle {
            row_height: 32.0,
            spacing: 6.0,
            text: Color::WHITE,
            dim: Color::rgb(0.6, 0.62, 0.68),
            accent: Color::rgb(0.35, 0.7, 1.0),
        }
    }
}

/// What a built widget is.
#[derive(Clone, Debug, PartialEq)]
pub enum WidgetKind {
    /// Text that cannot be focused.
    Label,
    /// A button.
    Button,
    /// A slider at `fraction` (0.0 to 1.0) of its range.
    Slider {
        /// How far along the range the value is.
        fraction: f32,
        /// The value as shown.
        value: f32,
    },
    /// A toggle, on or off.
    Toggle {
        /// Whether it is on.
        on: bool,
    },
}

/// A widget built this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    /// Where it is.
    pub rect: Rect,
    /// Its text.
    pub label: String,
    /// What it is.
    pub kind: WidgetKind,
    /// Whether it has focus.
    pub focused: bool,
}

/// A menu, kept by the game between frames to remember the focus. See
/// the [module docs](self).
#[derive(Clone, Debug, Default)]
pub struct Ui {
    style: UiStyle,
    area: Rect,
    input: UiInput,
    cursor: f32,
    focus: usize,
    focusable: usize,
    focusable_last_frame: usize,
    last_pointer: Option<Vec2>,
    dragging: Option<usize>,
    widgets: Vec<Widget>,
}

impl Ui {
    /// A menu with the default style.
    pub fn new() -> Ui {
        Ui::default()
    }

    /// A menu drawn with `style`.
    pub fn with_style(style: UiStyle) -> Ui {
        Ui {
            style,
            ..Ui::default()
        }
    }

    /// The style.
    pub fn style_mut(&mut self) -> &mut UiStyle {
        &mut self.style
    }

    /// Starts this frame's menu in `area`, reading `input`. Focus moves
    /// up and down here, wrapping around.
    pub fn begin(&mut self, area: Rect, input: UiInput) {
        self.area = area;
        self.input = input;
        self.cursor = area.min.y;
        self.widgets.clear();
        self.focusable = 0;
        let count = self.focusable_last_frame;
        if count > 0 {
            self.focus = self.focus.min(count - 1);
            if input.down {
                self.focus = (self.focus + 1) % count;
            }
            if input.up {
                self.focus = (self.focus + count - 1) % count;
            }
        }
        if !input.pointer_down {
            self.dragging = None;
        }
    }

    /// Ends this frame's menu.
    pub fn end(&mut self) {
        self.focusable_last_frame = self.focusable;
        self.last_pointer = self.input.pointer;
    }

    /// Whether back was pressed this frame.
    pub fn back(&self) -> bool {
        self.input.back
    }

    /// Which focusable widget has focus, counting from 0 at the top.
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// Moves focus to the `index`th focusable widget, as a menu that
    /// opens on its second item does.
    pub fn set_focus(&mut self, index: usize) {
        self.focus = index;
    }

    /// The widgets built since [`begin`](Ui::begin).
    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }

    /// Leaves `height` pixels of space.
    pub fn space(&mut self, height: f32) {
        self.cursor += height;
    }

    fn row(&mut self) -> Rect {
        let rect = Rect::from_position_size(
            (self.area.min.x, self.cursor),
            (self.area.width(), self.style.row_height),
        );
        self.cursor += self.style.row_height + self.style.spacing;
        rect
    }

    /// Lays out a focusable row: returns its rect, index, whether it has
    /// focus, and whether it was clicked.
    fn focusable_row(&mut self) -> (Rect, usize, bool, bool) {
        let rect = self.row();
        let index = self.focusable;
        self.focusable += 1;
        let over = self.input.pointer.is_some_and(|p| rect.contains(p));
        if over && self.input.pointer != self.last_pointer {
            self.focus = index;
        }
        let clicked = over && self.input.pointer_pressed;
        if clicked {
            self.focus = index;
        }
        (rect, index, self.focus == index, clicked)
    }

    /// Text that cannot be focused.
    pub fn label(&mut self, text: impl Into<String>) {
        let rect = self.row();
        self.widgets.push(Widget {
            rect,
            label: text.into(),
            kind: WidgetKind::Label,
            focused: false,
        });
    }

    /// A button. True on the frame it is pressed.
    pub fn button(&mut self, text: impl Into<String>) -> bool {
        let (rect, _, focused, clicked) = self.focusable_row();
        self.widgets.push(Widget {
            rect,
            label: text.into(),
            kind: WidgetKind::Button,
            focused,
        });
        clicked || (focused && self.input.confirm)
    }

    /// A toggle for `value`. True on the frame it flips.
    pub fn toggle(&mut self, text: impl Into<String>, value: &mut bool) -> bool {
        let (rect, _, focused, clicked) = self.focusable_row();
        let input = self.input;
        let flip = clicked || (focused && (input.confirm || input.left || input.right));
        if flip {
            *value = !*value;
        }
        self.widgets.push(Widget {
            rect,
            label: text.into(),
            kind: WidgetKind::Toggle { on: *value },
            focused,
        });
        flip
    }

    /// The part of a row a slider's track takes: its right half.
    fn track(rect: Rect) -> Rect {
        let half = rect.width() / 2.0;
        Rect::new(
            (rect.min.x + half, rect.min.y),
            (rect.max.x - 56.0_f32.min(half / 2.0), rect.max.y),
        )
    }

    /// A slider for `value` within `range`, moved by `step` with left and
    /// right, or dragged with the mouse. True on the frames it changes.
    pub fn slider(
        &mut self,
        text: impl Into<String>,
        value: &mut f32,
        range: RangeInclusive<f32>,
        step: f32,
    ) -> bool {
        let (lo, hi) = (*range.start(), *range.end());
        let (rect, index, focused, clicked) = self.focusable_row();
        let before = *value;
        let input = self.input;
        if focused {
            if input.left {
                *value -= step;
            }
            if input.right {
                *value += step;
            }
        }
        let track = Ui::track(rect);
        if clicked {
            self.dragging = Some(index);
        }
        if self.dragging == Some(index)
            && let Some(pointer) = input.pointer
            && track.width() > 0.0
        {
            let t = ((pointer.x - track.min.x) / track.width()).clamp(0.0, 1.0);
            let mut v = lo + t * (hi - lo);
            if step > 0.0 {
                v = lo + ((v - lo) / step).round() * step;
            }
            *value = v;
        }
        *value = value.clamp(lo.min(hi), hi.max(lo));
        let fraction = if hi != lo {
            (*value - lo) / (hi - lo)
        } else {
            0.0
        };
        self.widgets.push(Widget {
            rect,
            label: text.into(),
            kind: WidgetKind::Slider {
                fraction,
                value: *value,
            },
            focused,
        });
        *value != before
    }

    /// Draws the widgets built this frame with the current font.
    pub fn draw(&self) {
        let style = self.style;
        let before = graphics::color();
        let text_height = graphics::font().height();
        for widget in &self.widgets {
            let rect = widget.rect;
            let text_y = rect.min.y + (rect.height() - text_height) / 2.0;
            if widget.focused {
                graphics::set_color(style.accent.with_alpha(0.22));
                graphics::rectangle(DrawMode::Fill, rect.min, rect.size());
                graphics::set_color(style.accent);
                graphics::rectangle(DrawMode::Fill, rect.min, (4.0, rect.height()));
            }
            let pad = 12.0;
            match &widget.kind {
                WidgetKind::Label => {
                    graphics::set_color(style.dim);
                    graphics::print(widget.label.as_str(), (rect.min.x, text_y));
                }
                WidgetKind::Button => {
                    graphics::set_color(style.text);
                    graphics::printf(
                        widget.label.as_str(),
                        (rect.min.x, text_y),
                        rect.width(),
                        Align::Center,
                    );
                }
                WidgetKind::Toggle { on } => {
                    graphics::set_color(style.text);
                    graphics::print(widget.label.as_str(), (rect.min.x + pad, text_y));
                    let size = rect.height() * 0.5;
                    let corner = Vec2::new(rect.max.x - pad - size, rect.center().y - size / 2.0);
                    graphics::set_color(if *on { style.accent } else { style.dim });
                    let mode = if *on { DrawMode::Fill } else { DrawMode::Line };
                    graphics::rectangle(mode, corner, (size, size));
                }
                WidgetKind::Slider { fraction, value } => {
                    graphics::set_color(style.text);
                    graphics::print(widget.label.as_str(), (rect.min.x + pad, text_y));
                    let track = Ui::track(rect);
                    let y = track.center().y;
                    graphics::set_color(style.dim);
                    graphics::rectangle(
                        DrawMode::Fill,
                        (track.min.x, y - 2.0),
                        (track.width(), 4.0),
                    );
                    graphics::set_color(style.accent);
                    let filled = track.width() * fraction;
                    graphics::rectangle(DrawMode::Fill, (track.min.x, y - 2.0), (filled, 4.0));
                    let handle = Vec2::new(track.min.x + filled - 5.0, y - 9.0);
                    graphics::rectangle(DrawMode::Fill, handle, (10.0, 18.0));
                    graphics::set_color(style.text);
                    let shown = if value.fract() == 0.0 {
                        format!("{value:.0}")
                    } else {
                        format!("{value:.2}")
                    };
                    graphics::printf(
                        shown.as_str(),
                        (track.max.x, text_y),
                        rect.max.x - track.max.x - pad,
                        Align::Right,
                    );
                }
            }
        }
        graphics::set_color(before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_position_size((0.0, 0.0), (200.0, 400.0))
    }

    fn frame(ui: &mut Ui, input: UiInput, volume: &mut f32, on: &mut bool) -> (bool, bool) {
        ui.begin(area(), input);
        ui.label("Menu");
        let play = ui.button("Play");
        ui.slider("Volume", volume, 0.0..=1.0, 0.25);
        ui.toggle("Music", on);
        let quit = ui.button("Quit");
        ui.end();
        (play, quit)
    }

    #[test]
    fn keys_move_focus_and_press() {
        let mut ui = Ui::new();
        let (mut volume, mut on) = (0.5, false);
        frame(&mut ui, UiInput::default(), &mut volume, &mut on);
        assert_eq!(ui.focus(), 0);
        assert!(!ui.widgets()[0].focused); // labels never focus
        let confirm = UiInput {
            confirm: true,
            ..UiInput::default()
        };
        assert_eq!(frame(&mut ui, confirm, &mut volume, &mut on), (true, false));

        let up = UiInput {
            up: true,
            ..UiInput::default()
        };
        frame(&mut ui, up, &mut volume, &mut on); // wraps to Quit
        assert_eq!(ui.focus(), 3);
        assert!(ui.widgets()[4].focused);

        ui.set_focus(1);
        let right = UiInput {
            right: true,
            ..UiInput::default()
        };
        frame(&mut ui, right, &mut volume, &mut on);
        assert_eq!(volume, 0.75);
        frame(&mut ui, right, &mut volume, &mut on);
        frame(&mut ui, right, &mut volume, &mut on);
        assert_eq!(volume, 1.0); // held at the top
        ui.set_focus(2);
        frame(&mut ui, right, &mut volume, &mut on);
        assert!(on);
    }

    #[test]
    fn the_mouse_focuses_clicks_and_drags() {
        let mut ui = Ui::new();
        let (mut volume, mut on) = (0.0, false);
        frame(&mut ui, UiInput::default(), &mut volume, &mut on);
        // Rows are 32 high with 6 between: Quit is the fifth, at y 152.
        let over_quit = UiInput::default().with_pointer((100.0, 160.0), false, false);
        frame(&mut ui, over_quit, &mut volume, &mut on);
        assert_eq!(ui.focus(), 3);
        let click = UiInput::default().with_pointer((100.0, 160.0), true, true);
        assert_eq!(frame(&mut ui, click, &mut volume, &mut on), (false, true));

        // The volume row is at y 76; its track spans x 100 to 150.
        let grab = UiInput::default().with_pointer((110.0, 90.0), true, true);
        frame(&mut ui, grab, &mut volume, &mut on);
        assert_eq!(volume, 0.25);
        let drag = UiInput::default().with_pointer((200.0, 300.0), false, true);
        frame(&mut ui, drag, &mut volume, &mut on);
        assert_eq!(volume, 1.0); // dragging keeps hold off the row
        let release = UiInput::default().with_pointer((100.0, 90.0), false, false);
        frame(&mut ui, release, &mut volume, &mut on);
        assert_eq!(volume, 1.0);
    }
}
