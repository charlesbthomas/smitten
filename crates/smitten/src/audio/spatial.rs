//! Where sounds are, and how that changes what the listener hears: quieter
//! with distance, panned toward the side they are on, and shifted in pitch
//! when they move, as OpenAL does for LÖVE.

use std::f32::consts::{FRAC_PI_2, PI};

/// How a sound gets quieter with distance, the equivalent of LÖVE's
/// `love.audio.setDistanceModel`.
///
/// Each model is the OpenAL formula of the same name. `reference` and
/// `max` are a source's attenuation distances and `rolloff` its rolloff.
/// The clamped models treat anything nearer than `reference` as being at
/// `reference` and anything farther than `max` as being at `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DistanceModel {
    /// Distance does not change the volume.
    None,
    /// `reference / (reference + rolloff * (distance - reference))`.
    Inverse,
    /// The default, as in LÖVE.
    #[default]
    InverseClamped,
    /// Falls in a straight line from full volume at `reference` to silent
    /// at `max`, when the rolloff is 1.0.
    Linear,
    LinearClamped,
    /// `(distance / reference) ^ -rolloff`.
    Exponent,
    ExponentClamped,
}

pub(crate) type Vector = [f32; 3];

fn sub(a: Vector, b: Vector) -> Vector {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vector, b: Vector) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: Vector) -> f32 {
    dot(a, a).sqrt()
}

/// The speed of sound in metres per second, which OpenAL assumes when
/// working out how much a moving sound changes in pitch.
const SPEED_OF_SOUND: f32 = 343.3;

/// The one listener every positioned sound is heard by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Listener {
    pub position: Vector,
    pub velocity: Vector,
    /// The direction the listener faces.
    pub forward: Vector,
    /// The direction of the top of the listener's head.
    pub up: Vector,
    pub model: DistanceModel,
    pub doppler_scale: f32,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            velocity: [0.0; 3],
            // OpenAL's defaults, which put positive x on the right.
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            model: DistanceModel::default(),
            doppler_scale: 1.0,
        }
    }
}

/// Where one source is and how it carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    pub position: Vector,
    pub velocity: Vector,
    /// The way the source faces. Zero means it sounds the same in every
    /// direction.
    pub direction: Vector,
    /// Whether `position` is measured from the listener rather than from
    /// the world's origin.
    pub relative: bool,
    pub reference_distance: f32,
    pub max_distance: f32,
    pub rolloff: f32,
    /// The full widths of the cone, in radians, inside which the source is
    /// at full volume and outside which it is at `cone_outer_volume`.
    pub cone_inner: f32,
    pub cone_outer: f32,
    pub cone_outer_volume: f32,
    pub min_volume: f32,
    pub max_volume: f32,
    /// How strongly air muffles the sound with distance, from 0.0 (not at
    /// all) to 10.0.
    pub air_absorption: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            velocity: [0.0; 3],
            direction: [0.0; 3],
            relative: false,
            reference_distance: 1.0,
            max_distance: f32::MAX,
            rolloff: 1.0,
            cone_inner: 2.0 * PI,
            cone_outer: 2.0 * PI,
            cone_outer_volume: 0.0,
            min_volume: 0.0,
            max_volume: 1.0,
            air_absorption: 0.0,
        }
    }
}

/// How much of the high frequencies is left per unit of distance per unit
/// of air absorption: OpenAL's `AL_AIR_ABSORPTION_FACTOR`, about 0.05 dB
/// lost per metre.
const AIR_HIGH_GAIN_PER_UNIT: f32 = 0.99426;

/// What a listener hears of a source at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Heard {
    /// The volume of the left and right speakers.
    pub gains: [f32; 2],
    /// How much faster or slower the sound plays because it is moving.
    pub pitch: f32,
    /// How much of the high frequencies the air leaves, from 0.0 to 1.0.
    pub high_gain: f32,
}

impl Placement {
    /// How much quieter the source is for being `distance` away.
    fn attenuation(&self, model: DistanceModel, distance: f32) -> f32 {
        let reference = self.reference_distance;
        let max = self.max_distance.max(reference);
        let rolloff = self.rolloff;
        let clamped = distance.clamp(reference, max);
        let gain = match model {
            DistanceModel::None => 1.0,
            DistanceModel::Inverse | DistanceModel::InverseClamped => {
                let distance = if model == DistanceModel::Inverse {
                    distance
                } else {
                    clamped
                };
                let denominator = reference + rolloff * (distance - reference);
                if denominator > 0.0 {
                    reference / denominator
                } else {
                    1.0
                }
            }
            DistanceModel::Linear | DistanceModel::LinearClamped => {
                let distance = if model == DistanceModel::Linear {
                    distance.min(max)
                } else {
                    clamped
                };
                if max > reference {
                    1.0 - rolloff * (distance - reference) / (max - reference)
                } else {
                    1.0
                }
            }
            DistanceModel::Exponent | DistanceModel::ExponentClamped => {
                let distance = if model == DistanceModel::Exponent {
                    distance
                } else {
                    clamped
                };
                if distance > 0.0 && reference > 0.0 {
                    (distance / reference).powf(-rolloff)
                } else {
                    1.0
                }
            }
        };
        gain.clamp(0.0, 1.0)
    }

    /// How much quieter the source is for facing away from the listener.
    /// `to_listener` points from the source to the listener.
    fn cone(&self, to_listener: Vector) -> f32 {
        let facing = length(self.direction);
        let distance = length(to_listener);
        if facing == 0.0 || distance == 0.0 {
            return 1.0;
        }
        let cosine = (dot(self.direction, to_listener) / (facing * distance)).clamp(-1.0, 1.0);
        // The cone angles are full widths, so compare twice the angle off
        // the source's direction.
        let angle = 2.0 * cosine.acos();
        let inner = self.cone_inner.clamp(0.0, 2.0 * PI);
        let outer = self.cone_outer.clamp(inner, 2.0 * PI);
        if angle <= inner {
            1.0
        } else if angle >= outer {
            self.cone_outer_volume
        } else {
            let through = (angle - inner) / (outer - inner);
            1.0 + (self.cone_outer_volume - 1.0) * through
        }
    }

    /// What `listener` hears of a source playing at `volume`.
    pub fn heard(&self, listener: &Listener, volume: f32) -> Heard {
        // Where the source is from the listener, in the listener's own
        // terms: x to the right, y up and negative z ahead.
        let (offset, listener_velocity) = if self.relative {
            (self.position, [0.0; 3])
        } else {
            (sub(self.position, listener.position), listener.velocity)
        };
        let distance = length(offset);
        let side = if self.relative {
            offset[0]
        } else {
            let right = cross(listener.forward, listener.up);
            let right_length = length(right);
            if right_length > 0.0 {
                dot(offset, right) / right_length
            } else {
                0.0
            }
        };
        let pan = if distance > 0.0 {
            (side / distance).clamp(-1.0, 1.0)
        } else {
            0.0
        };

        let to_listener = [-offset[0], -offset[1], -offset[2]];
        let gain = (volume * self.attenuation(listener.model, distance) * self.cone(to_listener))
            .clamp(self.min_volume, self.max_volume);

        // The ear nearer the sound hears it at full volume, so a sound in
        // the middle is as loud as one that is not positioned. The farther
        // ear fades along a quarter circle to silence at the side.
        let far = (pan.abs() * FRAC_PI_2).cos();
        let (left, right) = if pan >= 0.0 { (far, 1.0) } else { (1.0, far) };

        let high_gain = if self.air_absorption > 0.0 {
            AIR_HIGH_GAIN_PER_UNIT.powf(self.air_absorption * distance)
        } else {
            1.0
        };

        Heard {
            gains: [gain * left, gain * right],
            pitch: self.doppler(listener, to_listener, listener_velocity),
            high_gain,
        }
    }

    /// OpenAL's Doppler shift, from the source's and listener's speeds
    /// along the line between them.
    fn doppler(&self, listener: &Listener, to_listener: Vector, listener_velocity: Vector) -> f32 {
        let scale = listener.doppler_scale.max(0.0);
        let distance = length(to_listener);
        if scale == 0.0 || distance == 0.0 {
            return 1.0;
        }
        let limit = SPEED_OF_SOUND / scale;
        let listener_speed = (dot(to_listener, listener_velocity) / distance).min(limit);
        let source_speed = (dot(to_listener, self.velocity) / distance).min(limit);
        let shift =
            (SPEED_OF_SOUND - scale * listener_speed) / (SPEED_OF_SOUND - scale * source_speed);
        if shift.is_finite() {
            shift.clamp(0.1, 10.0)
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard_at(position: Vector) -> Heard {
        Placement {
            position,
            ..Placement::default()
        }
        .heard(&Listener::default(), 1.0)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn air_absorption_muffles_distant_sounds() {
        let near = Placement {
            position: [0.0, 0.0, -10.0],
            air_absorption: 1.0,
            ..Placement::default()
        };
        let far = Placement {
            position: [0.0, 0.0, -100.0],
            ..near
        };
        let listener = Listener::default();
        assert_eq!(heard_at([0.0, 0.0, -100.0]).high_gain, 1.0);
        let near = near.heard(&listener, 1.0).high_gain;
        let far = far.heard(&listener, 1.0).high_gain;
        assert!(close(near, AIR_HIGH_GAIN_PER_UNIT.powi(10)));
        assert!(far < near && near < 1.0);
    }

    #[test]
    fn a_sound_on_the_listener_is_heard_in_full() {
        let heard = heard_at([0.0; 3]);
        assert_eq!(heard.gains, [1.0, 1.0]);
        assert_eq!(heard.pitch, 1.0);
    }

    #[test]
    fn a_sound_to_the_right_is_heard_in_the_right_ear() {
        // Within the reference distance, so only the panning applies.
        let heard = heard_at([0.5, 0.0, 0.0]);
        assert!(close(heard.gains[1], 1.0));
        assert!(close(heard.gains[0], 0.0));

        let left = heard_at([-0.5, 0.0, 0.0]);
        assert!(close(left.gains[0], 1.0) && close(left.gains[1], 0.0));

        // Straight ahead is the middle.
        let ahead = heard_at([0.0, 0.0, -0.5]);
        assert!(close(ahead.gains[0], 1.0) && close(ahead.gains[1], 1.0));
    }

    #[test]
    fn turning_the_listener_around_swaps_the_ears() {
        let listener = Listener {
            forward: [0.0, 0.0, 1.0],
            ..Listener::default()
        };
        let heard = Placement {
            position: [0.5, 0.0, 0.0],
            ..Placement::default()
        }
        .heard(&listener, 1.0);
        assert!(close(heard.gains[0], 1.0) && close(heard.gains[1], 0.0));
    }

    #[test]
    fn the_inverse_model_halves_the_volume_at_twice_the_distance() {
        let heard = heard_at([0.0, 0.0, -2.0]);
        assert!(close(heard.gains[0], 0.5));
        let farther = heard_at([0.0, 0.0, -4.0]);
        assert!(close(farther.gains[0], 0.25));
    }

    #[test]
    fn every_distance_model_matches_openal() {
        let placement = Placement {
            reference_distance: 10.0,
            max_distance: 110.0,
            rolloff: 1.0,
            ..Placement::default()
        };
        let at = |model, distance| placement.attenuation(model, distance);
        assert_eq!(at(DistanceModel::None, 60.0), 1.0);
        assert!(close(at(DistanceModel::Inverse, 20.0), 0.5));
        assert!(close(at(DistanceModel::Inverse, 5.0), 2.0f32.min(1.0)));
        assert!(close(
            at(DistanceModel::InverseClamped, 1_000.0),
            10.0 / 110.0
        ));
        assert!(close(at(DistanceModel::LinearClamped, 60.0), 0.5));
        assert!(close(at(DistanceModel::LinearClamped, 500.0), 0.0));
        assert!(close(at(DistanceModel::Linear, 60.0), 0.5));
        assert!(close(at(DistanceModel::Exponent, 40.0), 0.25f32.powf(1.0)));
        assert!(close(
            at(DistanceModel::ExponentClamped, 1_000.0),
            10.0 / 110.0
        ));
    }

    #[test]
    fn a_relative_source_ignores_where_the_listener_is() {
        let listener = Listener {
            position: [100.0, 0.0, 0.0],
            ..Listener::default()
        };
        let heard = Placement {
            relative: true,
            ..Placement::default()
        }
        .heard(&listener, 1.0);
        assert_eq!(heard.gains, [1.0, 1.0]);

        let absolute = Placement::default().heard(&listener, 1.0);
        assert!(absolute.gains[0] < 0.02);
    }

    #[test]
    fn volume_limits_hold_the_final_volume() {
        let placement = Placement {
            position: [0.0, 0.0, -100.0],
            min_volume: 0.2,
            ..Placement::default()
        };
        let heard = placement.heard(&Listener::default(), 1.0);
        assert!(close(heard.gains[0], 0.2));

        let capped = Placement {
            max_volume: 0.5,
            ..Placement::default()
        }
        .heard(&Listener::default(), 1.0);
        assert!(close(capped.gains[0], 0.5));
    }

    #[test]
    fn a_source_facing_away_is_quieter() {
        let placement = Placement {
            position: [0.0, 0.0, -1.0],
            cone_inner: PI / 2.0,
            cone_outer: PI,
            cone_outer_volume: 0.25,
            ..Placement::default()
        };
        let facing = Placement {
            direction: [0.0, 0.0, 1.0],
            ..placement
        };
        let away = Placement {
            direction: [0.0, 0.0, -1.0],
            ..placement
        };
        let listener = Listener::default();
        assert!(close(facing.heard(&listener, 1.0).gains[0], 1.0));
        assert!(close(away.heard(&listener, 1.0).gains[0], 0.25));
    }

    #[test]
    fn a_sound_coming_closer_is_higher_and_going_away_is_lower() {
        let approaching = Placement {
            position: [0.0, 0.0, -10.0],
            velocity: [0.0, 0.0, 34.33],
            ..Placement::default()
        };
        let pitch = approaching.heard(&Listener::default(), 1.0).pitch;
        assert!(close(pitch, 1.0 / 0.9));

        let leaving = Placement {
            velocity: [0.0, 0.0, -34.33],
            ..approaching
        };
        assert!(leaving.heard(&Listener::default(), 1.0).pitch < 1.0);

        let no_doppler = Listener {
            doppler_scale: 0.0,
            ..Listener::default()
        };
        assert_eq!(approaching.heard(&no_doppler, 1.0).pitch, 1.0);
    }
}
