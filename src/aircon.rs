//! What a room's sensor tells the air conditioner under the same label, for now: above 26℃
//! cool at 25℃, below 15℃ heat at 20℃, below 20℃ stop whatever else it runs, and otherwise
//! leave it as it is.
//!
//! The air conditioner hears infrared only and beeps at every frame, so a command goes out only
//! when it differs from what its child last took; once on, the air conditioner's own
//! thermostat holds the setpoint.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Cool,
    Heat,
    Dry,
}

impl Mode {
    /// The Thermostat's `SystemMode` for the mode; `None` for the others (Auto, Fan only).
    #[must_use]
    pub const fn from_system_mode(mode: u64) -> Option<Self> {
        match mode {
            3 => Some(Self::Cool),
            4 => Some(Self::Heat),
            8 => Some(Self::Dry),
            _ => None,
        }
    }
}

/// What the air conditioner is told; a frame carries the whole state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "power", rename_all = "snake_case")]
pub enum Command {
    Off,
    On { mode: Mode, celsius: u8 },
}

/// The air conditioner as its child last took it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Running {
    pub on: bool,
    pub mode: Option<Mode>,
    /// The heating and cooling setpoints in 0.01℃.
    pub heating: Option<i64>,
    pub cooling: Option<i64>,
}

/// Above this the room is cooled.
pub const COOL_ABOVE: f64 = 26.0;
/// The setpoint it is cooled to.
pub const COOL_TO: u8 = 25;
/// Below this the room is heated.
pub const HEAT_BELOW: f64 = 15.0;
/// The setpoint it is heated to.
pub const HEAT_TO: u8 = 20;
/// Below this anything but heating stops.
pub const STOP_BELOW: f64 = 20.0;

/// Why a run sends what it sends, with the room's ℃.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Why {
    /// Above [`COOL_ABOVE`]: cooling.
    Hot(f64),
    /// Above [`COOL_ABOVE`] and cooling at [`COOL_TO`] already.
    Cooling(f64),
    /// Below [`HEAT_BELOW`]: heating.
    Cold(f64),
    /// Below [`HEAT_BELOW`] and heating at [`HEAT_TO`] already.
    Heating(f64),
    /// Below [`STOP_BELOW`] while running other than heating: stopping it.
    Cooled(f64),
    /// Below [`STOP_BELOW`] and heating: left heating.
    StillHeating(f64),
    /// Below [`STOP_BELOW`] and off already.
    Stopped(f64),
    /// From [`STOP_BELOW`] to [`COOL_ABOVE`]: left as it is.
    Warm(f64),
}

/// What one run sends, if anything, and why.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    pub send: Option<Command>,
    pub why: Why,
}

/// What the room at `room` ℃ calls for from the air conditioner running as `running`.
#[must_use]
pub fn decide(room: f64, running: Running) -> Decision {
    let heating = running.on && running.mode == Some(Mode::Heat);
    let (send, why) = if room > COOL_ABOVE {
        let cooling = running.on && running.mode == Some(Mode::Cool);
        if cooling && running.cooling == Some(i64::from(COOL_TO) * 100) {
            (None, Why::Cooling(room))
        } else {
            let cool = Command::On {
                mode: Mode::Cool,
                celsius: COOL_TO,
            };
            (Some(cool), Why::Hot(room))
        }
    } else if room < HEAT_BELOW {
        if heating && running.heating == Some(i64::from(HEAT_TO) * 100) {
            (None, Why::Heating(room))
        } else {
            let heat = Command::On {
                mode: Mode::Heat,
                celsius: HEAT_TO,
            };
            (Some(heat), Why::Cold(room))
        }
    } else if room < STOP_BELOW {
        if heating {
            (None, Why::StillHeating(room))
        } else if running.on {
            (Some(Command::Off), Why::Cooled(room))
        } else {
            (None, Why::Stopped(room))
        }
    } else {
        (None, Why::Warm(room))
    };
    Decision { send, why }
}

fn celsius(value: f64) -> String {
    format!("{value:.1}℃")
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Hot(room) => write!(f, "室温が{}で26℃を超えたため、冷房25℃にする", celsius(room)),
            Self::Cooling(room) => write!(
                f,
                "室温が{}で26℃を超えているが、冷房25℃で運転中",
                celsius(room)
            ),
            Self::Cold(room) => write!(
                f,
                "室温が{}で15℃を下回ったため、暖房20℃にする",
                celsius(room)
            ),
            Self::Heating(room) => write!(
                f,
                "室温が{}で15℃を下回っているが、暖房20℃で運転中",
                celsius(room)
            ),
            Self::Cooled(room) => write!(
                f,
                "室温が{}で20℃を下回ったため、エアコンを止める",
                celsius(room)
            ),
            Self::StillHeating(room) => write!(f, "室温が{}で、暖房中のため続ける", celsius(room)),
            Self::Stopped(room) => write!(f, "室温が{}で、止まっているためそのまま", celsius(room)),
            Self::Warm(room) => write!(f, "室温が{}で20〜26℃のため、変えない", celsius(room)),
        }
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

    fn on(mode: Mode, celsius: u8) -> Command {
        Command::On { mode, celsius }
    }

    const OFF: Running = Running {
        on: false,
        mode: Some(Mode::Cool),
        heating: Some(2300),
        cooling: Some(2500),
    };
    const COOLING: Running = Running { on: true, ..OFF };
    const HEATING: Running = Running {
        on: true,
        mode: Some(Mode::Heat),
        heating: Some(2000),
        cooling: Some(2200),
    };

    fn sent(room: f64, running: Running) -> Option<Command> {
        decide(room, running).send
    }

    #[test]
    fn below_15_heats_at_20_whatever_it_runs() {
        for running in [OFF, COOLING] {
            assert_eq!(sent(14.9, running), Some(on(Mode::Heat, 20)));
        }
        // Heating at another setpoint is set to 20℃.
        let warmer = Running {
            heating: Some(2200),
            ..HEATING
        };
        assert_eq!(sent(10.0, warmer), Some(on(Mode::Heat, 20)));
        assert_eq!(sent(14.9, HEATING), None);
        assert_eq!(decide(14.9, HEATING).why, Why::Heating(14.9));
    }

    #[test]
    fn below_20_stops_anything_but_heating() {
        let auto = Running {
            mode: None,
            ..COOLING
        };
        let dry = Running {
            mode: Some(Mode::Dry),
            ..COOLING
        };
        for running in [COOLING, auto, dry] {
            assert_eq!(sent(19.9, running), Some(Command::Off));
        }
        assert_eq!(sent(15.0, COOLING), Some(Command::Off));
        assert_eq!(sent(19.9, OFF), None);
        // Heating goes on up to 20℃ and past it: it is left to the air conditioner.
        assert_eq!(sent(15.0, HEATING), None);
        assert_eq!(sent(19.9, HEATING), None);
    }

    #[test]
    fn above_26_cools_at_25_whatever_it_runs() {
        for running in [OFF, HEATING] {
            assert_eq!(sent(26.1, running), Some(on(Mode::Cool, 25)));
        }
        // Cooling at another setpoint is set to 25℃.
        let colder = Running {
            cooling: Some(2600),
            ..COOLING
        };
        assert_eq!(sent(30.0, colder), Some(on(Mode::Cool, 25)));
        assert_eq!(sent(26.1, COOLING), None);
        assert_eq!(decide(26.1, COOLING).why, Why::Cooling(26.1));
    }

    #[test]
    fn from_20_to_26_nothing_is_sent() {
        for running in [OFF, COOLING, HEATING] {
            assert_eq!(sent(20.0, running), None);
            assert_eq!(sent(26.0, running), None);
        }
    }

    #[test]
    fn the_reason_says_the_room_and_what_was_done() {
        assert_eq!(
            decide(14.04, OFF).why.to_string(),
            "室温が14.0℃で15℃を下回ったため、暖房20℃にする"
        );
        assert_eq!(
            decide(19.5, COOLING).why.to_string(),
            "室温が19.5℃で20℃を下回ったため、エアコンを止める"
        );
        assert_eq!(
            decide(24.0, COOLING).why.to_string(),
            "室温が24.0℃で20〜26℃のため、変えない"
        );
        assert_eq!(
            decide(27.5, OFF).why.to_string(),
            "室温が27.5℃で26℃を超えたため、冷房25℃にする"
        );
    }

    #[test]
    fn writes_setpoints_apart_mode_fan_then_on() {
        let cool = on(Mode::Cool, 25);
        let heat = on(Mode::Heat, 25);
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
            steps(on(Mode::Dry, 24), Some(2700)),
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
}
