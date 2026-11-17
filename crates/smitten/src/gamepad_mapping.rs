//! Reading and editing gamepad mapping strings, in SDL's
//! `GUID,name,a:b0,leftx:a0,dpup:h0.1,...` format.

use smitten_core::{GamepadAxis, GamepadButton, GamepadInput, JoystickHat, JoystickInput};

/// The name a mapping uses for a gamepad input.
pub(crate) fn field_name(input: GamepadInput) -> &'static str {
    match input {
        GamepadInput::Button(button) => match button {
            GamepadButton::A => "a",
            GamepadButton::B => "b",
            GamepadButton::X => "x",
            GamepadButton::Y => "y",
            GamepadButton::Back => "back",
            GamepadButton::Guide => "guide",
            GamepadButton::Start => "start",
            GamepadButton::LeftStick => "leftstick",
            GamepadButton::RightStick => "rightstick",
            GamepadButton::LeftShoulder => "leftshoulder",
            GamepadButton::RightShoulder => "rightshoulder",
            GamepadButton::DpadUp => "dpup",
            GamepadButton::DpadDown => "dpdown",
            GamepadButton::DpadLeft => "dpleft",
            GamepadButton::DpadRight => "dpright",
        },
        GamepadInput::Axis(axis) => match axis {
            GamepadAxis::LeftX => "leftx",
            GamepadAxis::LeftY => "lefty",
            GamepadAxis::RightX => "rightx",
            GamepadAxis::RightY => "righty",
            GamepadAxis::TriggerLeft => "lefttrigger",
            GamepadAxis::TriggerRight => "righttrigger",
        },
    }
}

/// How a mapping writes a joystick input.
pub(crate) fn input_value(input: JoystickInput) -> String {
    match input {
        JoystickInput::Button(index) => format!("b{index}"),
        JoystickInput::Axis(index) => format!("a{index}"),
        JoystickInput::Hat(index, direction) => format!("h{index}.{}", direction.bits()),
    }
}

/// Reads a joystick input as a mapping writes it. Half-axis and inverted
/// markers (`+a1`, `-a1`, `a1~`) are read as the whole axis.
pub(crate) fn parse_input(value: &str) -> Option<JoystickInput> {
    let value = value
        .trim()
        .trim_start_matches(['+', '-'])
        .trim_end_matches('~');
    let (kind, rest) = value.split_at_checked(1)?;
    match kind {
        "b" => rest.parse().ok().map(JoystickInput::Button),
        "a" => rest.parse().ok().map(JoystickInput::Axis),
        "h" => {
            let (hat, bits) = rest.split_once('.')?;
            Some(JoystickInput::Hat(
                hat.parse().ok()?,
                JoystickHat::from_bits(bits.parse().ok()?),
            ))
        }
        _ => None,
    }
}

/// The GUID a mapping line is for.
pub(crate) fn guid_of(mapping: &str) -> Option<&str> {
    let guid = mapping.split(',').next()?.trim();
    (!guid.is_empty() && !guid.starts_with('#')).then_some(guid)
}

/// The joystick input a mapping binds to `input`.
pub(crate) fn lookup(mapping: &str, input: GamepadInput) -> Option<JoystickInput> {
    let name = field_name(input);
    mapping
        .split(',')
        .skip(2)
        .filter_map(|field| field.split_once(':'))
        // A half axis such as `+leftx` still names the axis.
        .find(|(key, _)| key.trim().trim_start_matches(['+', '-']) == name)
        .and_then(|(_, value)| parse_input(value))
}

/// Binds `input` to `from` in `mapping`, replacing what it was bound to.
pub(crate) fn with_binding(mapping: &str, input: GamepadInput, from: JoystickInput) -> String {
    let name = field_name(input);
    let mut fields: Vec<String> = mapping
        .trim()
        .trim_end_matches(',')
        .split(',')
        .map(str::to_string)
        .collect();
    let binding = format!("{name}:{}", input_value(from));
    let existing = fields.iter().skip(2).position(|field| {
        field
            .split_once(':')
            .is_some_and(|(key, _)| key.trim().trim_start_matches(['+', '-']) == name)
    });
    match existing {
        Some(index) => fields[index + 2] = binding,
        None => fields.push(binding),
    }
    let mut mapping = fields.join(",");
    mapping.push(',');
    mapping
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAD: &str = "03000000de280000ff11000001000000,Steam Virtual Gamepad,a:b0,b:b1,leftx:a0,lefty:a1,dpup:h0.1,-righty:a4~,platform:Linux,";

    #[test]
    fn bindings_are_read_from_a_mapping() {
        let button = |button| GamepadInput::Button(button);
        assert_eq!(
            lookup(PAD, button(GamepadButton::B)),
            Some(JoystickInput::Button(1))
        );
        assert_eq!(
            lookup(PAD, GamepadInput::Axis(GamepadAxis::LeftY)),
            Some(JoystickInput::Axis(1))
        );
        assert_eq!(
            lookup(PAD, button(GamepadButton::DpadUp)),
            Some(JoystickInput::Hat(0, JoystickHat::Up))
        );
        assert_eq!(
            lookup(PAD, GamepadInput::Axis(GamepadAxis::RightY)),
            Some(JoystickInput::Axis(4))
        );
        assert_eq!(lookup(PAD, button(GamepadButton::Guide)), None);
    }

    #[test]
    fn a_binding_replaces_the_old_one_or_is_added() {
        let changed = with_binding(
            PAD,
            GamepadInput::Button(GamepadButton::A),
            JoystickInput::Button(3),
        );
        assert!(
            changed
                .starts_with("03000000de280000ff11000001000000,Steam Virtual Gamepad,a:b3,b:b1,")
        );
        assert!(changed.ends_with("platform:Linux,"));

        let added = with_binding(
            "abc,Pad,",
            GamepadInput::Button(GamepadButton::DpadLeft),
            JoystickInput::Hat(1, JoystickHat::Left),
        );
        assert_eq!(added, "abc,Pad,dpleft:h1.8,");
    }

    #[test]
    fn the_guid_is_the_first_field() {
        assert_eq!(guid_of(PAD), Some("03000000de280000ff11000001000000"));
        assert_eq!(guid_of("# a comment"), None);
        assert_eq!(guid_of(""), None);
    }
}
