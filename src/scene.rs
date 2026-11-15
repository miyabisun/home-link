//! The home's routines, such as "おやすみ" and "帰宅": each a name and the steps it runs in order,
//! written in YAML so that they change without a build. A step is one small thing home-link
//! already does (a label on or off, the door, a PC, an HTTP request) or a wait for a registered
//! phone to be near home.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_norway::Value;

use crate::sesame;

/// The routines of a home that has written none: the four names, the door only.
pub const DEFAULT: &str = "\
# 操作ごとに、上から順に実行するステップを書く。
#   - on: <label>          labelを入れる
#   - off: <label>         labelを切る
#   - door: lock           玄関を施錠する（unlock で解錠）
#   - sleep: <PC>          PCをスリープさせる（wake で起こす）
#   - http: POST <URL>     HTTPの要求を送る（body: にJSONを書ける）
#   - near: 100            登録した電話が自宅から100m以内に入るまで、ここで待つ
#     wait: 30             待つ時間（分）。過ぎたら残りは実行しない
おやすみ:
  - door: lock
おはよう: []
外出:
  - door: lock
帰宅:
  - near: 100
    wait: 30
  - door: unlock
";

/// The longest routines document kept, in bytes.
pub const MAX_BYTES: usize = 16 * 1024;
/// How long a phone waits at a `near` step that names no `wait`, in minutes.
pub const DEFAULT_WAIT: u32 = 30;
/// The longest `wait`, in minutes.
pub const MAX_WAIT: u32 = 180;
/// How old a phone's fix may be to say where it is, in milliseconds.
pub const FRESH_MS: u64 = 60_000;

/// One routine: its name and steps, in the order written.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub name: String,
    pub steps: Vec<Step>,
}

/// One step of a routine.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Switches the label named `name`.
    Label {
        name: String,
        on: bool,
    },
    Door(sesame::Command),
    /// Has the PC named `name` sleep, or wake.
    Pc {
        name: String,
        sleep: bool,
    },
    Http {
        method: String,
        url: String,
        body: Option<serde_json::Value>,
    },
    /// Waits for a registered phone within `metres` of home, for `wait` minutes at most.
    Near {
        metres: f64,
        wait: u32,
    },
}

impl fmt::Display for Step {
    /// The step as it is written in YAML, on one line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Label { name, on: true } => write!(f, "on: {name}"),
            Self::Label { name, on: false } => write!(f, "off: {name}"),
            Self::Door(sesame::Command::Lock) => write!(f, "door: lock"),
            Self::Door(sesame::Command::Unlock) => write!(f, "door: unlock"),
            Self::Pc { name, sleep: true } => write!(f, "sleep: {name}"),
            Self::Pc { name, sleep: false } => write!(f, "wake: {name}"),
            Self::Http { method, url, .. } => write!(f, "http: {method} {url}"),
            Self::Near { metres, .. } => write!(f, "near: {metres}"),
        }
    }
}

/// The keys that make a step what it is; a step has exactly one.
const ACTIONS: [&str; 7] = ["on", "off", "door", "sleep", "wake", "http", "near"];
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
/// The farthest `near` asks for, in metres.
const MAX_METRES: f64 = 100_000.0;

/// The routines `yaml` writes, in its order.
///
/// # Errors
/// What is wrong with the document, in words for whoever wrote it.
pub fn parse(yaml: &str) -> Result<Vec<Scene>, String> {
    if yaml.len() > MAX_BYTES {
        return Err(format!("長すぎます（{MAX_BYTES}バイトまで）"));
    }
    let document: Value =
        serde_norway::from_str(yaml).map_err(|e| format!("YAMLとして読めません: {e}"))?;
    let routines = match document {
        Value::Null => return Ok(Vec::new()),
        Value::Mapping(routines) => routines,
        _ => return Err("操作の名前と、そのステップの並びを書いてください".into()),
    };
    routines
        .iter()
        .map(|(name, steps)| {
            let name = text(name)
                .filter(|name| !name.is_empty())
                .ok_or("操作の名前が空です")?;
            let steps = match steps {
                Value::Null => &Vec::new(),
                Value::Sequence(steps) => steps,
                _ => return Err(format!("「{name}」: ステップを「- 」で並べてください")),
            };
            let steps = steps
                .iter()
                .enumerate()
                .map(|(index, written)| {
                    step(written).map_err(|wrong| format!("「{name}」の{}番目: {wrong}", index + 1))
                })
                .collect::<Result<_, _>>()?;
            Ok(Scene { name, steps })
        })
        .collect()
}

/// A scalar as the name it writes: `room`, or `7` for a label named so.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn step(written: &Value) -> Result<Step, String> {
    let one = || format!("{} のどれか1つを書いてください", ACTIONS.join("・"));
    let Value::Mapping(written) = written else {
        return Err(one());
    };
    let keys: Vec<String> = written.keys().filter_map(text).collect();
    let mut actions = keys.iter().filter(|key| ACTIONS.contains(&key.as_str()));
    let (Some(action), None, true) = (actions.next(), actions.next(), keys.len() == written.len())
    else {
        return Err(one());
    };
    let option = match action.as_str() {
        "http" => "body",
        "near" => "wait",
        _ => "",
    };
    if let Some(extra) = keys.iter().find(|key| *key != action && *key != option) {
        return Err(format!("{extra} は使えません"));
    }
    let value = &written[action.as_str()];
    let name = || {
        text(value)
            .filter(|name| !name.is_empty())
            .ok_or(format!("{action} に名前を書いてください"))
    };
    Ok(match action.as_str() {
        "on" | "off" => Step::Label {
            name: name()?,
            on: action == "on",
        },
        "sleep" | "wake" => Step::Pc {
            name: name()?,
            sleep: action == "sleep",
        },
        "door" => Step::Door(match value.as_str() {
            Some("lock") => sesame::Command::Lock,
            Some("unlock") => sesame::Command::Unlock,
            _ => return Err("door は lock か unlock です".into()),
        }),
        "http" => {
            let request = value.as_str().and_then(|request| {
                let (method, url) = request.trim().split_once(' ')?;
                let (method, url) = (method.to_uppercase(), url.trim());
                let rest = url
                    .strip_prefix("http://")
                    .or_else(|| url.strip_prefix("https://"))?;
                (METHODS.contains(&method.as_str()) && !rest.is_empty())
                    .then(|| (method, url.to_owned()))
            });
            let (method, url) = request.ok_or(format!(
                "http は「POST http://…」の形で書いてください（{}）",
                METHODS.join("・")
            ))?;
            let body = written
                .get("body")
                .map(serde_json::to_value)
                .transpose()
                .map_err(|_| "body をJSONにできません")?;
            Step::Http { method, url, body }
        }
        _ => {
            let metres = value
                .as_f64()
                .filter(|metres| *metres > 0.0 && *metres <= MAX_METRES)
                .ok_or("near は0より大きいメートルで書いてください")?;
            let wait = match written.get("wait") {
                None => DEFAULT_WAIT,
                Some(wait) => wait
                    .as_u64()
                    .and_then(|wait| u32::try_from(wait).ok())
                    .filter(|wait| (1..=MAX_WAIT).contains(wait))
                    .ok_or(format!("wait は1〜{MAX_WAIT}の分で書いてください"))?,
            };
            Step::Near { metres, wait }
        }
    })
}

/// Where a phone says it is: degrees, the radius in metres it vouches for, and how long ago
/// it measured, by its own clock.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    pub age_ms: u64,
}

/// Why a fix does not put the phone near home.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Away {
    /// Measured longer ago than [`FRESH_MS`].
    Stale,
    /// Vouched for no closer than this many metres, more than the radius asked.
    Vague(f64),
    /// This many metres from home.
    Far(f64),
}

/// The mean radius of the Earth, in metres.
const EARTH: f64 = 6_371_008.8;

/// The metres between two places given as latitude and longitude in degrees.
#[must_use]
pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat_a, lat_b) = (a.0.to_radians(), b.0.to_radians());
    let half = |degrees: f64| (degrees.to_radians() / 2.0).sin().powi(2);
    let chord = half(b.0 - a.0) + lat_a.cos() * lat_b.cos() * half(b.1 - a.1);
    2.0 * EARTH * chord.sqrt().min(1.0).asin()
}

/// The metres from `home` to `fix` when it is fresh, sure to `metres` and within them.
///
/// # Errors
/// Why the fix does not put the phone within `metres` of `home`.
pub fn near(home: (f64, f64), fix: &Fix, metres: f64) -> Result<f64, Away> {
    if fix.age_ms > FRESH_MS {
        return Err(Away::Stale);
    }
    if fix.accuracy > metres {
        return Err(Away::Vague(fix.accuracy));
    }
    let away = distance(home, (fix.latitude, fix.longitude));
    if away > metres {
        return Err(Away::Far(away));
    }
    Ok(away)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(yaml: &str) -> Vec<Step> {
        let scenes = parse(yaml).unwrap();
        assert_eq!(scenes.len(), 1, "{scenes:?}");
        scenes.into_iter().next().unwrap().steps
    }

    #[test]
    fn the_default_has_the_four_routines_in_order() {
        let scenes = parse(DEFAULT).unwrap();
        let names: Vec<&str> = scenes.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["おやすみ", "おはよう", "外出", "帰宅"]);
        assert_eq!(scenes[0].steps, [Step::Door(sesame::Command::Lock)]);
        assert_eq!(scenes[1].steps, []);
        assert_eq!(
            scenes[3].steps,
            [
                Step::Near {
                    metres: 100.0,
                    wait: 30
                },
                Step::Door(sesame::Command::Unlock)
            ]
        );
    }

    #[test]
    fn each_kind_of_step_is_read() {
        let yaml = "
帰宅:
  - on: 空調
  - off: room
  - door: lock
  - sleep: desk
  - wake: desk
  - http: POST http://127.0.0.1:5009/api/notify
    body: { to: discord, count: 3 }
  - http: get https://example.com/a?b=1
  - near: 50.5
  - near: 100
    wait: 5
  - on: 7
";
        let label = |name: &str, on| Step::Label {
            name: name.into(),
            on,
        };
        let pc = |sleep| Step::Pc {
            name: "desk".into(),
            sleep,
        };
        assert_eq!(
            steps(yaml),
            [
                label("空調", true),
                label("room", false),
                Step::Door(sesame::Command::Lock),
                pc(true),
                pc(false),
                Step::Http {
                    method: "POST".into(),
                    url: "http://127.0.0.1:5009/api/notify".into(),
                    body: Some(serde_json::json!({ "to": "discord", "count": 3 })),
                },
                Step::Http {
                    method: "GET".into(),
                    url: "https://example.com/a?b=1".into(),
                    body: None,
                },
                Step::Near {
                    metres: 50.5,
                    wait: DEFAULT_WAIT
                },
                Step::Near {
                    metres: 100.0,
                    wait: 5
                },
                label("7", true),
            ]
        );
    }

    #[test]
    fn a_routine_may_have_no_steps_and_comments_are_skipped() {
        assert_eq!(steps("# 朝\nおはよう:\n"), []);
        assert_eq!(steps("おはよう: []"), []);
        assert_eq!(parse("").unwrap(), []);
        assert_eq!(parse("# nothing yet\n").unwrap(), []);
    }

    #[test]
    fn what_is_wrong_is_said_with_the_routine_and_the_step() {
        let wrong = |yaml: &str| parse(yaml).unwrap_err();
        assert_eq!(
            wrong("- a"),
            "操作の名前と、そのステップの並びを書いてください"
        );
        assert_eq!(
            wrong("帰宅: 3"),
            "「帰宅」: ステップを「- 」で並べてください"
        );
        assert_eq!(
            wrong("帰宅:\n  - door: open"),
            "「帰宅」の1番目: door は lock か unlock です"
        );
        assert_eq!(
            wrong("帰宅:\n  - door: lock\n  - on: a\n    off: b"),
            "「帰宅」の2番目: on・off・door・sleep・wake・http・near のどれか1つを書いてください"
        );
        assert_eq!(
            wrong("帰宅:\n  - turn: on"),
            "「帰宅」の1番目: on・off・door・sleep・wake・http・near のどれか1つを書いてください"
        );
        assert_eq!(
            wrong("帰宅:\n  - on: a\n    wait: 3"),
            "「帰宅」の1番目: wait は使えません"
        );
        assert_eq!(
            wrong("帰宅:\n  - on: ''"),
            "「帰宅」の1番目: on に名前を書いてください"
        );
        assert_eq!(
            wrong("帰宅:\n  - http: ftp://a"),
            "「帰宅」の1番目: http は「POST http://…」の形で書いてください（GET・POST・PUT・PATCH・DELETE）"
        );
        assert_eq!(
            wrong("帰宅:\n  - http: POST /api/labels"),
            "「帰宅」の1番目: http は「POST http://…」の形で書いてください（GET・POST・PUT・PATCH・DELETE）"
        );
        assert_eq!(
            wrong("帰宅:\n  - near: 0"),
            "「帰宅」の1番目: near は0より大きいメートルで書いてください"
        );
        assert_eq!(
            wrong("帰宅:\n  - near: 100\n    wait: 0"),
            "「帰宅」の1番目: wait は1〜180の分で書いてください"
        );
        assert_eq!(wrong("'': []"), "操作の名前が空です");
        assert!(
            wrong("帰宅: [").starts_with("YAMLとして読めません: "),
            "{}",
            wrong("帰宅: [")
        );
        assert_eq!(
            wrong(&format!("# {}\n", "x".repeat(MAX_BYTES))),
            "長すぎます（16384バイトまで）"
        );
    }

    #[test]
    fn a_step_reads_back_as_it_is_written() {
        let yaml = "a:\n  - on: 空調\n  - off: room\n  - door: unlock\n  - sleep: desk\n  - wake: desk\n  - http: POST http://h/p\n  - near: 100\n";
        let written: Vec<String> = steps(yaml).iter().map(ToString::to_string).collect();
        assert_eq!(
            written,
            [
                "on: 空調",
                "off: room",
                "door: unlock",
                "sleep: desk",
                "wake: desk",
                "http: POST http://h/p",
                "near: 100"
            ]
        );
    }

    #[test]
    fn the_distance_is_measured_over_the_globe() {
        // One degree of latitude is 111.19 km on a sphere of the mean radius.
        let degree = distance((35.0, 139.0), (36.0, 139.0));
        assert!((degree - 111_195.0).abs() < 1.0, "{degree}");
        // A degree of longitude shrinks with the cosine of the latitude.
        let east = distance((60.0, 10.0), (60.0, 11.0));
        assert!((east - 55_597.0).abs() < 2.0, "{east}");
        assert!(distance((34.3, 132.5), (34.3, 132.5)).abs() < 1e-6);
    }

    #[test]
    fn near_wants_a_fresh_fix_sure_to_the_radius_and_within_it() {
        let home = (34.0, 132.0);
        // 0.0009° of latitude is about 100.08 m.
        let at = |latitude: f64, accuracy: f64, age_ms: u64| Fix {
            latitude,
            longitude: 132.0,
            accuracy,
            age_ms,
        };
        let inside = near(home, &at(34.0008, 12.0, 1_000), 100.0).unwrap();
        assert!((inside - 88.96).abs() < 0.1, "{inside}");
        assert!(
            matches!(near(home, &at(34.00091, 12.0, 0), 100.0), Err(Away::Far(m)) if (m - 101.2).abs() < 0.1)
        );
        assert_eq!(
            near(home, &at(34.0, 100.5, 0), 100.0),
            Err(Away::Vague(100.5))
        );
        assert!(near(home, &at(34.0, 100.0, FRESH_MS), 100.0).is_ok());
        assert_eq!(
            near(home, &at(34.0, 5.0, FRESH_MS + 1), 100.0),
            Err(Away::Stale)
        );
    }
}
