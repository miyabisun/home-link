//! What keeps the meter heard when its readings stop: powering the adapter on again, power
//! cycling it, and what asks for the user meanwhile. Decides only; [`super::receive`] carries
//! it out.

use serde::Serialize;

use crate::watch::Attention;

/// How long the adapter may stay powered off before it is powered on again; a commissioning
/// over home-server's BLE proxy powers it off for about a second before taking it.
pub const OFF_FOR: i64 = 60;
/// How soon after powering the adapter on it may be powered on again.
pub const POWER_ON_EVERY: i64 = 30;
/// How long discovery may go without the meter before the adapter is power cycled.
pub const DEAF_FOR: i64 = 600;
/// How soon after a power cycle another may follow.
pub const CYCLE_EVERY: i64 = 1800;
/// How long the meter may go unheard before it asks for the user.
pub const ASK_AFTER: i64 = 900;
/// Below this battery percent the meter asks for its battery to be changed.
pub const LOW_BATTERY: u8 = 20;

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

/// Why the meter is not heard, as the admin screen shows it.
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
    /// Looks at Bluetooth at `now`, `held` while a commissioning has the adapter; what to do
    /// to the adapter, if anything.
    pub fn look(&mut self, now: i64, held: bool, sight: Sight) -> Option<Mend> {
        self.heard_at.get_or_insert(now);
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
        if off {
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
            self.off_since = None;
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
        }
    }

    /// Takes a reading at `at` with its `battery` percent.
    pub fn heard(&mut self, at: i64, battery: u8) {
        self.heard_at = Some(at);
        self.at = Some(at);
        self.quiet = 0;
        self.battery = Some(battery);
    }

    /// What asks for the user as of the latest look or reading: the meter unheard for
    /// [`ASK_AFTER`], with what Bluetooth shows, else its battery below [`LOW_BATTERY`].
    #[must_use]
    pub fn attention(&self) -> Vec<Attention> {
        if self.quiet >= ASK_AFTER {
            let trouble = match self.adapter {
                None => Trouble::NoBluetooth,
                Some(AdapterView { present: false, .. }) => Trouble::NoAdapter,
                Some(AdapterView {
                    powered: true,
                    discovering: true,
                    ..
                }) => Trouble::MeterSilent,
                Some(_) => Trouble::AdapterDown,
            };
            return vec![Attention::MeterLost {
                since: self.heard_at.unwrap_or_default(),
                trouble,
                battery: self.battery,
            }];
        }
        self.battery
            .filter(|&battery| battery < LOW_BATTERY)
            .map(|battery| Attention::MeterBattery { battery })
            .into_iter()
            .collect()
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

    /// Looks every ten seconds from `from` to `to` inclusive, collecting what each does to the adapter.
    fn looks(care: &mut Care, from: i64, to: i64, held: bool, sight: Sight) -> Vec<(i64, Mend)> {
        (from..=to)
            .step_by(10)
            .filter_map(|at| Some((at, care.look(at, held, sight)?)))
            .collect()
    }

    /// Looks every ten seconds from `from` to `to` inclusive, collecting when the meter first
    /// asks for the user.
    fn first_asked(care: &mut Care, from: i64, to: i64, held: bool, sight: Sight) -> Option<i64> {
        (from..=to).step_by(10).find(|&at| {
            care.look(at, held, sight);
            !care.attention().is_empty()
        })
    }

    #[test]
    fn an_adapter_off_a_minute_is_powered_on_not_before() {
        let mut care = Care::default();
        care.heard(0, 80);
        assert_eq!(care.look(0, false, OFF), None, "just off");
        assert_eq!(care.look(59, false, OFF), None, "59 s off");
        assert_eq!(care.look(60, false, OFF), Some(Mend::PowerOn), "60 s off");
    }

    #[test]
    fn an_adapter_that_stays_off_is_powered_on_every_thirty_seconds_at_most() {
        let mut care = Care::default();
        care.heard(0, 80);
        let turns = looks(&mut care, 0, 150, false, OFF);
        assert_eq!(
            turns,
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
        care.heard(0, 80);
        assert_eq!(looks(&mut care, 0, 300, true, OFF), [], "held");
        assert_eq!(
            looks(&mut care, 310, 370, false, OFF),
            [(370, Mend::PowerOn)],
            "a minute from the hold's end, not from when it went off"
        );
    }

    #[test]
    fn an_adapter_back_on_by_itself_is_left_alone() {
        let mut care = Care::default();
        care.heard(0, 80);
        looks(&mut care, 0, 50, false, OFF);
        care.look(55, false, ON);
        assert_eq!(
            looks(&mut care, 60, 110, false, OFF),
            [],
            "the minute counts again from off"
        );
    }

    #[tokio::test]
    async fn without_a_meter_nothing_is_done_to_bluetooth() {
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            super::super::receive(&super::super::Meter::new(None), || 0),
        )
        .await
        .expect("returns at once without reaching BlueZ");
    }

    #[test]
    fn discovery_deaf_ten_minutes_power_cycles_not_nine() {
        let mut care = Care::default();
        care.heard(0, 80);
        let turns = looks(&mut care, 0, 600, false, ON);
        assert_eq!(
            turns,
            [(600, Mend::PowerCycle)],
            "at ten minutes, not at nine"
        );
    }

    #[test]
    fn a_power_cycle_comes_every_thirty_minutes_at_most() {
        let mut care = Care::default();
        care.heard(0, 80);
        let turns = looks(&mut care, 0, 4200, false, ON);
        assert_eq!(
            turns,
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
        care.heard(0, 80);
        assert_eq!(
            looks(&mut care, 0, 1200, true, ON),
            [],
            "held: a rejoin try or commissioning"
        );
        assert_eq!(
            looks(&mut care, 1210, 1800, false, ON),
            [(1800, Mend::PowerCycle)],
            "ten minutes unheard outside the hold"
        );
    }

    #[test]
    fn a_meter_heard_again_counts_anew() {
        let mut care = Care::default();
        care.heard(0, 80);
        looks(&mut care, 0, 590, false, ON);
        care.heard(595, 80);
        assert_eq!(looks(&mut care, 600, 1190, false, ON), [], "heard at 595");
    }

    #[test]
    fn unheard_fifteen_minutes_asks_for_the_user_until_heard_again() {
        let mut care = Care::default();
        care.heard(0, 80);
        assert_eq!(
            first_asked(&mut care, 0, 1800, false, ON),
            Some(900),
            "at 15 minutes"
        );
        looks(&mut care, 910, 1800, false, ON);
        assert_eq!(
            care.attention(),
            [Attention::MeterLost {
                since: 0,
                trouble: Trouble::MeterSilent,
                battery: Some(80)
            }],
            "still asking at 30 minutes"
        );
        care.heard(1805, 79);
        assert_eq!(care.attention(), [], "heard again");
    }

    #[test]
    fn heard_again_at_fourteen_minutes_asks_nothing() {
        let mut care = Care::default();
        care.heard(0, 80);
        assert_eq!(
            first_asked(&mut care, 0, 840, false, ON),
            None,
            "14 minutes"
        );
        care.heard(845, 80);
        assert_eq!(
            first_asked(&mut care, 850, 1700, false, ON),
            None,
            "counted anew"
        );
    }

    #[test]
    fn a_hold_is_not_counted_as_unheard() {
        let mut care = Care::default();
        care.heard(0, 80);
        looks(&mut care, 0, 600, false, ON);
        looks(&mut care, 610, 1200, true, ON);
        assert_eq!(
            first_asked(&mut care, 1210, 1600, false, ON),
            Some(1500),
            "600 s before the hold and 300 s after it"
        );
    }

    #[test]
    fn the_trouble_shown_is_what_bluetooth_shows_now() {
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
            care.heard(0, 55);
            looks(&mut care, 0, 900, false, ON);
            care.look(910, false, sight);
            assert_eq!(
                care.attention(),
                [Attention::MeterLost {
                    since: 0,
                    trouble,
                    battery: Some(55)
                }],
                "{sight:?}"
            );
        }
    }

    #[test]
    fn a_meter_never_heard_is_lost_from_the_first_look() {
        let mut care = Care::default();
        assert_eq!(care.attention(), [], "not looked at yet");
        assert_eq!(
            first_asked(&mut care, 100, 1000, false, ON),
            Some(1000),
            "15 minutes from the first look"
        );
        assert_eq!(
            care.attention(),
            [Attention::MeterLost {
                since: 100,
                trouble: Trouble::MeterSilent,
                battery: None
            }]
        );
    }

    #[test]
    fn a_battery_below_twenty_asks_for_the_user_while_the_meter_is_heard() {
        let mut care = Care::default();
        care.heard(0, 20);
        assert_eq!(care.attention(), [], "20 % is not below");
        care.heard(10, 19);
        assert_eq!(care.attention(), [Attention::MeterBattery { battery: 19 }]);
        care.heard(20, 21);
        assert_eq!(care.attention(), [], "changed");
        care.heard(30, 19);
        looks(&mut care, 30, 930, false, ON);
        assert!(
            matches!(
                care.attention().as_slice(),
                [Attention::MeterLost {
                    battery: Some(19),
                    ..
                }]
            ),
            "lost: the battery goes with it, not twice: {:?}",
            care.attention()
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
        care.heard(0, 80);
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
        care.look(70, false, Sight::NoAdapter);
        assert_eq!(
            serde_json::to_value(&care).expect("serializes")["adapter"],
            serde_json::json!({"present": false, "powered": false, "discovering": false}),
            "unplugged"
        );
        care.look(80, false, Sight::NoBluetooth);
        assert_eq!(
            serde_json::to_value(&care).expect("serializes")["adapter"],
            serde_json::Value::Null,
            "BlueZ not answering"
        );
    }
}
