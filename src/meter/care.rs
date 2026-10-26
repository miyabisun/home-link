//! What keeps the meter heard when its readings stop: powering the adapter on again, power
//! cycling it, and telling the user. Decides only; [`super::receive`] carries it out.

use serde::Serialize;

use crate::watch::{hhmm, span};

/// How long the adapter may stay powered off before it is powered on again; a commissioning
/// over home-server's BLE proxy powers it off for about a second before taking it.
pub const OFF_FOR: i64 = 60;
/// How soon after powering the adapter on it may be powered on again.
pub const POWER_ON_EVERY: i64 = 30;
/// How long discovery may go without the meter before the adapter is power cycled.
pub const DEAF_FOR: i64 = 600;
/// How soon after a power cycle another may follow.
pub const CYCLE_EVERY: i64 = 1800;
/// How long the meter may go unheard before the user is told.
pub const TELL_AFTER: i64 = 900;
/// Below this battery percent the user is told to change it.
pub const LOW_BATTERY: u8 = 20;
/// The battery percent it must come back to before a low battery is told again.
pub const BATTERY_BACK: u8 = 21;

/// The meter's adapter as `BlueZ` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Adapter {
    pub powered: bool,
    pub discovering: bool,
}

/// What was found of Bluetooth at one look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sight {
    /// `BlueZ` did not answer.
    NoBluetooth,
    /// `BlueZ` answered without an adapter: the UB500 is unplugged or gone with a USB reset.
    NoAdapter,
    Adapter(Adapter),
}

/// What is done to the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mend {
    PowerOn,
    /// Off then on, so `BlueZ` resets the controller.
    PowerCycle,
}

/// Why the meter is not heard, as told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trouble {
    NoBluetooth,
    NoAdapter,
    /// The adapter is there but stays off or out of discovery.
    AdapterDown,
    /// Discovery runs, and only the meter is not heard.
    MeterSilent,
}

/// A message to the user about the meter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// Not heard since `since` for [`TELL_AFTER`].
    Lost {
        since: i64,
        trouble: Trouble,
        battery: Option<u8>,
    },
    /// Heard again at `until`, after it was told lost.
    Back {
        since: i64,
        until: i64,
    },
    LowBattery(u8),
}

/// What one look calls for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Turn {
    pub mend: Option<Mend>,
    pub notice: Option<Notice>,
}

/// The meter's keeping, from one look and one reading to the next.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Care {
    /// The latest look.
    #[serde(skip)]
    at: Option<i64>,
    /// The latest reading, else the first look.
    #[serde(skip)]
    heard_at: Option<i64>,
    /// Seconds without a reading since then, leaving out the commissionings' holds.
    #[serde(skip)]
    quiet: i64,
    /// Since when the adapter was seen powered off outside a hold.
    #[serde(skip)]
    off_since: Option<i64>,
    #[serde(skip)]
    battery: Option<u8>,
    #[serde(skip)]
    told_lost: bool,
    #[serde(skip)]
    told_battery: bool,
    /// The latest look's adapter; `None` before the first look or without `BlueZ`.
    adapter: Option<AdapterView>,
    powered_on_at: Option<i64>,
    power_cycled_at: Option<i64>,
}

/// The adapter as `GET /api/meter` shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct AdapterView {
    present: bool,
    powered: bool,
    discovering: bool,
}

impl Care {
    /// Looks at Bluetooth at `now`; `held` while a commissioning has the adapter, `tell` when
    /// messages have somewhere to go.
    pub fn look(&mut self, now: i64, held: bool, sight: Sight, tell: bool) -> Turn {
        let since = *self.heard_at.get_or_insert(now);
        if !held {
            self.quiet += now - self.at.unwrap_or(now);
        }
        self.at = Some(now);
        self.adapter = match sight {
            Sight::NoBluetooth => None,
            Sight::NoAdapter => Some(AdapterView {
                present: false,
                powered: false,
                discovering: false,
            }),
            Sight::Adapter(Adapter {
                powered,
                discovering,
            }) => Some(AdapterView {
                present: true,
                powered,
                discovering,
            }),
        };
        let off = matches!(sight, Sight::Adapter(Adapter { powered: false, .. })) && !held;
        if !off {
            self.off_since = None;
        }
        let mend = if off {
            let off_since = *self.off_since.get_or_insert(now);
            (now - off_since >= OFF_FOR
                && self
                    .powered_on_at
                    .is_none_or(|at| now - at >= POWER_ON_EVERY))
            .then(|| {
                self.powered_on_at = Some(now);
                Mend::PowerOn
            })
        } else {
            (sight
                == Sight::Adapter(Adapter {
                    powered: true,
                    discovering: true,
                })
                && !held
                && self.quiet >= DEAF_FOR
                && self
                    .power_cycled_at
                    .is_none_or(|at| now - at >= CYCLE_EVERY))
            .then(|| {
                self.power_cycled_at = Some(now);
                Mend::PowerCycle
            })
        };
        let notice = (tell && !self.told_lost && self.quiet >= TELL_AFTER).then(|| {
            self.told_lost = true;
            Notice::Lost {
                since,
                trouble: match sight {
                    Sight::NoBluetooth => Trouble::NoBluetooth,
                    Sight::NoAdapter => Trouble::NoAdapter,
                    Sight::Adapter(Adapter {
                        powered: true,
                        discovering: true,
                    }) => Trouble::MeterSilent,
                    Sight::Adapter(_) => Trouble::AdapterDown,
                },
                battery: self.battery,
            }
        });
        Turn { mend, notice }
    }

    /// Takes a reading at `at` with its `battery` percent.
    pub fn heard(&mut self, at: i64, battery: u8, tell: bool) -> Vec<Notice> {
        let since = self.heard_at.replace(at).unwrap_or(at);
        self.at = Some(at);
        self.quiet = 0;
        self.battery = Some(battery);
        let mut notices = Vec::new();
        if std::mem::take(&mut self.told_lost) {
            notices.push(Notice::Back { since, until: at });
        }
        if battery >= BATTERY_BACK {
            self.told_battery = false;
        } else if battery < LOW_BATTERY && tell && !self.told_battery {
            self.told_battery = true;
            notices.push(Notice::LowBattery(battery));
        }
        notices
    }
}

impl Notice {
    /// What kind of message it is, for the log.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Lost { .. } => "meter_lost",
            Self::Back { .. } => "meter_back",
            Self::LowBattery(_) => "meter_battery",
        }
    }

    /// The message as sent, in Japanese.
    #[must_use]
    pub fn text(&self) -> String {
        match *self {
            Self::Lost {
                since,
                trouble,
                battery,
            } => {
                let what = format!("温度計を{}から読めていません。", hhmm(since));
                let todo = match (trouble, battery) {
                    (Trouble::NoBluetooth, _) => String::new(),
                    (Trouble::NoAdapter, _) => "UB500が外れています。挿し直してください。".into(),
                    (Trouble::AdapterDown, _) => "UB500を挿し直してください。".into(),
                    (Trouble::MeterSilent, Some(battery)) => {
                        format!("温度計の電池（最後は{battery}%）と置き場所を確かめてください。")
                    }
                    (Trouble::MeterSilent, None) => {
                        "温度計の電池と置き場所を確かめてください。".into()
                    }
                };
                what + &todo
            }
            Self::Back { since, until } => {
                format!("温度計を{}ぶりに読めました。", span(until - since))
            }
            Self::LowBattery(battery) => {
                format!("温度計の電池が残り{battery}%です。交換してください。")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Sight = Sight::Adapter(Adapter {
        powered: true,
        discovering: true,
    });
    const OFF: Sight = Sight::Adapter(Adapter {
        powered: false,
        discovering: false,
    });

    /// Looks every ten seconds from `from` to `to` inclusive, collecting what each calls for.
    fn looks(care: &mut Care, from: i64, to: i64, held: bool, sight: Sight) -> Vec<(i64, Turn)> {
        (from..=to)
            .step_by(10)
            .map(|at| (at, care.look(at, held, sight, true)))
            .filter(|(_, turn)| *turn != Turn::default())
            .collect()
    }

    fn mends(turns: &[(i64, Turn)]) -> Vec<(i64, Mend)> {
        turns
            .iter()
            .filter_map(|(at, turn)| Some((*at, turn.mend?)))
            .collect()
    }

    fn notices(turns: &[(i64, Turn)]) -> Vec<(i64, Notice)> {
        turns
            .iter()
            .filter_map(|(at, turn)| Some((*at, turn.notice?)))
            .collect()
    }

    #[test]
    fn an_adapter_off_a_minute_is_powered_on_not_before() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        assert_eq!(care.look(0, false, OFF, true), Turn::default(), "just off");
        assert_eq!(care.look(59, false, OFF, true), Turn::default(), "59 s off");
        assert_eq!(
            care.look(60, false, OFF, true).mend,
            Some(Mend::PowerOn),
            "60 s off"
        );
    }

    #[test]
    fn an_adapter_that_stays_off_is_powered_on_every_thirty_seconds_at_most() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        let turns = looks(&mut care, 0, 150, false, OFF);
        assert_eq!(
            mends(&turns),
            [
                (60, Mend::PowerOn),
                (90, Mend::PowerOn),
                (120, Mend::PowerOn),
                (150, Mend::PowerOn)
            ],
            "once a minute off, then every 30 s"
        );
    }

    #[test]
    fn an_adapter_off_during_a_hold_is_left_to_the_commissioning() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        assert_eq!(mends(&looks(&mut care, 0, 300, true, OFF)), [], "held");
        assert_eq!(
            mends(&looks(&mut care, 310, 370, false, OFF)),
            [(370, Mend::PowerOn)],
            "a minute from the hold's end, not from when it went off"
        );
    }

    #[test]
    fn an_adapter_back_on_by_itself_is_left_alone() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        looks(&mut care, 0, 50, false, OFF);
        care.look(55, false, ON, true);
        assert_eq!(
            mends(&looks(&mut care, 60, 110, false, OFF)),
            [],
            "the minute counts again from off"
        );
    }

    #[tokio::test]
    async fn without_a_meter_nothing_is_done_to_bluetooth() {
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            super::super::receive(&super::super::Meter::new(None), || 0, true, |_| async {}),
        )
        .await
        .expect("returns at once without reaching BlueZ");
    }

    #[test]
    fn discovery_deaf_ten_minutes_power_cycles_not_nine() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        let turns = looks(&mut care, 0, 600, false, ON);
        assert_eq!(
            mends(&turns),
            [(600, Mend::PowerCycle)],
            "at ten minutes, not at nine"
        );
    }

    #[test]
    fn a_power_cycle_comes_every_thirty_minutes_at_most() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        let turns = looks(&mut care, 0, 4200, false, ON);
        assert_eq!(
            mends(&turns),
            [
                (600, Mend::PowerCycle),
                (2400, Mend::PowerCycle),
                (4200, Mend::PowerCycle)
            ],
            "30 minutes apart"
        );
    }

    #[test]
    fn no_power_cycle_during_a_hold_nor_from_its_time() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        assert_eq!(
            mends(&looks(&mut care, 0, 1200, true, ON)),
            [],
            "held: a rejoin try or commissioning"
        );
        assert_eq!(
            mends(&looks(&mut care, 1210, 1800, false, ON)),
            [(1800, Mend::PowerCycle)],
            "ten minutes unheard outside the hold"
        );
    }

    #[test]
    fn a_meter_heard_again_counts_anew() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        looks(&mut care, 0, 590, false, ON);
        care.heard(595, 80, true);
        assert_eq!(
            mends(&looks(&mut care, 600, 1190, false, ON)),
            [],
            "heard at 595"
        );
    }

    #[test]
    fn unheard_fifteen_minutes_is_told_once_and_once_more_when_back() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        let turns = looks(&mut care, 0, 1800, false, ON);
        assert_eq!(
            notices(&turns),
            [(
                900,
                Notice::Lost {
                    since: 0,
                    trouble: Trouble::MeterSilent,
                    battery: Some(80)
                }
            )],
            "at 15 minutes, not again at 16 nor 30"
        );
        assert_eq!(
            care.heard(1805, 79, true),
            [Notice::Back {
                since: 0,
                until: 1805
            }],
            "back"
        );
        assert_eq!(care.heard(1815, 79, true), [], "told back once");
    }

    #[test]
    fn heard_again_at_fourteen_minutes_tells_nothing() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        assert_eq!(
            notices(&looks(&mut care, 0, 840, false, ON)),
            [],
            "14 minutes"
        );
        assert_eq!(care.heard(845, 80, true), [], "no lost, no back");
        assert_eq!(
            notices(&looks(&mut care, 850, 1700, false, ON)),
            [],
            "counted anew"
        );
    }

    #[test]
    fn a_hold_is_not_counted_as_unheard() {
        let mut care = Care::default();
        care.heard(0, 80, true);
        looks(&mut care, 0, 600, false, ON);
        looks(&mut care, 610, 1200, true, ON);
        let turns = looks(&mut care, 1210, 1600, false, ON);
        assert_eq!(
            notices(&turns)
                .iter()
                .map(|(at, _)| *at)
                .collect::<Vec<_>>(),
            [1500],
            "600 s before the hold and 300 s after it"
        );
    }

    #[test]
    fn without_a_destination_nothing_is_told() {
        let mut care = Care::default();
        care.heard(0, 10, false);
        let told: Vec<_> = (0..=1800)
            .step_by(10)
            .filter_map(|at| care.look(at, false, ON, false).notice)
            .collect();
        assert_eq!(told, [], "lost");
        assert_eq!(care.heard(1805, 10, false), [], "neither back nor battery");
    }

    #[test]
    fn the_trouble_told_is_what_bluetooth_shows_at_fifteen_minutes() {
        for (sight, trouble) in [
            (Sight::NoBluetooth, Trouble::NoBluetooth),
            (Sight::NoAdapter, Trouble::NoAdapter),
            (OFF, Trouble::AdapterDown),
            (
                Sight::Adapter(Adapter {
                    powered: true,
                    discovering: false,
                }),
                Trouble::AdapterDown,
            ),
            (ON, Trouble::MeterSilent),
        ] {
            let mut care = Care::default();
            care.heard(0, 55, true);
            assert_eq!(
                notices(&looks(&mut care, 0, 900, false, sight)),
                [(
                    900,
                    Notice::Lost {
                        since: 0,
                        trouble,
                        battery: Some(55)
                    }
                )],
                "{sight:?}"
            );
        }
    }

    #[test]
    fn a_meter_never_heard_is_lost_from_the_first_look() {
        let mut care = Care::default();
        assert_eq!(
            notices(&looks(&mut care, 100, 1000, false, ON)),
            [(
                1000,
                Notice::Lost {
                    since: 100,
                    trouble: Trouble::MeterSilent,
                    battery: None
                }
            )],
            "15 minutes from the first look"
        );
    }

    #[test]
    fn a_low_battery_is_told_once_until_it_is_back_above_twenty() {
        let mut care = Care::default();
        assert_eq!(care.heard(0, 20, true), [], "20 % is not below");
        assert_eq!(
            care.heard(10, 19, true),
            [Notice::LowBattery(19)],
            "below 20 %"
        );
        assert_eq!(care.heard(20, 18, true), [], "told once");
        assert_eq!(care.heard(30, 20, true), [], "20 % is not back");
        assert_eq!(care.heard(40, 19, true), [], "still not back");
        assert_eq!(care.heard(50, 21, true), [], "back at 21 %");
        assert_eq!(
            care.heard(60, 19, true),
            [Notice::LowBattery(19)],
            "low again"
        );
    }

    #[test]
    fn the_api_shows_the_adapter_and_what_was_done() {
        let mut care = Care::default();
        assert_eq!(
            serde_json::to_value(&care).expect("serializes"),
            serde_json::json!({"adapter": null, "powered_on_at": null, "power_cycled_at": null}),
            "before the first look"
        );
        care.heard(0, 80, true);
        looks(&mut care, 0, 60, false, OFF);
        assert_eq!(
            serde_json::to_value(&care).expect("serializes"),
            serde_json::json!({
                "adapter": {"present": true, "powered": false, "discovering": false},
                "powered_on_at": 60,
                "power_cycled_at": null
            }),
            "off and powered on"
        );
        care.look(70, false, Sight::NoAdapter, true);
        assert_eq!(
            serde_json::to_value(&care).expect("serializes")["adapter"],
            serde_json::json!({"present": false, "powered": false, "discovering": false}),
            "unplugged"
        );
        care.look(80, false, Sight::NoBluetooth, true);
        assert_eq!(
            serde_json::to_value(&care).expect("serializes")["adapter"],
            serde_json::Value::Null,
            "BlueZ not answering"
        );
    }

    #[test]
    fn the_lost_message_tells_apart_an_unplugged_adapter_one_down_and_the_meter_alone() {
        let lost = |trouble, battery| {
            Notice::Lost {
                since: 0,
                trouble,
                battery,
            }
            .text()
        };
        let unplugged = lost(Trouble::NoAdapter, Some(55));
        let down = lost(Trouble::AdapterDown, Some(55));
        let silent = lost(Trouble::MeterSilent, Some(55));
        assert!(unplugged.contains("挿し直して"), "{unplugged}");
        assert!(down.contains("挿し直して"), "{down}");
        assert_ne!(unplugged, down, "only the unplugged one says it is out");
        assert!(
            silent.contains("温度計の電池") && silent.contains("55%"),
            "{silent}"
        );
        assert!(!silent.contains("UB500"), "{silent}");
        assert!(
            !lost(Trouble::MeterSilent, None).contains('%'),
            "no battery heard yet"
        );
        let unknown = lost(Trouble::NoBluetooth, None);
        assert!(
            !unknown.contains("UB500") && !unknown.contains("電池"),
            "nothing for the user to do: {unknown}"
        );
        for text in [&unplugged, &down, &silent, &unknown] {
            assert!(text.contains("09:00"), "since when, in Tokyo: {text}");
        }
    }
}
