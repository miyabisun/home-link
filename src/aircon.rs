//! Keeps the room near a target temperature with the air conditioner, from the room meter
//! and the temperature outside.
//!
//! The air conditioner hears infrared only and beeps at every frame, so a command goes out only
//! when it differs from the last one sent, at most every half hour unless the user acts; the
//! air conditioner's own thermostat holds the setpoint between them.
//!
//! The outside decides which way the room drifts with the air conditioner off: from the target
//! up it warms, so nothing heats it; from five degrees below the target down it cools, so
//! nothing cools or dries it. While the outside pushes the room away from the target, cooling
//! from two degrees above and heating from five below, reaching the target leaves the air
//! conditioner running at its own low output, which costs less than stopping and starting it;
//! otherwise the air conditioner stops at the target and starts again once the room leaves the
//! band, each state lasting half an hour at least. The setpoint moves a degree at a time for
//! as long as the meter's half-hour average stays half a degree off the target, since the air
//! conditioner holds its own intake's temperature, not the meter's.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The user's switch, target and night, kept across restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Off turns the air conditioner off and stops the adjustment until switched on again.
    pub on: bool,
    /// Degrees Celsius the room is kept around.
    pub target: u8,
    /// The night in minutes after midnight in Asia/Tokyo, when only leaving the band is sent.
    pub night_from: u16,
    pub night_until: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            on: true,
            target: 25,
            night_from: 22 * 60,
            night_until: 7 * 60,
        }
    }
}

impl Settings {
    /// Whether `minute` after midnight falls in the night; a night of no length never does.
    #[must_use]
    pub fn is_night(&self, minute: u32) -> bool {
        let (from, until) = (u32::from(self.night_from), u32::from(self.night_until));
        if from <= until {
            (from..until).contains(&minute)
        } else {
            minute >= from || minute < until
        }
    }
}

/// The targets the air conditioner takes with both setpoints inside its 16–32℃ ([`SETPOINT_GAP`] apart).
pub const TARGETS: std::ops::RangeInclusive<u8> = 18..=30;

/// Minutes in a day, the bound of the night's ends.
pub const MINUTES_A_DAY: u16 = 24 * 60;

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

impl Command {
    fn mode(self) -> Option<Mode> {
        match self {
            Self::Off => None,
            Self::On { mode, .. } => Some(mode),
        }
    }
}

/// What set off a run: the ten-minute loop, or the user switching or changing the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Loop,
    Switch,
    Target,
}

/// One run's view of the room and the outside, both fresh or `None`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// Unix seconds.
    pub now: i64,
    /// The meter's ℃ and %RH.
    pub room: Option<(f64, u8)>,
    /// ℃ outside.
    pub outdoor: Option<f64>,
    pub trigger: Trigger,
}

/// What the adjustment remembers between runs, kept across restarts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    /// Degrees each mode's setpoint is off the target, from the corrections so far.
    pub cool_offset: i8,
    pub heat_offset: i8,
    /// When the air conditioner was last switched on or off, and last sent anything, in Unix seconds.
    pub switched_at: Option<i64>,
    pub sent_at: Option<i64>,
    /// Since when the running setpoint has been watched, and the meter's temperatures in the last half hour.
    pub watch_from: Option<i64>,
    pub samples: Vec<(i64, f64)>,
    /// Whether the last change out of the band is moving the room.
    pub check: Option<Check>,
}

/// A command sent while the room was out of the band, awaiting the room's answer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub mode: Mode,
    /// When it was sent, and the room's temperature then.
    pub at: i64,
    pub temperature: f64,
    /// Sent once more already; then the room's silence means the air conditioner may not hear.
    pub resent: bool,
    pub suspect: bool,
}

/// Why a run wants what it wants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Why {
    Off,
    NoReading,
    High {
        room: f64,
        target: u8,
    },
    Low {
        room: f64,
        target: u8,
    },
    Humid {
        humidity: u8,
    },
    HoldCool {
        outdoor: f64,
        target: u8,
    },
    HoldHeat {
        outdoor: f64,
        target: u8,
    },
    NoHeat {
        outdoor: f64,
    },
    NoCool {
        outdoor: f64,
    },
    Reached {
        outdoor: f64,
        target: u8,
    },
    Prepare {
        outdoor: f64,
        mode: Mode,
    },
    /// Inside the band: the running mode holds, or with no outside reading the room's side starts one.
    Keep,
    Idle,
    Correct {
        average: f64,
        from: u8,
        to: u8,
    },
    Resend,
    Suspect,
}

/// Why a command that differs from the last one is not sent yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// The air conditioner runs, or stays off, half an hour at least.
    MinRun,
    MinOff,
    /// Something went out within the half hour.
    Recent,
    /// At night only leaving the band is sent.
    Night,
    /// The air conditioner may not be hearing.
    Suspect,
}

/// One run's outcome: what to send, why, and what to remember if it is accepted or not.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// What the room calls for, sent or not; `None` without a reading.
    pub command: Option<Command>,
    pub send: Option<Command>,
    pub why: Why,
    pub hold: Option<Hold>,
    /// Whether the outside's temperature was known.
    pub outdoor: bool,
    pub kept: Memory,
    pub sent: Memory,
}

/// How long the air conditioner stays on or off at least, the least time between automatic
/// sends, the span the setpoint is watched over, and the wait for the room to answer a command.
pub const HALF_HOUR: i64 = 30 * 60;
/// Degrees either side of the target the room may drift before the air conditioner changes over.
const BAND: f64 = 1.0;
/// Relative humidity from which a room inside the band is dried, and the one it is dried down to.
const HUMID: u8 = 65;
const DRY_ENOUGH: u8 = 55;
/// Degrees below the target from which the outside cools the room, and above it from which it
/// heats the room enough to keep cooling at the target.
const COLD_BELOW: f64 = 5.0;
const HOT_ABOVE: f64 = 2.0;
/// The half-hour average's distance from the target that moves the setpoint a degree, and how
/// far the setpoint may move from the target.
const OFF_BY: f64 = 0.5;
const OFFSET_LIMIT: i8 = 3;

/// The run for `input` given the `last` command the air conditioner accepted and what was
/// remembered; `settings` off overrides everything.
#[must_use]
pub fn decide(
    settings: Settings,
    input: &Input,
    last: Option<Command>,
    memory: &Memory,
) -> Decision {
    let now = input.now;
    let user = input.trigger != Trigger::Loop;
    let mut kept = memory.clone();
    if user {
        kept.check = None;
    }
    let outdoor = input.outdoor.is_some();
    if !settings.on {
        kept.check = None;
        let send = (last != Some(Command::Off)).then_some(Command::Off);
        let accepted = after_send(&kept, now, last, Command::Off);
        return Decision {
            command: Some(Command::Off),
            send,
            why: Why::Off,
            hold: None,
            outdoor,
            kept,
            sent: accepted,
        };
    }
    let Some((room, humidity)) = input.room else {
        let unchanged = kept.clone();
        return Decision {
            command: None,
            send: None,
            why: Why::NoReading,
            hold: None,
            outdoor,
            kept,
            sent: unchanged,
        };
    };
    let running = last.and_then(Command::mode);
    let night = settings.is_night(crate::schedule::local(now).1);
    let resend = kept.observe(now, room, f64::from(settings.target), running, night);

    let (want, mut why) = want(settings.target, room, humidity, running, input);
    let mut command = want.map_or(Command::Off, |mode| Command::On {
        mode,
        celsius: setpoint(settings.target, mode, &kept),
    });
    if resend {
        command = last.unwrap_or(command);
        why = Why::Resend;
    } else if let Some((corrected, correct)) = correction(settings.target, command, &kept, now)
        .filter(|_| want.is_some() && want == running)
    {
        (command, why) = (corrected, correct);
    }
    let suspect = kept.check.is_some_and(|c| c.suspect);
    if suspect && !user {
        why = Why::Suspect;
    }

    let changes = resend || Some(command) != last;
    let hold = if !changes || user {
        None
    } else if suspect {
        Some(Hold::Suspect)
    } else {
        // A resend is due at night as by day.
        hold(memory, now, running, command, why, night && !resend)
    };
    let send = (changes && hold.is_none()).then_some(command);

    let mut accepted = after_send(&kept, now, last, command);
    if let Why::Correct { to, .. } = why {
        let offset = i8::try_from(i16::from(to) - i16::from(settings.target)).unwrap_or(0);
        match command.mode() {
            Some(Mode::Heat) => accepted.heat_offset = offset,
            _ => accepted.cool_offset = offset,
        }
        accepted.watch_from = Some(now);
        accepted.samples.clear();
    }
    let mode = command.mode();
    if resend || (mode != running && matches!(why, Why::High { .. } | Why::Low { .. })) {
        accepted.check = mode.map(|mode| Check {
            mode,
            at: now,
            temperature: room,
            resent: resend,
            suspect: false,
        });
    }
    Decision {
        command: Some(command),
        send,
        why,
        hold,
        outdoor,
        kept,
        sent: accepted,
    }
}

impl Memory {
    /// Takes the room's temperature at `now` into the half hour the running setpoint is
    /// watched over, which the night does not correct, and into the check of the last change
    /// out of the band; whether that change is due to be sent again.
    fn observe(
        &mut self,
        now: i64,
        room: f64,
        target: f64,
        running: Option<Mode>,
        night: bool,
    ) -> bool {
        if night || !matches!(running, Some(Mode::Cool | Mode::Heat)) {
            self.watch_from = None;
            self.samples.clear();
        } else {
            self.watch_from.get_or_insert(now);
            self.samples.retain(|&(at, _)| at >= now - HALF_HOUR);
            self.samples.push((now, room));
        }
        let Some(check) = self.check else {
            return false;
        };
        let out_of_band = match check.mode {
            Mode::Cool => room >= target + BAND,
            Mode::Heat => room <= target - BAND,
            Mode::Dry => false,
        };
        let moved = match check.mode {
            Mode::Cool => room < check.temperature,
            _ => room > check.temperature,
        };
        if running != Some(check.mode) || !out_of_band || (now - check.at >= HALF_HOUR && moved) {
            self.check = None;
        } else if now - check.at >= HALF_HOUR {
            if !check.resent {
                return true;
            }
            self.check = Some(Check {
                suspect: true,
                ..check
            });
        }
        false
    }
}

/// Why an automatic `command` that changes what runs is not sent yet, if it is not; at
/// `night`, only a room out of its band is acted on.
fn hold(
    memory: &Memory,
    now: i64,
    running: Option<Mode>,
    command: Command,
    why: Why,
    night: bool,
) -> Option<Hold> {
    let switches = (command != Command::Off) != running.is_some();
    if switches && memory.switched_at.is_some_and(|at| now - at < HALF_HOUR) {
        Some(if running.is_some() {
            Hold::MinRun
        } else {
            Hold::MinOff
        })
    } else if memory.sent_at.is_some_and(|at| now - at < HALF_HOUR) {
        Some(Hold::Recent)
    } else if night && !matches!(why, Why::High { .. } | Why::Low { .. }) {
        Some(Hold::Night)
    } else {
        None
    }
}

/// The mode the room calls for, `None` for off, and why.
fn want(
    target: u8,
    room: f64,
    humidity: u8,
    running: Option<Mode>,
    input: &Input,
) -> (Option<Mode>, Why) {
    let t = f64::from(target);
    let outside = input.outdoor;
    let o = outside.unwrap_or(f64::NAN);
    // Without the outside every mode may run and none stops by itself, as before it was known.
    let may_heat = outside.is_none_or(|o| o < t);
    let may_cool = outside.is_none_or(|o| o > t - COLD_BELOW);
    let holds_cool = outside.is_none_or(|o| o >= t + HOT_ABOVE);
    let holds_heat = outside.is_none_or(|o| o <= t - COLD_BELOW);
    let in_band = t - BAND < room && room < t + BAND;
    let humid = humidity >= HUMID || (running == Some(Mode::Dry) && humidity > DRY_ENOUGH);
    if room >= t + BAND && may_cool {
        return (Some(Mode::Cool), Why::High { room, target });
    }
    if room <= t - BAND && may_heat {
        return (Some(Mode::Heat), Why::Low { room, target });
    }
    if in_band && humid && may_cool && running != Some(Mode::Heat) {
        return (Some(Mode::Dry), Why::Humid { humidity });
    }
    match running {
        Some(Mode::Heat) if !may_heat => (None, Why::NoHeat { outdoor: o }),
        Some(Mode::Heat) if !holds_heat && room >= t => (None, Why::Reached { outdoor: o, target }),
        Some(Mode::Heat) if outside.is_some() && room >= t => {
            (Some(Mode::Heat), Why::HoldHeat { outdoor: o, target })
        }
        Some(Mode::Heat) => (Some(Mode::Heat), Why::Keep),
        Some(_) if !may_cool => (None, Why::NoCool { outdoor: o }),
        Some(_) if !holds_cool && room <= t => (None, Why::Reached { outdoor: o, target }),
        Some(_) if outside.is_some() && room <= t => {
            (Some(Mode::Cool), Why::HoldCool { outdoor: o, target })
        }
        Some(_) => (Some(Mode::Cool), Why::Keep),
        None => match outside {
            None if room < t => (Some(Mode::Heat), Why::Keep),
            None => (Some(Mode::Cool), Why::Keep),
            // Switched on before coming home: the side the outside pushes starts at once.
            Some(o)
                if input.trigger == Trigger::Switch && o >= t + HOT_ABOVE && room > t - BAND =>
            {
                (
                    Some(Mode::Cool),
                    Why::Prepare {
                        outdoor: o,
                        mode: Mode::Cool,
                    },
                )
            }
            Some(o)
                if input.trigger == Trigger::Switch && o <= t - COLD_BELOW && room < t + BAND =>
            {
                (
                    Some(Mode::Heat),
                    Why::Prepare {
                        outdoor: o,
                        mode: Mode::Heat,
                    },
                )
            }
            Some(o) if room >= t + BAND => (None, Why::NoCool { outdoor: o }),
            Some(o) if room <= t - BAND => (None, Why::NoHeat { outdoor: o }),
            Some(_) => (None, Why::Idle),
        },
    }
}

/// The setpoints each mode takes inside the air conditioner's 16–32℃, the other one [`SETPOINT_GAP`] away.
fn range(mode: Mode) -> (i16, i16) {
    match mode {
        Mode::Heat => (16, 30),
        Mode::Cool | Mode::Dry => (18, 32),
    }
}

/// `mode`'s setpoint for `target` with its correction; drying keeps the target.
fn setpoint(target: u8, mode: Mode, memory: &Memory) -> u8 {
    let offset = match mode {
        Mode::Cool => memory.cool_offset,
        Mode::Heat => memory.heat_offset,
        Mode::Dry => 0,
    };
    let (low, high) = range(mode);
    u8::try_from((i16::from(target) + i16::from(offset)).clamp(low, high)).unwrap_or(target)
}

/// `command` with its setpoint a degree towards the half hour's average, and why, when the
/// running setpoint has been watched half an hour and both the average and the latest reading
/// are [`OFF_BY`] off the target on the same side.
fn correction(target: u8, command: Command, memory: &Memory, now: i64) -> Option<(Command, Why)> {
    let Command::On {
        mode: mode @ (Mode::Cool | Mode::Heat),
        celsius,
    } = command
    else {
        return None;
    };
    if memory.watch_from.is_none_or(|from| now - from < HALF_HOUR) || memory.samples.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss, reason = "a handful of samples")]
    let average = memory.samples.iter().map(|(_, t)| t).sum::<f64>() / memory.samples.len() as f64;
    let off = average - f64::from(target);
    // A room still on its way to the target, its latest reading near it, is left to arrive.
    let latest = memory
        .samples
        .last()
        .map_or(0.0, |(_, t)| t - f64::from(target));
    if off.abs() < OFF_BY || latest.abs() < OFF_BY || latest.signum() != off.signum() {
        return None;
    }
    let step: i16 = if off > 0.0 { -1 } else { 1 };
    let offset = i16::from(celsius) - i16::from(target) + step;
    let (low, high) = range(mode);
    let limit = i16::from(OFFSET_LIMIT);
    let to = (i16::from(target) + offset.clamp(-limit, limit)).clamp(low, high);
    let to = u8::try_from(to).ok().filter(|&to| to != celsius)?;
    Some((
        Command::On { mode, celsius: to },
        Why::Correct {
            average,
            from: celsius,
            to,
        },
    ))
}

/// What to remember once `command` is accepted after `last`.
fn after_send(memory: &Memory, now: i64, last: Option<Command>, command: Command) -> Memory {
    let mut sent = memory.clone();
    sent.sent_at = Some(now);
    let running = last.and_then(Command::mode);
    if (command != Command::Off) != running.is_some() {
        sent.switched_at = Some(now);
    }
    if command.mode() != running {
        sent.watch_from = matches!(command.mode(), Some(Mode::Cool | Mode::Heat)).then_some(now);
        sent.samples.clear();
        sent.check = None;
    }
    sent
}

fn celsius(value: f64) -> String {
    format!("{value:.1}℃")
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = |mode| match mode {
            Mode::Cool => "冷房",
            Mode::Heat => "暖房",
            Mode::Dry => "除湿",
        };
        match *self {
            Self::Off => write!(f, "OFFのため、エアコンを切る"),
            Self::NoReading => write!(f, "温度計の新しい値が無いため、変えない"),
            Self::High { room, target } => {
                write!(
                    f,
                    "室温{}が{target}℃より1℃以上高いため、冷房にする",
                    celsius(room)
                )
            }
            Self::Low { room, target } => {
                write!(
                    f,
                    "室温{}が{target}℃より1℃以上低いため、暖房にする",
                    celsius(room)
                )
            }
            Self::Humid { humidity } => write!(f, "湿度{humidity}%のため、除湿する"),
            Self::HoldCool { outdoor, target } => {
                write!(
                    f,
                    "外が{}のため、{target}℃に下がっても冷房を続ける",
                    celsius(outdoor)
                )
            }
            Self::HoldHeat { outdoor, target } => {
                write!(
                    f,
                    "外が{}のため、{target}℃に上がっても暖房を続ける",
                    celsius(outdoor)
                )
            }
            Self::NoHeat { outdoor } => {
                write!(
                    f,
                    "外が{}で部屋は暖まる側のため、暖房は入れない",
                    celsius(outdoor)
                )
            }
            Self::NoCool { outdoor } => {
                write!(
                    f,
                    "外が{}で部屋は冷える側のため、冷房・除湿は入れない",
                    celsius(outdoor)
                )
            }
            Self::Reached { outdoor, target } => {
                write!(
                    f,
                    "外が{}で、室温が{target}℃に届いたため切る",
                    celsius(outdoor)
                )
            }
            Self::Prepare { outdoor, mode } => write!(
                f,
                "ONにした直後で外が{}のため、すぐ{}で入れる",
                celsius(outdoor),
                name(mode)
            ),
            Self::Keep => write!(f, "室温が目標±1℃の中のため、今の運転を保つ"),
            Self::Idle => write!(f, "室温が目標±1℃の中のため、止めておく"),
            Self::Correct { average, from, to } => write!(
                f,
                "30分の室温の平均が{}のため、設定温度を{from}℃から{to}℃にする",
                celsius(average)
            ),
            Self::Resend => write!(f, "送ってから30分、室温が動かないため、同じ状態を送り直す"),
            Self::Suspect => write!(
                f,
                "送り直しても室温が動かない。エアコンが信号を受けていない可能性がある"
            ),
        }
    }
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.outdoor && !matches!(self.why, Why::Off | Why::NoReading) {
            write!(f, "外気温が無いため室温だけで判断: ")?;
        }
        write!(f, "{}", self.why)?;
        match self.hold {
            None | Some(Hold::Suspect) => Ok(()),
            Some(Hold::MinRun) => write!(f, "（運転は30分続けるため、まだ切らない）"),
            Some(Hold::MinOff) => write!(f, "（停止は30分続けるため、まだ入れない）"),
            Some(Hold::Recent) => write!(f, "（30分以内に送ったため、次の回に送る）"),
            Some(Hold::Night) => write!(f, "（夜のため、朝まで送らない）"),
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

    const ON: Settings = Settings {
        on: true,
        target: 25,
        night_from: 22 * 60,
        night_until: 7 * 60,
    };
    const OFF: Settings = Settings { on: false, ..ON };

    /// 2026-10-24 12:00 in Asia/Tokyo, and 23:00.
    const NOON: i64 = 1_792_810_800;
    const ELEVEN_PM: i64 = NOON + 11 * 3600;
    const MINUTE: i64 = 60;

    fn on(mode: Mode, celsius: u8) -> Command {
        Command::On { mode, celsius }
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "compared with what `decide` returns"
    )]
    fn run(mode: Mode) -> Option<Command> {
        Some(on(mode, 25))
    }

    /// One run at noon from nothing remembered, without the outside: the adjustment of v0.1.47.
    fn room_only(
        settings: Settings,
        reading: Option<(f64, u8)>,
        last: Option<Command>,
    ) -> Option<Command> {
        let input = Input {
            now: NOON,
            room: reading,
            outdoor: None,
            trigger: Trigger::Loop,
        };
        decide(settings, &input, last, &Memory::default()).send
    }

    /// The air conditioner over time: each run decides, and what is sent is accepted.
    struct Room {
        settings: Settings,
        now: i64,
        outdoor: Option<f64>,
        last: Option<Command>,
        memory: Memory,
        sends: Vec<(i64, Command)>,
        decision: Option<Decision>,
    }

    impl Room {
        fn new(now: i64, outdoor: Option<f64>, last: Option<Command>) -> Self {
            Self {
                settings: ON,
                now,
                outdoor,
                last,
                memory: Memory::default(),
                sends: Vec::new(),
                decision: None,
            }
        }

        /// One run with the room at `temperature` ℃ and `humidity` %; what it sent.
        fn run_with(
            &mut self,
            temperature: f64,
            humidity: u8,
            trigger: Trigger,
        ) -> Option<Command> {
            let input = Input {
                now: self.now,
                room: Some((temperature, humidity)),
                outdoor: self.outdoor,
                trigger,
            };
            let decision = decide(self.settings, &input, self.last, &self.memory);
            if let Some(command) = decision.send {
                self.last = Some(command);
                self.memory = decision.sent.clone();
                self.sends.push((self.now, command));
            } else {
                self.memory = decision.kept.clone();
            }
            self.decision = Some(decision.clone());
            decision.send
        }

        /// A loop's run, then ten minutes on.
        fn tick(&mut self, temperature: f64) -> Option<Command> {
            self.tick_humid(temperature, 50)
        }

        fn tick_humid(&mut self, temperature: f64, humidity: u8) -> Option<Command> {
            let sent = self.run_with(temperature, humidity, Trigger::Loop);
            self.now += 10 * MINUTE;
            sent
        }

        /// The loop's runs at `temperatures`, ten minutes apart; whatever went out.
        fn ticks(&mut self, temperatures: &[f64]) -> Vec<Command> {
            temperatures.iter().filter_map(|&t| self.tick(t)).collect()
        }

        fn reason(&self) -> String {
            self.decision.as_ref().expect("a run").to_string()
        }
    }

    // ---- The outside decides which way the room drifts (cases 1–4, 6, 14) ----

    #[test]
    fn hot_outside_keeps_cooling_past_the_target_then_raises_the_setpoint() {
        // Case 1: 30℃ outside, cooling while the room falls 26 → 25 → 24.5.
        let mut room = Room::new(NOON, Some(30.0), run(Mode::Cool));
        assert_eq!(room.ticks(&[26.0, 25.0, 24.5, 24.5, 24.5]), []);
        assert!(
            room.reason()
                .contains("外が30.0℃のため、25℃に下がっても冷房を続ける"),
            "{}",
            room.reason()
        );
        // 24.5℃ for half an hour (20–50 minutes): one degree up.
        assert_eq!(room.ticks(&[24.5]), [on(Mode::Cool, 26)]);
        assert_eq!(room.ticks(&[24.6, 24.7]), []);
    }

    #[test]
    fn cold_outside_keeps_heating_past_the_target_then_lowers_the_setpoint() {
        // Case 2: 5℃ outside, heating while the room rises 24 → 25 → 25.5.
        let mut room = Room::new(NOON, Some(5.0), run(Mode::Heat));
        assert_eq!(room.ticks(&[24.0, 25.0, 25.5, 25.5, 25.5]), []);
        assert_eq!(room.ticks(&[25.5]), [on(Mode::Heat, 24)]);
        // Warmer still, but no cooling with the outside this cold.
        room.now += HALF_HOUR;
        room.ticks(&[26.5, 26.5]);
        assert!(
            matches!(
                room.last,
                Some(Command::On {
                    mode: Mode::Heat,
                    ..
                })
            ),
            "{:?}",
            room.last
        );
    }

    #[test]
    fn mild_outside_stops_at_the_target_and_starts_out_of_the_band() {
        // Case 3: 22℃ outside, cooling a warm room.
        let mut room = Room::new(NOON, Some(22.0), Some(Command::Off));
        assert_eq!(room.ticks(&[26.0]), [run(Mode::Cool).unwrap()]);
        // Reached at 10 minutes, but it runs half an hour at least.
        assert_eq!(room.ticks(&[25.0, 24.8]), []);
        assert!(room.reason().contains("30分"), "{}", room.reason());
        assert_eq!(room.ticks(&[24.8]), [Command::Off]);
        // Out of the band again at 40 minutes, but it stays off half an hour at least.
        assert_eq!(room.ticks(&[26.0, 26.0]), []);
        assert_eq!(room.ticks(&[26.0]), [run(Mode::Cool).unwrap()]);
        let start = room.sends[0].0;
        let at: Vec<_> = room
            .sends
            .iter()
            .map(|(at, _)| (at - start) / MINUTE)
            .collect();
        assert_eq!(at, [0, 30, 60]);
    }

    #[test]
    fn warm_outside_never_heats() {
        // Case 4: 28℃ outside, off, the room at 24℃.
        let mut room = Room::new(NOON, Some(28.0), Some(Command::Off));
        assert_eq!(room.ticks(&[24.0, 23.0, 22.0]), []);
        assert!(
            room.reason().contains("暖房は入れない"),
            "{}",
            room.reason()
        );
        // From 25℃ outside up, whatever was running.
        let mut room = Room::new(NOON, Some(25.0), run(Mode::Cool));
        assert_eq!(room.ticks(&[23.5]), [Command::Off]);
    }

    #[test]
    fn cold_outside_never_cools_or_dries() {
        let mut room = Room::new(NOON, Some(20.0), Some(Command::Off));
        assert_eq!(room.ticks(&[26.5, 27.0]), []);
        assert!(
            room.reason().contains("冷房・除湿は入れない"),
            "{}",
            room.reason()
        );
    }

    #[test]
    fn dries_unless_the_outside_is_cold() {
        // Case 6: 27℃ outside, 25℃ and 70%.
        let mut room = Room::new(NOON, Some(27.0), run(Mode::Cool));
        assert_eq!(room.tick_humid(25.0, 70), run(Mode::Dry));
        let mut room = Room::new(NOON, Some(27.0), Some(Command::Off));
        assert_eq!(room.tick_humid(25.0, 70), run(Mode::Dry));
        for last in [Some(Command::Off), run(Mode::Heat)] {
            let mut room = Room::new(NOON, Some(15.0), last);
            assert_eq!(room.tick_humid(25.0, 70), None);
        }
    }

    #[test]
    fn switching_on_starts_at_once_when_the_outside_pushes() {
        // Case 14: 30℃ outside, 25.5℃ inside the band, off.
        for (outdoor, mode) in [(30.0, Mode::Cool), (27.0, Mode::Cool), (5.0, Mode::Heat)] {
            let mut room = Room::new(NOON, Some(outdoor), Some(Command::Off));
            room.memory.switched_at = Some(NOON - MINUTE);
            room.memory.sent_at = Some(NOON - MINUTE);
            assert_eq!(
                room.run_with(25.5, 50, Trigger::Switch),
                run(mode),
                "{outdoor}"
            );
        }
        // 22℃ outside: nothing until the room leaves the band.
        let mut room = Room::new(NOON, Some(22.0), Some(Command::Off));
        assert_eq!(room.run_with(25.5, 50, Trigger::Switch), None);
        assert_eq!(room.ticks(&[25.5, 25.9]), []);
        assert_eq!(room.ticks(&[26.0]), [run(Mode::Cool).unwrap()]);
    }

    // ---- The setpoint follows the meter (case 5) ----

    #[test]
    fn the_setpoint_moves_a_degree_each_half_hour_down_to_three_below() {
        let mut room = Room::new(NOON, Some(30.0), run(Mode::Cool));
        let half_hour = [26.5; 3];
        assert_eq!(room.ticks(&half_hour), []);
        assert_eq!(room.ticks(&[26.5]), [on(Mode::Cool, 24)]);
        // Each new setpoint is watched half an hour from when it was sent.
        assert_eq!(room.ticks(&half_hour[1..]), []);
        assert_eq!(room.ticks(&[26.5]), [on(Mode::Cool, 23)]);
        assert_eq!(room.ticks(&half_hour[1..]), []);
        assert_eq!(room.ticks(&[26.5]), [on(Mode::Cool, 22)]);
        assert_eq!(room.ticks(&[26.5; 12]), []);
        // A restart keeps the correction: off and on again starts at 22℃.
        let kept: Memory =
            serde_json::from_str(&serde_json::to_string(&room.memory).unwrap()).unwrap();
        assert_eq!(kept, room.memory);
        room.settings = OFF;
        assert_eq!(room.run_with(26.5, 50, Trigger::Switch), Some(Command::Off));
        room.settings = ON;
        assert_eq!(
            room.run_with(26.5, 50, Trigger::Switch),
            Some(on(Mode::Cool, 22))
        );
        // A new target keeps the offset.
        room.settings.target = 27;
        assert_eq!(
            room.run_with(26.5, 50, Trigger::Target),
            Some(on(Mode::Cool, 24))
        );
    }

    #[test]
    fn the_setpoint_stays_inside_the_air_conditioner() {
        let mut room = Room::new(NOON, None, Some(on(Mode::Heat, 17)));
        room.settings.target = 18;
        room.memory.heat_offset = -1;
        assert_eq!(room.ticks(&[18.6; 4]), [on(Mode::Heat, 16)]);
        assert_eq!(room.ticks(&[18.6; 12]), []);
    }

    #[test]
    fn a_small_or_short_difference_leaves_the_setpoint() {
        let mut room = Room::new(NOON, Some(30.0), run(Mode::Cool));
        assert_eq!(room.ticks(&[25.4; 12]), []);
        let mut room = Room::new(NOON, Some(30.0), run(Mode::Cool));
        assert_eq!(
            room.ticks(&[25.0, 26.5, 25.0, 25.0, 25.0, 26.5, 25.0, 25.0, 25.0]),
            []
        );
        // Still on its way down, the latest reading near the target.
        let mut room = Room::new(NOON, Some(30.0), run(Mode::Cool));
        assert_eq!(room.ticks(&[27.0, 26.5, 25.8, 25.4]), []);
    }

    // ---- Without the outside (case 7) and the user's switch (case 8) ----

    #[test]
    fn without_the_outside_the_room_alone_decides_and_it_never_stops() {
        {
            let mut room = Room::new(NOON, None, run(Mode::Cool));
            assert_eq!(room.ticks(&[25.0, 24.6, 24.6]), []);
            assert_eq!(room.ticks(&[24.0]), [run(Mode::Heat).unwrap()]);
            room.now += HALF_HOUR;
            assert_eq!(room.ticks(&[25.2, 25.0, 25.4]), []);
            assert_eq!(room.ticks(&[26.0]), [run(Mode::Cool).unwrap()]);
            assert!(room.sends.iter().all(|(_, c)| *c != Command::Off));
        }
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
                    room_only(ON, Some((temperature, 50)), last),
                    want,
                    "{temperature} after {last:?}"
                );
            }
        }
    }

    #[test]
    fn keeps_the_running_mode_inside_the_band() {
        for temperature in [24.1, 25.0, 25.9] {
            assert_eq!(
                room_only(ON, Some((temperature, 50)), run(Mode::Heat)),
                None
            );
            assert_eq!(
                room_only(ON, Some((temperature, 50)), run(Mode::Cool)),
                None
            );
        }
    }

    #[test]
    fn starts_inside_the_band_by_the_side_of_the_target() {
        for last in [None, Some(Command::Off)] {
            assert_eq!(room_only(ON, Some((24.9, 50)), last), run(Mode::Heat));
            assert_eq!(room_only(ON, Some((25.0, 50)), last), run(Mode::Cool));
            assert_eq!(room_only(ON, Some((25.9, 50)), last), run(Mode::Cool));
        }
    }

    #[test]
    fn dries_a_humid_room_inside_the_band_until_it_is_dry_enough() {
        assert_eq!(
            room_only(ON, Some((25.0, 65)), run(Mode::Cool)),
            run(Mode::Dry)
        );
        assert_eq!(room_only(ON, Some((24.5, 70)), None), run(Mode::Dry));
        assert_eq!(room_only(ON, Some((25.0, 64)), run(Mode::Cool)), None);
        assert_eq!(room_only(ON, Some((25.0, 56)), run(Mode::Dry)), None);
        assert_eq!(
            room_only(ON, Some((25.0, 55)), run(Mode::Dry)),
            run(Mode::Cool)
        );
        // Heating dries the air itself; it runs on until the room is warm.
        assert_eq!(room_only(ON, Some((25.0, 80)), run(Mode::Heat)), None);
        // Out of the band the temperature comes first.
        assert_eq!(
            room_only(ON, Some((26.0, 80)), run(Mode::Dry)),
            run(Mode::Cool)
        );
        assert_eq!(
            room_only(ON, Some((24.0, 80)), run(Mode::Dry)),
            run(Mode::Heat)
        );
    }

    #[test]
    fn the_band_follows_the_target() {
        let cooler = Settings { target: 22, ..ON };
        assert_eq!(
            room_only(cooler, Some((23.0, 50)), None),
            Some(on(Mode::Cool, 22))
        );
        assert_eq!(
            room_only(cooler, Some((21.0, 50)), Some(on(Mode::Cool, 22))),
            Some(on(Mode::Heat, 22))
        );
        // A new target is sent at once, in the same mode.
        assert_eq!(
            room_only(cooler, Some((22.5, 50)), run(Mode::Cool)),
            Some(on(Mode::Cool, 22))
        );
    }

    #[test]
    fn leaves_the_air_conditioner_without_a_fresh_reading() {
        for last in [None, run(Mode::Cool), Some(Command::Off)] {
            assert_eq!(room_only(ON, None, last), None);
        }
    }

    #[test]
    fn off_turns_it_off_once_whatever_the_room() {
        for reading in [None, Some((31.0, 80)), Some((15.0, 30))] {
            for last in [None, run(Mode::Cool), run(Mode::Heat)] {
                assert_eq!(room_only(OFF, reading, last), Some(Command::Off));
            }
            assert_eq!(room_only(OFF, reading, Some(Command::Off)), None);
        }
        // Off overrides the half hour between sends too.
        let mut room = Room::new(NOON, Some(30.0), Some(Command::Off));
        room.tick(27.0);
        room.settings = OFF;
        assert_eq!(room.run_with(27.0, 50, Trigger::Switch), Some(Command::Off));
        room.now += 10 * MINUTE;
        assert_eq!(room.ticks(&[30.0; 6]), []);
    }

    #[test]
    fn on_after_off_turns_it_on_for_the_room() {
        assert_eq!(
            room_only(ON, Some((28.0, 50)), Some(Command::Off)),
            run(Mode::Cool)
        );
        assert_eq!(
            room_only(ON, Some((20.0, 50)), Some(Command::Off)),
            run(Mode::Heat)
        );
    }

    // ---- How often it beeps (cases 9, 12, 13) ----

    #[test]
    fn sends_by_itself_at_most_every_half_hour() {
        // Case 9: a room swinging across the band for two days, the outside with it.
        let mut room = Room::new(NOON, Some(22.0), Some(Command::Off));
        for step in 0..288 {
            let swing = [23.5, 24.5, 26.2, 25.4, 23.8, 27.0, 25.0, 24.0, 26.0];
            room.outdoor = Some([22.0, 31.0, 18.0, 26.0][step / 20 % 4]);
            room.tick_humid(swing[step % swing.len()], [50, 70, 60][step / 7 % 3]);
        }
        assert!(room.sends.len() > 10, "{:?}", room.sends);
        for pair in room.sends.windows(2) {
            assert!(pair[1].0 - pair[0].0 >= HALF_HOUR, "{pair:?}");
        }
        // The user's switch and target go out at once.
        let mut room = Room::new(NOON, Some(30.0), Some(Command::Off));
        room.tick(27.0);
        room.settings.target = 24;
        assert_eq!(
            room.run_with(27.0, 50, Trigger::Target),
            Some(on(Mode::Cool, 24))
        );
    }

    #[test]
    fn sends_once_more_when_the_room_does_not_answer_then_reports_it() {
        // Case 12: cooling sent out of the band, and the room stays at 27℃.
        let mut room = Room::new(NOON, Some(30.0), Some(Command::Off));
        assert_eq!(room.ticks(&[27.0, 27.0, 27.0]), [run(Mode::Cool).unwrap()]);
        assert_eq!(room.ticks(&[27.0]), [run(Mode::Cool).unwrap()]);
        assert!(room.reason().contains("送り直す"), "{}", room.reason());
        assert_eq!(room.ticks(&[27.0, 27.0, 27.0, 27.0, 27.0, 27.0, 27.0]), []);
        assert!(
            room.reason().contains("エアコンが信号を受けていない可能性"),
            "{}",
            room.reason()
        );
        // The room moving clears it, and the adjustment goes on without sending it again.
        room.ticks(&[26.5, 26.5, 26.5]);
        assert!(!room.reason().contains("可能性"), "{}", room.reason());
        assert_eq!(
            room.sends
                .iter()
                .filter(|(_, c)| *c == on(Mode::Cool, 25))
                .count(),
            2
        );
        // Heating alike.
        let mut room = Room::new(NOON, Some(10.0), Some(Command::Off));
        assert_eq!(room.ticks(&[23.0, 23.0, 23.0, 23.0]).len(), 2);
        assert_eq!(room.ticks(&[23.0; 6]), []);
        assert!(room.reason().contains("可能性"), "{}", room.reason());
    }

    #[test]
    fn a_room_that_answers_is_not_sent_again() {
        let mut room = Room::new(NOON, Some(30.0), Some(Command::Off));
        assert_eq!(
            room.ticks(&[27.0, 26.5, 25.8, 25.4, 25.2]),
            [run(Mode::Cool).unwrap()]
        );
    }

    #[test]
    fn at_night_only_leaving_the_band_is_sent() {
        // Case 13: from 22:00 a correction or a stop at the target waits for the morning.
        let mut room = Room::new(ELEVEN_PM, Some(30.0), run(Mode::Cool));
        assert_eq!(room.ticks(&[26.5; 12]), []);
        let mut room = Room::new(ELEVEN_PM, Some(22.0), Some(Command::Off));
        assert_eq!(room.ticks(&[26.0]), [run(Mode::Cool).unwrap()]);
        assert_eq!(room.ticks(&[25.0, 24.5, 24.5, 24.5, 24.5]), []);
        assert!(room.reason().contains("夜"), "{}", room.reason());
        // What it would send is kept for the record, though held.
        let held = room.decision.as_ref().unwrap();
        assert_eq!(
            (held.command, held.send, held.hold),
            (Some(Command::Off), None, Some(Hold::Night))
        );
        assert_eq!(room.ticks(&[23.9]), [run(Mode::Heat).unwrap()]);
        // From 07:00 the morning's first chance stops it.
        let mut room = Room::new(NOON - 5 * 3600 - 10 * MINUTE, Some(22.0), run(Mode::Cool));
        assert_eq!(room.ticks(&[24.8]), []);
        assert_eq!(room.ticks(&[24.8]), [Command::Off]);
    }

    #[test]
    fn the_night_may_be_moved_or_dropped() {
        let settings = Settings {
            night_from: 30,
            night_until: 5 * 60,
            ..ON
        };
        assert!(settings.is_night(30) && settings.is_night(299));
        assert!(!settings.is_night(29) && !settings.is_night(300));
        assert!(ON.is_night(22 * 60) && ON.is_night(0) && ON.is_night(7 * 60 - 1));
        assert!(!ON.is_night(7 * 60) && !ON.is_night(22 * 60 - 1));
        let none = Settings {
            night_from: 0,
            night_until: 0,
            ..ON
        };
        assert!((0..1440).all(|m| !none.is_night(m)));
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

    #[test]
    fn settings_default_to_on_at_25_with_the_night_from_22_to_7() {
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), ON);
        assert_eq!(
            serde_json::from_str::<Settings>(r#"{"on":false,"target":25}"#).unwrap(),
            OFF
        );
    }
}
