//! The hourly brightness and colour temperature of lights that are on.

use serde::{Deserialize, Serialize};

use crate::matter::Light;

/// Asia/Tokyo keeps UTC+9 all year.
const JST: i64 = 9 * 3600;
/// Dim from 22:00 until 05:00.
const NIGHT_FROM: u32 = 22 * 60;
const NIGHT_UNTIL: u32 = 5 * 60;
/// Brighten and cool over this long after sunrise, warm over this long before sunset.
const RAMP: f64 = 120.0;

/// The adjustable values; off until the user turns it on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    /// Where sunrise and sunset are computed; Tokyo (Shinjuku) unless the user gives a city.
    pub latitude: f64,
    pub longitude: f64,
    /// `CurrentLevel` by day and from 22:00 to 05:00, 1–254.
    pub day_level: u8,
    pub night_level: u8,
    /// Warm white at night and in the evening, cool white by day.
    pub warm_kelvin: u16,
    pub cool_kelvin: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            latitude: 35.6895,
            longitude: 139.6917,
            day_level: 254,
            night_level: 40,
            warm_kelvin: 2700,
            cool_kelvin: 5000,
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
        } else {
            None
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
/// `sun` as sunrise and sunset minutes.
// The values are rounded and clamped into range before the casts.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn target(minute: u32, sun: (u32, u32), settings: &Settings) -> (u8, u16) {
    if !(NIGHT_UNTIL..NIGHT_FROM).contains(&minute) {
        return (settings.night_level, settings.warm_kelvin);
    }
    let (minute, sunrise, sunset) = (f64::from(minute), f64::from(sun.0), f64::from(sun.1));
    let rise = ((minute - sunrise) / RAMP).clamp(0.0, 1.0);
    let set = ((sunset - minute) / RAMP).clamp(0.0, 1.0);
    let lerp = |from: f64, to: f64, f: f64| (from + (to - from) * f).round();
    let level = lerp(
        f64::from(settings.night_level),
        f64::from(settings.day_level),
        rise,
    );
    let kelvin = lerp(
        f64::from(settings.warm_kelvin),
        f64::from(settings.cool_kelvin),
        rise.min(set),
    );
    (level as u8, kelvin as u16)
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
}

/// What to send `light`, before its On/Off is read again, or why to send nothing.
///
/// # Errors
/// The reason the light is left alone.
pub fn plan(
    light: &Light,
    all_off: bool,
    settings: &Settings,
    level: u8,
    kelvin: u16,
) -> Result<Target, Skip> {
    let target = fit(level, kelvin, light);
    if all_off {
        Err(Skip::AllOff)
    } else if !settings.enabled {
        Err(Skip::Disabled)
    } else if !light.reachable {
        Err(Skip::NoResponse)
    } else if target.level.is_none() && target.mireds.is_none() {
        Err(Skip::Unsupported)
    } else {
        Ok(target)
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

    #[test]
    fn night_from_22_to_5_is_dim_and_warm() {
        let s = Settings::default();
        let sun = (at(5, 39), at(17, 24));
        assert_eq!(target(at(22, 0), sun, &s), (40, 2700));
        assert_eq!(target(at(2, 0), sun, &s), (40, 2700));
        assert_eq!(target(at(4, 59), sun, &s), (40, 2700));
        // The evening before 22:00 keeps the day's level, warm.
        assert_eq!(target(at(21, 59), sun, &s), (254, 2700));
    }

    #[test]
    fn mornings_brighten_and_cool_from_sunrise() {
        let s = Settings::default();
        let sun = (at(6, 0), at(17, 0));
        // 05:00 is before sunrise: still the night's values.
        assert_eq!(target(at(5, 0), sun, &s), (40, 2700));
        assert_eq!(target(at(6, 0), sun, &s), (40, 2700));
        assert_eq!(target(at(7, 0), sun, &s), (147, 3850));
        assert_eq!(target(at(8, 0), sun, &s), (254, 5000));
        assert_eq!(target(at(12, 0), sun, &s), (254, 5000));
    }

    #[test]
    fn evenings_warm_toward_sunset() {
        let s = Settings::default();
        let sun = (at(6, 0), at(17, 0));
        assert_eq!(target(at(15, 0), sun, &s), (254, 5000));
        assert_eq!(target(at(16, 0), sun, &s), (254, 3850));
        assert_eq!(target(at(17, 0), sun, &s), (254, 2700));
        assert_eq!(target(at(19, 0), sun, &s), (254, 2700));
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

    #[test]
    fn all_off_and_disabled_leave_every_light_alone() {
        let on = Settings {
            enabled: true,
            ..Settings::default()
        };
        let t2 = bulb(Some((1, 254)), Some((153, 370)));
        assert_eq!(plan(&t2, true, &on, 40, 2700), Err(Skip::AllOff));
        // "All off" wins over disabled, so the status shows why nothing was sent.
        assert_eq!(
            plan(&t2, true, &Settings::default(), 40, 2700),
            Err(Skip::AllOff)
        );
        assert_eq!(
            plan(&t2, false, &Settings::default(), 40, 2700),
            Err(Skip::Disabled)
        );
        let unreachable = Light {
            reachable: false,
            ..t2.clone()
        };
        assert_eq!(
            plan(&unreachable, false, &on, 40, 2700),
            Err(Skip::NoResponse)
        );
        assert_eq!(
            plan(&bulb(None, None), false, &on, 40, 2700),
            Err(Skip::Unsupported)
        );
        assert_eq!(
            plan(&t2, false, &on, 40, 2700),
            Ok(Target {
                level: Some(40),
                mireds: Some(370)
            })
        );
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
        ] {
            assert!(bad.invalid().is_some(), "{bad:?}");
        }
    }
}
