//! The brightness and colour temperature of lights that are on, every ten minutes.

use serde::{Deserialize, Serialize};

use crate::matter::Light;

/// Asia/Tokyo keeps UTC+9 all year.
const JST: i64 = 9 * 3600;
/// The evening reaches the night's values at 22:00; the morning starts at sunrise, 05:00 at the earliest.
const NIGHT_FROM: u32 = 22 * 60;
const MORNING_FROM: u32 = 5 * 60;

/// The adjustable values; off until the user turns it on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    /// Where sunrise and sunset are computed; Tokyo (Shinjuku) unless the user gives a city.
    pub latitude: f64,
    pub longitude: f64,
    /// When the morning reaches the day's values, in minutes after midnight.
    pub morning_end_minute: u32,
    /// `CurrentLevel` by day and at night, 1–254.
    pub day_level: u8,
    pub night_level: u8,
    /// Warm white at night, cool white by day.
    pub warm_kelvin: u16,
    pub cool_kelvin: u16,
    /// The percentage of the night's level from midnight until the morning starts, 1–100.
    pub late_night_percent: u8,
}

impl Default for Settings {
    /// 80% and 40% of 254, 5000 K and 3000 K, the morning ending at 10:00,
    /// half the night's level after midnight.
    fn default() -> Self {
        Self {
            enabled: false,
            latitude: 35.6895,
            longitude: 139.6917,
            morning_end_minute: 10 * 60,
            day_level: 203,
            night_level: 102,
            warm_kelvin: 3000,
            cool_kelvin: 5000,
            late_night_percent: 50,
        }
    }
}

impl Settings {
    /// Why the settings cannot be used, if they cannot.
    #[must_use]
    pub fn invalid(&self) -> Option<&'static str> {
        if !(-90.0..=90.0).contains(&self.latitude) || !(-180.0..=180.0).contains(&self.longitude) {
            Some("緯度は-90〜90、経度は-180〜180で指定してください")
        } else if !(1..=254).contains(&self.day_level) || !(1..=254).contains(&self.night_level) {
            Some("明るさは1〜254で指定してください")
        } else if !(1000..=10000).contains(&self.warm_kelvin)
            || !(1000..=10000).contains(&self.cool_kelvin)
            || self.warm_kelvin > self.cool_kelvin
        {
            Some("色温度は1000〜10000Kで、warm_kelvinをcool_kelvin以下にしてください")
        } else if !(MORNING_FROM + 1..NIGHT_FROM).contains(&self.morning_end_minute) {
            Some("morning_end_minuteは301〜1319（05:01〜21:59）で指定してください")
        } else if !(1..=100).contains(&self.late_night_percent) {
            Some("late_night_percentは1〜100で指定してください")
        } else {
            None
        }
    }
}

/// A label's own values for its lights; `None` keeps the schedule's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub day_level: Option<u8>,
    pub night_level: Option<u8>,
    pub cool_kelvin: Option<u16>,
    pub warm_kelvin: Option<u16>,
}

impl Settings {
    /// These settings with `label`'s values in place of the schedule's.
    #[must_use]
    pub fn with(&self, label: &Label) -> Self {
        Self {
            day_level: label.day_level.unwrap_or(self.day_level),
            night_level: label.night_level.unwrap_or(self.night_level),
            cool_kelvin: label.cool_kelvin.unwrap_or(self.cool_kelvin),
            warm_kelvin: label.warm_kelvin.unwrap_or(self.warm_kelvin),
            ..self.clone()
        }
    }
}

/// The day of the year (1-based) and the minute of the day in Asia/Tokyo at `unix` seconds.
#[must_use]
pub fn local(unix: i64) -> (u32, u32) {
    let local = unix + JST;
    let days = local.div_euclid(86_400);
    let minute = u32::try_from(local.rem_euclid(86_400) / 60).unwrap_or(0);
    let year_start = days_from_civil(civil_year(days), 1, 1);
    (u32::try_from(days - year_start + 1).unwrap_or(1), minute)
}

/// `unix` seconds as an Asia/Tokyo timestamp, `2026-10-05T23:00:00+09:00`.
#[must_use]
pub fn timestamp(unix: i64) -> String {
    let local = unix + JST;
    let (year, month, day) = civil(local.div_euclid(86_400));
    let seconds = local.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+09:00",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// `minute` of the day as `HH:MM`.
#[must_use]
pub fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

// Howard Hinnant's civil calendar algorithms.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn civil_year(days: i64) -> i64 {
    civil(days).0
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Sunrise and sunset as minutes of the day in Asia/Tokyo (NOAA's approximation,
/// within a few minutes). Polar day and night clamp to the whole or no day.
// The values are rounded and clamped into range before the casts.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn sun_times(day_of_year: u32, latitude: f64, longitude: f64) -> (u32, u32) {
    let g = 2.0 * std::f64::consts::PI / 365.0 * f64::from(day_of_year.saturating_sub(1));
    let eqtime = 229.18
        * (0.000_075 + 0.001_868 * g.cos()
            - 0.032_077 * g.sin()
            - 0.014_615 * (2.0 * g).cos()
            - 0.040_849 * (2.0 * g).sin());
    let decl = 0.006_918 - 0.399_912 * g.cos() + 0.070_257 * g.sin() - 0.006_758 * (2.0 * g).cos()
        + 0.000_907 * (2.0 * g).sin()
        - 0.002_697 * (3.0 * g).cos()
        + 0.001_48 * (3.0 * g).sin();
    let lat = latitude.to_radians();
    let cos_ha = 90.833_f64.to_radians().cos() / (lat.cos() * decl.cos()) - lat.tan() * decl.tan();
    let ha = cos_ha.clamp(-1.0, 1.0).acos().to_degrees();
    let noon = 720.0 - 4.0 * longitude - eqtime + 540.0;
    let minute = |m: f64| m.round().clamp(0.0, 1439.0) as u32;
    (minute(noon - 4.0 * ha), minute(noon + 4.0 * ha))
}

/// The level and colour temperature in kelvin for `minute` of the day, given
/// `sun` as sunrise and sunset minutes: the night's values rise linearly from
/// the morning's start to the day's by the morning end, and fall back from
/// sunset to 22:00. From midnight until the morning starts, the night's level
/// is scaled by `late_night_percent`, never below 1.
// The values are rounded and clamped into range before the casts.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn target(minute: u32, sun: (u32, u32), settings: &Settings) -> (u8, u16) {
    let start = sun.0.max(MORNING_FROM);
    let between = |from: u32, to: u32| f64::from(minute - from) / f64::from(to - from);
    let day = if minute >= NIGHT_FROM || minute < start {
        0.0
    } else if minute < settings.morning_end_minute {
        between(start, settings.morning_end_minute)
    } else if minute < sun.1 {
        1.0
    } else {
        1.0 - between(sun.1, NIGHT_FROM)
    };
    let lerp = |night: f64, by_day: f64| (night + (by_day - night) * day).round();
    let night_level = if minute < start {
        (f64::from(settings.night_level) * f64::from(settings.late_night_percent) / 100.0)
            .round()
            .max(1.0)
    } else {
        f64::from(settings.night_level)
    };
    (
        lerp(night_level, f64::from(settings.day_level)) as u8,
        lerp(
            f64::from(settings.warm_kelvin),
            f64::from(settings.cool_kelvin),
        ) as u16,
    )
}

/// What to send one light; `None` where it does not support the control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub level: Option<u8>,
    pub mireds: Option<u16>,
}

/// Fits `level` and `kelvin` into the ranges `light` reports.
// The values are rounded and clamped into range before the casts.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn fit(level: u8, kelvin: u16, light: &Light) -> Target {
    Target {
        level: light
            .levels
            .map(|(min, max)| level.clamp(min.max(1), max.max(min.max(1)))),
        mireds: light.mireds.map(|(min, max)| {
            let mireds = (1_000_000.0 / f64::from(kelvin)).round() as u16;
            mireds.clamp(min, max.max(min))
        }),
    }
}

/// Why a light was left alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Skip {
    /// "All off" was pressed last; nothing is written until "all on".
    AllOff,
    Disabled,
    NoResponse,
    /// Supports neither level nor colour temperature.
    Unsupported,
    /// Already sent these values.
    Unchanged,
}

/// What to send `light` for the `goal` level and kelvin, before its On/Off is
/// read again: only the values that differ from `last` sent, or why to send nothing.
///
/// # Errors
/// The reason the light is left alone.
pub fn plan(
    light: &Light,
    all_off: bool,
    settings: &Settings,
    goal: (u8, u16),
    last: Option<Target>,
) -> Result<Target, Skip> {
    let target = fit(goal.0, goal.1, light);
    let last = last.unwrap_or(Target {
        level: None,
        mireds: None,
    });
    let changed = Target {
        level: target.level.filter(|v| last.level != Some(*v)),
        mireds: target.mireds.filter(|v| last.mireds != Some(*v)),
    };
    if all_off {
        Err(Skip::AllOff)
    } else if !settings.enabled {
        Err(Skip::Disabled)
    } else if !light.reachable {
        Err(Skip::NoResponse)
    } else if target.level.is_none() && target.mireds.is_none() {
        Err(Skip::Unsupported)
    } else if changed.level.is_none() && changed.mireds.is_none() {
        Err(Skip::Unchanged)
    } else {
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO: (f64, f64) = (35.6895, 139.6917);

    fn at(hour: u32, minute: u32) -> u32 {
        hour * 60 + minute
    }

    fn near(actual: u32, expected: u32) -> bool {
        actual.abs_diff(expected) <= 3
    }

    #[test]
    fn local_time_is_tokyo_day_of_year_and_minute() {
        // 2026-10-05T14:00:00Z is 23:00 in Tokyo, day 278.
        assert_eq!(local(1_791_208_800), (278, at(23, 0)));
        // 2026-12-31T15:30:00Z is 00:30 on 2027-01-01 in Tokyo.
        assert_eq!(local(1_798_731_000), (1, 30));
        // 2028 is a leap year: 12-31 is day 366.
        assert_eq!(local(1_861_844_400), (366, at(12, 0)));
        assert_eq!(timestamp(1_791_208_800), "2026-10-05T23:00:00+09:00");
        assert_eq!(clock(at(5, 7)), "05:07");
    }

    #[test]
    fn sunrise_and_sunset_in_tokyo_match_the_almanac() {
        // National Astronomical Observatory of Japan, Tokyo.
        let (rise, set) = sun_times(278, TOKYO.0, TOKYO.1); // 10-05: 05:39, 17:24
        assert!(
            near(rise, at(5, 39)) && near(set, at(17, 24)),
            "{rise} {set}"
        );
        let (rise, set) = sun_times(172, TOKYO.0, TOKYO.1); // 06-21: 04:25, 19:00
        assert!(
            near(rise, at(4, 25)) && near(set, at(19, 0)),
            "{rise} {set}"
        );
        let (rise, set) = sun_times(356, TOKYO.0, TOKYO.1); // 12-22: 06:47, 16:32
        assert!(
            near(rise, at(6, 47)) && near(set, at(16, 32)),
            "{rise} {set}"
        );
        // Polar night and day do not wrap around.
        assert_eq!(sun_times(356, 80.0, 0.0).0, sun_times(356, 80.0, 0.0).1);
        let (rise, set) = sun_times(172, 80.0, 139.0);
        assert!(rise < set);
    }

    fn day() -> Settings {
        Settings::default()
    }

    #[test]
    fn defaults_are_80_and_40_percent_of_254_and_5000_to_3000_kelvin() {
        let s = day();
        assert_eq!((s.day_level, s.night_level), (203, 102));
        assert_eq!((s.cool_kelvin, s.warm_kelvin), (5000, 3000));
        assert_eq!(s.morning_end_minute, at(10, 0));
        assert_eq!(s.late_night_percent, 50);
    }

    #[test]
    fn night_from_22_until_the_morning_starts_is_dim_and_warm() {
        let s = day();
        let sun = (at(5, 39), at(17, 24));
        assert_eq!(target(at(22, 0), sun, &s), (102, 3000));
        assert_eq!(target(at(23, 59), sun, &s), (102, 3000));
        // From midnight the level is halved, the colour kept.
        assert_eq!(target(at(0, 0), sun, &s), (51, 3000));
        assert_eq!(target(at(2, 0), sun, &s), (51, 3000));
        assert_eq!(target(at(5, 38), sun, &s), (51, 3000));
        // The morning rises from the night's usual values.
        assert_eq!(target(at(5, 39), sun, &s), (102, 3000));
        // The evening reaches the night's values at 22:00.
        assert_eq!(target(at(21, 59), sun, &s).0, 102);
    }

    #[test]
    fn mornings_rise_linearly_from_sunrise_to_the_morning_end() {
        let s = day();
        let sun = (at(6, 0), at(17, 0));
        assert_eq!(target(at(5, 59), sun, &s), (51, 3000));
        assert_eq!(target(at(6, 0), sun, &s), (102, 3000));
        // Halfway from 06:00 to 10:00.
        assert_eq!(target(at(8, 0), sun, &s), (153, 4000));
        assert_eq!(target(at(10, 0), sun, &s), (203, 5000));
        assert_eq!(target(at(12, 0), sun, &s), (203, 5000));
        assert_eq!(target(at(16, 59), sun, &s), (203, 5000));
    }

    #[test]
    fn mornings_start_at_5_when_the_sun_rises_earlier() {
        let s = day();
        let sun = (at(4, 25), at(19, 0));
        assert_eq!(target(at(4, 30), sun, &s), (51, 3000));
        assert_eq!(target(at(5, 0), sun, &s), (102, 3000));
        // A fifth of 05:00 to 10:00.
        assert_eq!(target(at(6, 0), sun, &s), (122, 3400));
    }

    #[test]
    fn evenings_fall_linearly_from_sunset_to_22() {
        let s = day();
        let sun = (at(6, 0), at(18, 0));
        assert_eq!(target(at(18, 0), sun, &s), (203, 5000));
        // Halfway from 18:00 to 22:00.
        assert_eq!(target(at(20, 0), sun, &s), (153, 4000));
        assert_eq!(target(at(21, 0), sun, &s), (127, 3500));
        assert_eq!(target(at(22, 0), sun, &s), (102, 3000));
    }

    #[test]
    fn the_morning_end_is_a_setting() {
        let s = Settings {
            morning_end_minute: at(8, 0),
            ..day()
        };
        let sun = (at(6, 0), at(17, 0));
        assert_eq!(target(at(7, 0), sun, &s), (153, 4000));
        assert_eq!(target(at(8, 0), sun, &s), (203, 5000));
        // A sunrise after the morning end jumps to the day's values.
        assert_eq!(target(at(9, 0), (at(9, 30), at(17, 0)), &s), (51, 3000));
        assert_eq!(target(at(9, 30), (at(9, 30), at(17, 0)), &s), (203, 5000));
    }

    fn bulb(levels: Option<(u8, u8)>, mireds: Option<(u16, u16)>) -> Light {
        Light {
            node_id: 1,
            endpoint: 1,
            bridged: false,
            reachable: true,
            on: Some(true),
            product: None,
            levels,
            mireds,
        }
    }

    #[test]
    fn targets_fit_each_lights_range() {
        // T2: 153–370 mired. 2700 K is 370; 2000 K clamps to it, 10000 K to 153.
        let t2 = bulb(Some((1, 254)), Some((153, 370)));
        assert_eq!(
            fit(40, 2700, &t2),
            Target {
                level: Some(40),
                mireds: Some(370)
            }
        );
        assert_eq!(fit(40, 2000, &t2).mireds, Some(370));
        assert_eq!(fit(40, 10000, &t2).mireds, Some(153));
        // Tapo reports MinLevel 0: level 1 is the lowest sent.
        assert_eq!(fit(0, 5000, &bulb(Some((0, 254)), None)).level, Some(1));
        assert_eq!(fit(254, 5000, &bulb(Some((1, 200)), None)).level, Some(200));
        // A light without the control gets no value for it.
        assert_eq!(
            fit(40, 5000, &bulb(None, None)),
            Target {
                level: None,
                mireds: None
            }
        );
    }

    fn on() -> Settings {
        Settings {
            enabled: true,
            ..Settings::default()
        }
    }

    #[test]
    fn all_off_and_disabled_leave_every_light_alone() {
        let t2 = bulb(Some((1, 254)), Some((153, 370)));
        let night = (102, 3000);
        assert_eq!(plan(&t2, true, &on(), night, None), Err(Skip::AllOff));
        // "All off" wins over disabled, so the status shows why nothing was sent.
        assert_eq!(
            plan(&t2, true, &Settings::default(), night, None),
            Err(Skip::AllOff)
        );
        assert_eq!(
            plan(&t2, false, &Settings::default(), night, None),
            Err(Skip::Disabled)
        );
        let unreachable = Light {
            reachable: false,
            ..t2.clone()
        };
        assert_eq!(
            plan(&unreachable, false, &on(), night, None),
            Err(Skip::NoResponse)
        );
        assert_eq!(
            plan(&bulb(None, None), false, &on(), night, None),
            Err(Skip::Unsupported)
        );
        assert_eq!(
            plan(&t2, false, &on(), night, None),
            Ok(Target {
                level: Some(102),
                mireds: Some(333)
            })
        );
    }

    #[test]
    fn only_values_that_changed_since_the_last_send_are_sent() {
        let t2 = bulb(Some((1, 254)), Some((153, 370)));
        let sent = Target {
            level: Some(102),
            mireds: Some(333),
        };
        assert_eq!(
            plan(&t2, false, &on(), (102, 3000), Some(sent)),
            Err(Skip::Unchanged)
        );
        assert_eq!(
            plan(&t2, false, &on(), (105, 3000), Some(sent)),
            Ok(Target {
                level: Some(105),
                mireds: None
            })
        );
        assert_eq!(
            plan(&t2, false, &on(), (102, 3100), Some(sent)),
            Ok(Target {
                level: None,
                mireds: Some(323)
            })
        );
    }

    fn kitchen() -> Label {
        Label {
            night_level: Some(76),
            warm_kelvin: Some(2700),
            ..Label::default()
        }
    }

    #[test]
    fn a_labels_values_replace_the_schedules_and_nulls_keep_them() {
        let s = day();
        assert_eq!(s.with(&Label::default()), s);
        let merged = s.with(&kitchen());
        assert_eq!((merged.day_level, merged.night_level), (s.day_level, 76));
        assert_eq!(
            (merged.cool_kelvin, merged.warm_kelvin),
            (s.cool_kelvin, 2700)
        );
        let all = Label {
            day_level: Some(150),
            night_level: Some(50),
            cool_kelvin: Some(4500),
            warm_kelvin: Some(3500),
        };
        let merged = s.with(&all);
        assert_eq!(
            (
                merged.day_level,
                merged.night_level,
                merged.cool_kelvin,
                merged.warm_kelvin
            ),
            (150, 50, 4500, 3500)
        );
        // The times stay the schedule's.
        assert_eq!(merged.morning_end_minute, s.morning_end_minute);
        assert_eq!((merged.latitude, merged.enabled), (s.latitude, s.enabled));
    }

    #[test]
    fn a_label_follows_the_schedules_curve_with_its_own_values() {
        let s = day().with(&kitchen());
        let sun = (at(6, 0), at(18, 0));
        // Night from 22:00 until sunrise, the day's values from the morning end.
        assert_eq!(target(at(22, 0), sun, &s), (76, 2700));
        assert_eq!(target(at(5, 59), sun, &s), (38, 2700));
        assert_eq!(target(at(6, 0), sun, &s), (76, 2700));
        assert_eq!(target(at(10, 0), sun, &s), (203, 5000));
        // Halfway through the morning and the evening.
        assert_eq!(target(at(8, 0), sun, &s), (140, 3850));
        assert_eq!(target(at(20, 0), sun, &s), (140, 3850));
        // A minute before 22:00 is a 240th of the way back up.
        assert_eq!(target(at(21, 59), sun, &s).0, 77);
    }

    #[test]
    fn after_midnight_each_lights_night_level_is_scaled_by_the_percent() {
        let s = Settings {
            night_level: 51,
            warm_kelvin: 2700,
            ..day()
        };
        let sun = (at(5, 39), at(17, 24));
        let hallway = Label {
            night_level: Some(64),
            ..Label::default()
        };
        let game_pc = Label {
            warm_kelvin: Some(3500),
            ..Label::default()
        };
        assert_eq!(target(at(1, 0), sun, &s.with(&kitchen())), (38, 2700));
        assert_eq!(target(at(1, 0), sun, &s.with(&hallway)), (32, 2700));
        // 25.5 rounds up.
        assert_eq!(target(at(1, 0), sun, &s), (26, 2700));
        assert_eq!(target(at(1, 0), sun, &s.with(&game_pc)), (26, 3500));
        // 100% leaves the night as it is; the lowest level sent stays 1.
        let full = Settings {
            late_night_percent: 100,
            ..s.clone()
        };
        assert_eq!(target(at(1, 0), sun, &full), (51, 2700));
        let faint = Settings {
            night_level: 1,
            late_night_percent: 1,
            ..s
        };
        assert_eq!(target(at(1, 0), sun, &faint), (1, 2700));
    }

    #[test]
    fn labels_out_of_range_or_warmer_than_cool_are_rejected() {
        let s = day();
        assert_eq!(s.with(&kitchen()).invalid(), None);
        for bad in [
            Label {
                day_level: Some(0),
                ..Label::default()
            },
            Label {
                night_level: Some(255),
                ..Label::default()
            },
            Label {
                warm_kelvin: Some(999),
                ..Label::default()
            },
            Label {
                cool_kelvin: Some(10001),
                ..Label::default()
            },
            Label {
                warm_kelvin: Some(4000),
                cool_kelvin: Some(3500),
                ..Label::default()
            },
            // Warmer than the schedule's cool white.
            Label {
                warm_kelvin: Some(5500),
                ..Label::default()
            },
        ] {
            assert!(s.with(&bad).invalid().is_some(), "{bad:?}");
        }
    }

    #[test]
    fn settings_are_validated() {
        assert_eq!(Settings::default().invalid(), None);
        for bad in [
            Settings {
                latitude: 91.0,
                ..Settings::default()
            },
            Settings {
                night_level: 0,
                ..Settings::default()
            },
            Settings {
                day_level: 255,
                ..Settings::default()
            },
            Settings {
                warm_kelvin: 6000,
                ..Settings::default()
            },
            Settings {
                morning_end_minute: 5 * 60,
                ..Settings::default()
            },
            Settings {
                morning_end_minute: 22 * 60,
                ..Settings::default()
            },
            Settings {
                late_night_percent: 0,
                ..Settings::default()
            },
            Settings {
                late_night_percent: 101,
                ..Settings::default()
            },
        ] {
            assert!(bad.invalid().is_some(), "{bad:?}");
        }
    }
}
