/// A physical key, named by its position on a US layout.
///
/// Most variants name a position, which is what LÖVE calls a scancode.
/// The symbol keys from [`Key::Exclaim`] to [`Key::Underscore`] are the
/// exception: no US key sits where they do, so they are only ever
/// returned by `keyboard::layout_key`
/// on layouts that have them as a key of their own, such as `!` on a
/// French keyboard, and are never held or pressed by position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Up,
    Down,
    Left,
    Right,
    Space,
    Return,
    Escape,
    Tab,
    Backspace,
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Minus,
    Equals,
    LeftBracket,
    RightBracket,
    Backslash,
    Semicolon,
    Apostrophe,
    Grave,
    Comma,
    Period,
    Slash,
    CapsLock,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    /// The left Windows, Command or Super key.
    LeftGui,
    /// The right Windows, Command or Super key.
    RightGui,
    /// The keypad's 0 key.
    Kp0,
    /// The keypad's 1 key.
    Kp1,
    /// The keypad's 2 key.
    Kp2,
    /// The keypad's 3 key.
    Kp3,
    /// The keypad's 4 key.
    Kp4,
    /// The keypad's 5 key.
    Kp5,
    /// The keypad's 6 key.
    Kp6,
    /// The keypad's 7 key.
    Kp7,
    /// The keypad's 8 key.
    Kp8,
    /// The keypad's 9 key.
    Kp9,
    /// The keypad's `.` key, LÖVE's `kp.`.
    KpPeriod,
    /// The keypad's `,` key, which some keypads have instead of `.`.
    KpComma,
    /// The keypad's `/` key.
    KpDivide,
    /// The keypad's `*` key.
    KpMultiply,
    /// The keypad's `-` key.
    KpMinus,
    /// The keypad's `+` key.
    KpPlus,
    /// The keypad's Enter key.
    KpEnter,
    /// The keypad's `=` key, found on Mac keyboards.
    KpEquals,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    /// Num Lock, or Clear on a Mac keypad.
    NumLock,
    ScrollLock,
    /// Print Screen, LÖVE's `printscreen` scancode and `print` key.
    PrintScreen,
    SysReq,
    Pause,
    /// The context menu key, between the right Windows key and right
    /// Ctrl on PC keyboards.
    Application,
    Menu,
    Clear,
    Help,
    Power,
    /// The mode switch key on some international layouts.
    Mode,
    Execute,
    Select,
    Stop,
    Again,
    Undo,
    Cut,
    Copy,
    Paste,
    Find,
    Cancel,
    Mute,
    VolumeUp,
    VolumeDown,
    /// The media play or pause key, LÖVE's `audioplay`.
    MediaPlayPause,
    /// LÖVE's `audiostop`.
    MediaStop,
    /// LÖVE's `audionext`.
    MediaNextTrack,
    /// LÖVE's `audioprev`.
    MediaPreviousTrack,
    MediaSelect,
    MediaEject,
    /// The browser search key, LÖVE's `appsearch`.
    AppSearch,
    /// The browser home key, LÖVE's `apphome`.
    AppHome,
    /// The browser back key, LÖVE's `appback`.
    AppBack,
    /// The browser forward key, LÖVE's `appforward`.
    AppForward,
    /// The browser refresh key, LÖVE's `apprefresh`.
    AppRefresh,
    /// The browser bookmarks key, LÖVE's `appbookmarks`.
    AppBookmarks,
    /// The extra key beside the left Shift on ISO keyboards, `<>` on a
    /// German one.
    NonUsBackslash,
    /// The extra key beside Return on ISO keyboards, `#~` on a UK one.
    NonUsHash,
    /// `\` and `_` on a Japanese keyboard.
    International1,
    /// Katakana and Hiragana on a Japanese keyboard.
    International2,
    /// Yen on a Japanese keyboard.
    International3,
    /// Henkan on a Japanese keyboard.
    International4,
    /// Muhenkan on a Japanese keyboard.
    International5,
    International6,
    International7,
    International8,
    International9,
    /// Hangul and English on a Korean keyboard.
    Lang1,
    /// Hanja on a Korean keyboard.
    Lang2,
    /// Katakana on a Japanese keyboard.
    Lang3,
    /// Hiragana on a Japanese keyboard.
    Lang4,
    /// Zenkaku and Hankaku on a Japanese keyboard.
    Lang5,
    Lang6,
    Lang7,
    Lang8,
    Lang9,
    /// `!`, a key of its own on some layouts. Only ever a layout key.
    Exclaim,
    /// `"`, a key of its own on some layouts. Only ever a layout key.
    DoubleQuote,
    /// `#`, a key of its own on some layouts. Only ever a layout key.
    Hash,
    /// `$`, a key of its own on some layouts. Only ever a layout key.
    Dollar,
    /// `&`, a key of its own on some layouts. Only ever a layout key.
    Ampersand,
    /// `(`, a key of its own on some layouts. Only ever a layout key.
    LeftParen,
    /// `)`, a key of its own on some layouts. Only ever a layout key.
    RightParen,
    /// `*`, a key of its own on some layouts. Only ever a layout key.
    Asterisk,
    /// `+`, a key of its own on some layouts. Only ever a layout key.
    Plus,
    /// `:`, a key of its own on some layouts. Only ever a layout key.
    Colon,
    /// `<`, a key of its own on some layouts. Only ever a layout key.
    Less,
    /// `>`, a key of its own on some layouts. Only ever a layout key.
    Greater,
    /// `?`, a key of its own on some layouts. Only ever a layout key.
    Question,
    /// `@`, a key of its own on some layouts. Only ever a layout key.
    At,
    /// `^`, a key of its own on some layouts. Only ever a layout key.
    Caret,
    /// `_`, a key of its own on some layouts. Only ever a layout key.
    Underscore,
}
