//! Keeps the room near a target temperature with the air conditioner, from the room meter.
//!
//! The air conditioner hears infrared only and beeps at every frame, so a command goes out only
//! when it differs from the last one sent; the air conditioner's own thermostat holds the
//! setpoint between them.

use serde::{Deserialize, Serialize};

/// The user's switch and target, kept across restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Off turns the air conditioner off and stops the adjustment until switched on again.
    pub on: bool,
    /// Degrees Celsius the room is kept around.
    pub target: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            on: true,
            target: 25,
        }
    }
}

/// The targets the air conditioner takes with both setpoints inside its 16–32℃ ([`SETPOINT_GAP`] apart).
pub const TARGETS: std::ops::RangeInclusive<u8> = 18..=30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Cool,
    Heat,
    Dry,
}

/// What the air conditioner was last told; a frame carries the whole state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "power", rename_all = "snake_case")]
pub enum Command {
    Off,
    On { mode: Mode, celsius: u8 },
}

/// The command to send for the room's `reading` (℃, %RH), fresh or `None`, given the `last`
/// one sent; `None` when nothing is to be sent. Off overrides the adjustment; without a fresh
/// reading the air conditioner is left as it is.
#[must_use]
pub fn decide(
    settings: Settings,
    reading: Option<(f64, u8)>,
    last: Option<Command>,
) -> Option<Command> {
    let want = if settings.on {
        let (temperature, humidity) = reading?;
        let running = match last {
            Some(Command::On { mode, .. }) => Some(mode),
            _ => None,
        };
        Command::On {
            mode: mode(running, temperature, humidity, f64::from(settings.target)),
            celsius: settings.target,
        }
    } else {
        Command::Off
    };
    (last != Some(want)).then_some(want)
}

/// Degrees either side of the target the room may drift before heating and cooling change over.
const BAND: f64 = 1.0;
/// Relative humidity from which a room inside the band is dried, and the one it is dried down to.
const HUMID: u8 = 65;
const DRY_ENOUGH: u8 = 55;

/// The mode for the room after `running`: the side of the band decides, and inside it the
/// running mode holds, so a room near the target does not switch over every run.
fn mode(running: Option<Mode>, temperature: f64, humidity: u8, target: f64) -> Mode {
    if temperature <= target - BAND {
        return Mode::Heat;
    }
    if temperature >= target + BAND {
        return Mode::Cool;
    }
    let humid = humidity >= HUMID || (running == Some(Mode::Dry) && humidity > DRY_ENOUGH);
    match running {
        Some(Mode::Heat) => Mode::Heat,
        _ if humid => Mode::Dry,
        None if temperature < target => Mode::Heat,
        Some(Mode::Cool | Mode::Dry) | None => Mode::Cool,
    }
}

/// One write to the air conditioner's endpoint, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `cluster/attribute` and its value.
    Write(&'static str, i64),
    /// On/Off's On or Off command.
    Switch(bool),
}

pub const SYSTEM_MODE: &str = "513/28";
pub const COOLING_SETPOINT: &str = "513/17";
pub const HEATING_SETPOINT: &str = "513/18";
pub const FAN_MODE: &str = "514/0";

/// Degrees between the heating and cooling setpoints, at least the Auto mode's dead band.
pub const SETPOINT_GAP: i64 = 2;

/// The writes that put `command` on an air conditioner whose cooling setpoint is now
/// `cooling_now` (0.01℃): setpoints in an order that keeps them [`SETPOINT_GAP`] apart throughout.
#[must_use]
pub fn steps(command: Command, cooling_now: Option<i64>) -> Vec<Step> {
    let Command::On { mode, celsius } = command else {
        return vec![Step::Switch(false)];
    };
    let celsius = i64::from(celsius);
    let (heating, cooling, system_mode) = match mode {
        Mode::Heat => (celsius, celsius + SETPOINT_GAP, 4),
        Mode::Cool => (celsius - SETPOINT_GAP, celsius, 3),
        Mode::Dry => (celsius - SETPOINT_GAP, celsius, 8),
    };
    let lowering = cooling_now.is_some_and(|now| cooling * 100 < now);
    let (heating, cooling) = (
        Step::Write(HEATING_SETPOINT, heating * 100),
        Step::Write(COOLING_SETPOINT, cooling * 100),
    );
    let setpoints = if lowering {
        [heating, cooling]
    } else {
        [cooling, heating]
    };
    let mut steps = setpoints.to_vec();
    // FanMode Auto (5): the child reads its first FanMode, Off, as Auto too.
    steps.extend([
        Step::Write(SYSTEM_MODE, system_mode),
        Step::Write(FAN_MODE, 5),
        Step::Switch(true),
    ]);
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Settings = Settings {
        on: true,
        target: 25,
    };
    const OFF: Settings = Settings {
        on: false,
        target: 25,
    };

    #[allow(clippy::unnecessary_wraps)] // compared with what `decide` returns
    fn run(mode: Mode) -> Option<Command> {
        Some(Command::On { mode, celsius: 25 })
    }

    #[test]
    fn heats_below_and_cools_above_the_band() {
        for (temperature, mode) in [
            (24.0, Mode::Heat),
            (15.0, Mode::Heat),
            (26.0, Mode::Cool),
            (31.2, Mode::Cool),
        ] {
            for last in [
                None,
                run(Mode::Heat),
                run(Mode::Cool),
                run(Mode::Dry),
                Some(Command::Off),
            ] {
                let want = run(mode).filter(|_| last != run(mode));
                assert_eq!(
                    decide(ON, Some((temperature, 50)), last),
                    want,
                    "{temperature} after {last:?}"
                );
            }
        }
    }

    #[test]
    fn keeps_the_running_mode_inside_the_band() {
        for temperature in [24.1, 25.0, 25.9] {
            assert_eq!(decide(ON, Some((temperature, 50)), run(Mode::Heat)), None);
            assert_eq!(decide(ON, Some((temperature, 50)), run(Mode::Cool)), None);
        }
    }

    #[test]
    fn starts_inside_the_band_by_the_side_of_the_target() {
        for last in [None, Some(Command::Off)] {
            assert_eq!(decide(ON, Some((24.9, 50)), last), run(Mode::Heat));
            assert_eq!(decide(ON, Some((25.0, 50)), last), run(Mode::Cool));
            assert_eq!(decide(ON, Some((25.9, 50)), last), run(Mode::Cool));
        }
    }

    #[test]
    fn dries_a_humid_room_inside_the_band_until_it_is_dry_enough() {
        assert_eq!(
            decide(ON, Some((25.0, 65)), run(Mode::Cool)),
            run(Mode::Dry)
        );
        assert_eq!(decide(ON, Some((24.5, 70)), None), run(Mode::Dry));
        assert_eq!(decide(ON, Some((25.0, 64)), run(Mode::Cool)), None);
        assert_eq!(decide(ON, Some((25.0, 56)), run(Mode::Dry)), None);
        assert_eq!(
            decide(ON, Some((25.0, 55)), run(Mode::Dry)),
            run(Mode::Cool)
        );
        // Heating dries the air itself; it runs on until the room is warm.
        assert_eq!(decide(ON, Some((25.0, 80)), run(Mode::Heat)), None);
        // Out of the band the temperature comes first.
        assert_eq!(
            decide(ON, Some((26.0, 80)), run(Mode::Dry)),
            run(Mode::Cool)
        );
        assert_eq!(
            decide(ON, Some((24.0, 80)), run(Mode::Dry)),
            run(Mode::Heat)
        );
    }

    #[test]
    fn the_band_follows_the_target() {
        let cooler = Settings {
            on: true,
            target: 22,
        };
        assert_eq!(
            decide(cooler, Some((23.0, 50)), None),
            Some(Command::On {
                mode: Mode::Cool,
                celsius: 22
            })
        );
        assert_eq!(
            decide(
                cooler,
                Some((21.0, 50)),
                Some(Command::On {
                    mode: Mode::Cool,
                    celsius: 22
                })
            ),
            Some(Command::On {
                mode: Mode::Heat,
                celsius: 22
            })
        );
        // A new target is sent at once, in the same mode.
        assert_eq!(
            decide(cooler, Some((22.5, 50)), run(Mode::Cool)),
            Some(Command::On {
                mode: Mode::Cool,
                celsius: 22
            })
        );
    }

    #[test]
    fn leaves_the_air_conditioner_without_a_fresh_reading() {
        for last in [None, run(Mode::Cool), Some(Command::Off)] {
            assert_eq!(decide(ON, None, last), None);
        }
    }

    #[test]
    fn off_turns_it_off_once_whatever_the_room() {
        for reading in [None, Some((31.0, 80)), Some((15.0, 30))] {
            for last in [None, run(Mode::Cool), run(Mode::Heat)] {
                assert_eq!(decide(OFF, reading, last), Some(Command::Off));
            }
            assert_eq!(decide(OFF, reading, Some(Command::Off)), None);
        }
    }

    #[test]
    fn on_after_off_turns_it_on_for_the_room() {
        assert_eq!(
            decide(ON, Some((28.0, 50)), Some(Command::Off)),
            run(Mode::Cool)
        );
        assert_eq!(
            decide(ON, Some((20.0, 50)), Some(Command::Off)),
            run(Mode::Heat)
        );
    }

    #[test]
    fn writes_setpoints_apart_mode_fan_then_on() {
        let cool = Command::On {
            mode: Mode::Cool,
            celsius: 25,
        };
        let heat = Command::On {
            mode: Mode::Heat,
            celsius: 25,
        };
        let tail = [Step::Write(FAN_MODE, 5), Step::Switch(true)];
        // Raising the cooling setpoint goes first, lowering it last, so heating stays below it.
        assert_eq!(
            steps(cool, Some(2400)),
            [
                [
                    Step::Write(COOLING_SETPOINT, 2500),
                    Step::Write(HEATING_SETPOINT, 2300)
                ]
                .as_slice(),
                &[Step::Write(SYSTEM_MODE, 3)],
                &tail,
            ]
            .concat()
        );
        assert_eq!(
            steps(heat, Some(2500)),
            [
                [
                    Step::Write(COOLING_SETPOINT, 2700),
                    Step::Write(HEATING_SETPOINT, 2500)
                ]
                .as_slice(),
                &[Step::Write(SYSTEM_MODE, 4)],
                &tail,
            ]
            .concat()
        );
        assert_eq!(
            steps(
                Command::On {
                    mode: Mode::Dry,
                    celsius: 24
                },
                Some(2700)
            ),
            [
                [
                    Step::Write(HEATING_SETPOINT, 2200),
                    Step::Write(COOLING_SETPOINT, 2400)
                ]
                .as_slice(),
                &[Step::Write(SYSTEM_MODE, 8)],
                &tail,
            ]
            .concat()
        );
        assert_eq!(steps(cool, None)[0], Step::Write(COOLING_SETPOINT, 2500));
        assert_eq!(steps(Command::Off, Some(2500)), [Step::Switch(false)]);
    }

    #[test]
    fn settings_default_to_on_at_25() {
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), ON);
        assert_eq!(
            serde_json::from_str::<Settings>(r#"{"on":false}"#).unwrap(),
            OFF
        );
    }
}
