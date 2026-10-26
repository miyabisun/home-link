//! Bringing back a ledger light that left the fabric, by itself: what to look for in
//! Bluetooth advertisements, when to commission it again with its stored code, and why it
//! is not back yet. The light drops its keys when it leaves, so it cannot be kept; it can
//! only be registered again once it waits for commissioning. [`crate::rejoiner`] carries
//! these decisions out.

use serde::Serialize;

use crate::onboarding::{self, Discriminator};

/// The service UUID a Matter device waiting for commissioning advertises under.
pub const MATTER_SERVICE: &str = "0000fff6-0000-1000-8000-00805f9b34fb";
/// The longest home-link's BLE proxy stays open, holding the meter's adapter, in seconds: under the five
/// minutes [`crate::meter::FRESH_FOR`] a reading stands, so the room's reading never lapses.
pub const ATTEMPT_TIMEOUT: i64 = 180;
/// How long to wait for matterjs-server to answer a commissioning, in seconds: after a light
/// refuses the code, it answers only once its own discovery of about three minutes ends.
pub const COMMISSION_WAIT: i64 = 240;
/// How long the meter has its adapter back between two commissionings, in seconds.
pub const METER_GAP: i64 = 60;
/// How long an advertisement stands for a light waiting to be commissioned, in seconds.
pub const ADVERT_FRESH: i64 = 60;
/// How long a reason stands before it is told, in seconds, as the watch's other messages.
pub const TELL_AFTER: i64 = crate::watch::NOTIFY_AFTER;
/// The longest wait between two tries at a light that is advertising, in seconds.
pub const MAX_RETRY: i64 = 180;
/// The wait before trying a code the light refused again, in seconds: the refusal may
/// still be a Bluetooth failure matterjs-server words as one.
pub const WRONG_CODE_RETRY: i64 = 600;

/// What a commissionable Matter device says in its `0xFFF6` service data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advert {
    pub discriminator: u16,
    /// Both 0 in an extended announcement.
    pub vendor_id: u16,
    pub product_id: u16,
}

/// Reads `0xFFF6` service data: the opcode 0, then the 12-bit discriminator (the top four
/// bits are the advertisement's version), the vendor and the product, little-endian.
#[must_use]
pub fn advert(data: &[u8]) -> Option<Advert> {
    let &[0, d0, d1, v0, v1, p0, p1, ..] = data else {
        return None;
    };
    Some(Advert {
        discriminator: u16::from_le_bytes([d0, d1]) & 0x0FFF,
        vendor_id: u16::from_le_bytes([v0, v1]),
        product_id: u16::from_le_bytes([p0, p1]),
    })
}

/// matterjs-server's BLE proxy endpoint beside its WebSocket API at `url` (`…/ws`).
#[must_use]
pub fn proxy_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    let base = url.strip_suffix("/ws").unwrap_or(url);
    format!("{base}/ble")
}

/// The device a stored code names, as its advertisements show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sought {
    pub discriminator: Discriminator,
    /// The vendor and product of a QR payload; a manual code carries none.
    pub ids: Option<(u16, u16)>,
}

impl Sought {
    /// What to look for from a ledger code, a QR payload or a manual code.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Some(Self {
            discriminator: onboarding::discriminator(code)?,
            ids: onboarding::ids(code),
        })
    }

    /// Whether `advert` may come from this device: the same discriminator, as far as the code
    /// carries it, and the same vendor and product when both are known.
    #[must_use]
    pub fn matches(&self, advert: &Advert) -> bool {
        let discriminator = match self.discriminator {
            Discriminator::Long(long) => advert.discriminator == long,
            Discriminator::Short(short) => advert.discriminator >> 8 == u16::from(short),
        };
        let announced = advert.vendor_id != 0 || advert.product_id != 0;
        discriminator
            && self
                .ids
                .is_none_or(|ids| !announced || ids == (advert.vendor_id, advert.product_id))
    }
}

/// Why a light that left is not back yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// No advertisement of the light waiting for commissioning is heard.
    NoAdvert,
    /// It advertises, but commissioning it failed over Bluetooth or the network.
    ConnectFailed,
    /// It refused the stored code.
    WrongCode,
    /// The ledger holds no code for it.
    NoCode,
    /// home-link has no Bluetooth to look with, or matterjs-server no BLE proxy to take it.
    NoBluetooth,
}

impl Reason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoAdvert => "no_advert",
            Self::ConnectFailed => "connect_failed",
            Self::WrongCode => "wrong_code",
            Self::NoCode => "no_code",
            Self::NoBluetooth => "no_bluetooth",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::NoAdvert,
            Self::ConnectFailed,
            Self::WrongCode,
            Self::NoCode,
            Self::NoBluetooth,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == text)
    }

    /// Why a commissioning that failed under the API's error `code` left the light out.
    #[must_use]
    pub fn of_failure(code: &str) -> Self {
        match code {
            "wrong_code" => Self::WrongCode,
            "no_setup_code" => Self::NoCode,
            "bluetooth_unavailable" => Self::NoBluetooth,
            _ => Self::ConnectFailed,
        }
    }

    /// The reason for the light `name` in a message: what holds it, and what the user can do.
    #[must_use]
    pub fn sentence(self, name: &str) -> String {
        match self {
            Self::NoAdvert => {
                format!(
                    "{name}がまだ登録を待っていません。壁のスイッチで電源を入れ直してください。"
                )
            }
            Self::ConnectFailed => format!("{name}につながらず、登録し直せていません。"),
            Self::WrongCode => {
                format!("{name}が保存したコードを受け付けません。アプリで登録し直してください。")
            }
            Self::NoCode => {
                format!("{name}のコードが保存されていません。アプリで登録し直してください。")
            }
            Self::NoBluetooth => {
                format!("homeserverのBluetoothが使えず、{name}を登録し直せていません。")
            }
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::NoAdvert => 1,
            Self::ConnectFailed => 2,
            Self::WrongCode => 4,
            Self::NoCode => 8,
            Self::NoBluetooth => 16,
        }
    }
}

/// How long to wait after the `attempts`-th failed try before the next, in seconds: doubling
/// from 30 s up to [`MAX_RETRY`], so a light that advertises for a few minutes after it is
/// switched on is tried several times, and [`WRONG_CODE_RETRY`] after a refused code.
#[must_use]
pub fn retry_after(attempts: u32, reason: Option<Reason>) -> i64 {
    if reason == Some(Reason::WrongCode) {
        return WRONG_CODE_RETRY;
    }
    30_i64
        .checked_shl(attempts.saturating_sub(1))
        .unwrap_or(MAX_RETRY)
        .min(MAX_RETRY)
}

/// The pursuit of one light that left: kept with its stretch out of the fabric.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Pursuit {
    /// Commissionings tried, and when the last one ended.
    pub attempts: u32,
    pub attempted_at: Option<i64>,
    /// Why it is not back yet, and since when; `None` while it is about to be tried.
    pub reason: Option<Reason>,
    pub reason_since: Option<i64>,
    /// When its advertisement was last heard.
    pub advert_at: Option<i64>,
    /// Commissioned again by home-link; the next check finds it back.
    pub rejoined: bool,
    /// The reasons already told, as bits.
    #[serde(skip)]
    pub told: u8,
}

impl Pursuit {
    fn set(&mut self, at: i64, reason: Option<Reason>) {
        if self.reason != reason {
            self.reason = reason;
            self.reason_since = reason.map(|_| at);
        }
    }

    /// Takes in a look at `at` from the stretch that began `since`: whether there is a code,
    /// whether Bluetooth worked, and whether the light's advertisement was heard.
    pub fn look(&mut self, at: i64, since: i64, code: bool, bluetooth: bool, heard: bool) {
        if self.rejoined {
            return;
        }
        if heard {
            self.advert_at = Some(at);
        }
        let reason = if !code {
            Some(Reason::NoCode)
        } else if !bluetooth {
            Some(Reason::NoBluetooth)
        } else if self.advertising(at) {
            // A failure stands while the light keeps advertising; the next try may clear it.
            self.reason
                .filter(|reason| matches!(reason, Reason::ConnectFailed | Reason::WrongCode))
        } else if self.reason == Some(Reason::WrongCode) {
            // Switching the light off and on does not help a wrong code; it stands until tried again.
            self.reason
        } else if at
            - [self.advert_at, Some(since), self.attempted_at]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(since)
            >= ADVERT_FRESH
        {
            // Unheard for a minute since it was heard, it left, or it was last tried (nothing
            // listens while trying).
            Some(Reason::NoAdvert)
        } else {
            self.reason
        };
        self.set(at, reason);
    }

    fn advertising(&self, at: i64) -> bool {
        self.advert_at
            .is_some_and(|heard| at - heard < ADVERT_FRESH)
    }

    /// Whether to commission it now: it advertises, and the wait after the last try is over.
    #[must_use]
    pub fn due(&self, at: i64) -> bool {
        !self.rejoined
            && self.advertising(at)
            && !matches!(self.reason, Some(Reason::NoCode | Reason::NoBluetooth))
            && self
                .attempted_at
                .is_none_or(|last| at - last >= retry_after(self.attempts, self.reason))
    }

    /// Takes in a commissioning that ended at `at`: back, or not and why.
    pub fn attempted(&mut self, at: i64, result: Result<(), Reason>) {
        self.attempts += 1;
        self.attempted_at = Some(at);
        match result {
            Ok(()) => {
                self.rejoined = true;
                self.set(at, None);
            }
            Err(reason) => self.set(at, Some(reason)),
        }
    }

    /// The reason to tell at `at`: one that has stood for [`TELL_AFTER`] and was not told in this
    /// stretch; it is marked told.
    pub fn tell(&mut self, at: i64) -> Option<Reason> {
        let reason = self.reason?;
        let stood = at - self.reason_since? >= TELL_AFTER;
        (stood && self.told & reason.bit() == 0).then(|| {
            self.told |= reason.bit();
            reason
        })
    }

    /// Marks the reason now as told, when it went out with the message that the light left.
    pub fn told_now(&mut self) {
        if let Some(reason) = self.reason {
            self.told |= reason.bit();
        }
    }
}

/// Which of the `due` lights, by ledger row and last try, to commission at `at`: none until
/// the meter has had its adapter for [`METER_GAP`] since the last commissioning ended, then
/// the one tried longest ago.
#[must_use]
pub fn pick(at: i64, due: &[(i64, Option<i64>)], last_ended: Option<i64>) -> Option<i64> {
    if last_ended.is_some_and(|ended| at - ended < METER_GAP) {
        return None;
    }
    due.iter()
        .min_by_key(|(device, attempted)| (*attempted, *device))
        .map(|(device, _)| *device)
}

/// How often a light left the fabric: the times it left in the history, this one among them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Leaves {
    /// Times it left in the history (two weeks), this one included.
    pub count: u32,
    /// When it left the time before, if it did.
    pub previous_at: Option<i64>,
}

impl Leaves {
    /// From when the light left each time in the history and when it left this time.
    #[must_use]
    pub fn of(times: &[i64], now: i64) -> Self {
        let before: Vec<i64> = times.iter().copied().filter(|&at| at < now).collect();
        Self {
            count: u32::try_from(before.len())
                .unwrap_or(u32::MAX)
                .saturating_add(1),
            previous_at: before.into_iter().max(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_792_940_400;

    fn heard(at: i64) -> Pursuit {
        let mut pursuit = Pursuit::default();
        pursuit.look(at, at, true, true, true);
        pursuit
    }

    #[test]
    fn reads_the_service_data_matter_js_advertises() {
        // BtpCodec.encodeBleAdvertisementData(3841, 0xFFF1, 0x8000) after its 0xFFF6 UUID.
        let data = [0x00, 0x01, 0x0F, 0xF1, 0xFF, 0x00, 0x80, 0x00];
        assert_eq!(
            advert(&data),
            Some(Advert {
                discriminator: 3841,
                vendor_id: 0xFFF1,
                product_id: 0x8000
            }),
            "discriminator, vendor and product"
        );
        let versioned = [0x00, 0x01, 0x1F, 0xF1, 0xFF, 0x00, 0x80, 0x00];
        assert_eq!(
            advert(&versioned).map(|a| a.discriminator),
            Some(3841),
            "the version bits are not the discriminator's"
        );
        assert_eq!(advert(&data[..6]), None, "too short");
        assert_eq!(
            advert(&[0x01, 0x01, 0x0F, 0xF1, 0xFF, 0x00, 0x80]),
            None,
            "not a commissionable advertisement"
        );
    }

    #[test]
    fn a_qr_payload_is_matched_by_its_whole_discriminator_and_ids() {
        // The specification's example: 0xFFF1/0x8000, discriminator 3840.
        let sought = Sought::from_code("MT:Y.K9042C00KA0648G00").expect("a code");
        let advert = |discriminator, vendor_id, product_id| Advert {
            discriminator,
            vendor_id,
            product_id,
        };
        assert!(sought.matches(&advert(3840, 0xFFF1, 0x8000)), "itself");
        assert!(
            !sought.matches(&advert(3841, 0xFFF1, 0x8000)),
            "another light of the same product"
        );
        assert!(
            !sought.matches(&advert(3840, 0x115F, 0x8000)),
            "another vendor's device with the same discriminator"
        );
        assert!(
            sought.matches(&advert(3840, 0, 0)),
            "an extended announcement hides the ids"
        );
    }

    #[test]
    fn a_manual_code_is_matched_by_the_top_four_bits() {
        // The specification's manual code for the same device: short discriminator 15.
        let sought = Sought::from_code("34970112332").expect("a code");
        assert_eq!(sought.ids, None, "no ids in a manual code");
        let with = |discriminator| Advert {
            discriminator,
            vendor_id: 0x115F,
            product_id: 1,
        };
        assert!(sought.matches(&with(0xF00)), "3840");
        assert!(sought.matches(&with(0xFFF)), "4095 shares the top bits");
        assert!(!sought.matches(&with(0xE00)), "another top nibble");
        assert_eq!(Sought::from_code("not a code"), None, "no code");
    }

    #[test]
    fn tries_double_up_to_three_minutes_and_a_refused_code_waits_ten() {
        let waits: Vec<i64> = (1..=6).map(|n| retry_after(n, None)).collect();
        assert_eq!(waits, [30, 60, 120, 180, 180, 180], "doubling, then capped");
        assert_eq!(
            retry_after(40, Some(Reason::ConnectFailed)),
            180,
            "no overflow"
        );
        assert_eq!(retry_after(1, Some(Reason::WrongCode)), 600, "refused code");
    }

    #[test]
    fn no_advertisement_is_the_reason_after_a_minute_unheard() {
        let mut pursuit = Pursuit::default();
        pursuit.look(T0, T0, true, true, false);
        assert_eq!(pursuit.reason, None, "just left");
        pursuit.look(T0 + 59, T0, true, true, false);
        assert_eq!(pursuit.reason, None, "under a minute");
        pursuit.look(T0 + 60, T0, true, true, false);
        assert_eq!(pursuit.reason, Some(Reason::NoAdvert), "a minute unheard");
        assert_eq!(pursuit.reason_since, Some(T0 + 60), "since then");
        assert!(!pursuit.due(T0 + 60), "nothing to commission");
    }

    #[test]
    fn a_late_advertisement_is_tried_at_once() {
        let mut pursuit = Pursuit::default();
        for minute in 0..30 {
            pursuit.look(T0 + minute * 60, T0, true, true, false);
        }
        assert_eq!(
            pursuit.reason,
            Some(Reason::NoAdvert),
            "half an hour unheard"
        );
        let at = T0 + 30 * 60 + 7;
        pursuit.look(at, T0, true, true, true);
        assert_eq!(pursuit.reason, None, "heard: about to be tried");
        assert!(pursuit.due(at), "tried as soon as heard");
    }

    #[test]
    fn a_failed_try_waits_and_its_reason_stands_while_the_light_advertises() {
        let mut pursuit = heard(T0);
        assert!(pursuit.due(T0), "first try");
        pursuit.attempted(T0 + 40, Err(Reason::ConnectFailed));
        pursuit.look(T0 + 45, T0, true, true, true);
        assert_eq!(
            pursuit.reason,
            Some(Reason::ConnectFailed),
            "still advertising"
        );
        assert!(!pursuit.due(T0 + 69), "under 30 s since the try ended");
        pursuit.look(T0 + 70, T0, true, true, true);
        assert!(pursuit.due(T0 + 70), "30 s on");
        pursuit.attempted(T0 + 100, Err(Reason::WrongCode));
        pursuit.look(T0 + 130, T0, true, true, true);
        pursuit.look(T0 + 699, T0, true, true, true);
        assert!(!pursuit.due(T0 + 699), "a refused code waits ten minutes");
        pursuit.look(T0 + 700, T0, true, true, true);
        assert!(pursuit.due(T0 + 700), "then is tried again");
        pursuit.attempted(T0 + 730, Err(Reason::ConnectFailed));
        pursuit.look(T0 + 790, T0, true, true, false);
        assert_eq!(
            pursuit.reason,
            Some(Reason::NoAdvert),
            "the light stopped advertising: that is what holds it now"
        );
    }

    #[test]
    fn a_refused_code_stands_when_the_light_stops_advertising() {
        let mut pursuit = heard(T0);
        // Three minutes trying, unheard meanwhile: the advertisement is old when the try ends.
        pursuit.attempted(T0 + 180, Err(Reason::WrongCode));
        pursuit.look(T0 + 185, T0, true, true, false);
        assert_eq!(pursuit.reason, Some(Reason::WrongCode), "just tried");
        pursuit.look(T0 + 600, T0, true, true, false);
        assert_eq!(
            pursuit.reason,
            Some(Reason::WrongCode),
            "switching it off and on would not help a wrong code"
        );
    }

    #[test]
    fn a_light_unheard_while_it_was_tried_gets_a_minute_after_the_try() {
        let mut pursuit = heard(T0);
        pursuit.attempted(T0 + 180, Err(Reason::ConnectFailed));
        pursuit.look(T0 + 239, T0, true, true, false);
        assert_eq!(
            pursuit.reason,
            Some(Reason::ConnectFailed),
            "under a minute since the try"
        );
        pursuit.look(T0 + 240, T0, true, true, false);
        assert_eq!(
            pursuit.reason,
            Some(Reason::NoAdvert),
            "a minute unheard after it"
        );
    }

    #[test]
    fn no_code_and_no_bluetooth_come_first_and_keep_it_from_being_tried() {
        let mut pursuit = Pursuit::default();
        pursuit.look(T0, T0, false, true, true);
        assert_eq!(pursuit.reason, Some(Reason::NoCode), "no code");
        assert!(!pursuit.due(T0), "nothing to commission with");
        pursuit.look(T0 + 5, T0, true, false, false);
        assert_eq!(pursuit.reason, Some(Reason::NoBluetooth), "no Bluetooth");
        pursuit.look(T0 + 10, T0, true, true, true);
        assert_eq!(pursuit.reason, None, "Bluetooth back and heard");
        assert!(pursuit.due(T0 + 10), "tried");
    }

    #[test]
    fn a_rejoined_light_is_not_tried_again() {
        let mut pursuit = heard(T0);
        pursuit.attempted(T0 + 50, Ok(()));
        assert!(pursuit.rejoined, "back");
        assert_eq!(pursuit.reason, None, "no reason");
        pursuit.look(T0 + 300, T0, true, true, true);
        assert!(!pursuit.due(T0 + 300), "done");
        assert_eq!(pursuit.attempts, 1, "one try");
    }

    #[test]
    fn each_reason_is_told_once_after_five_minutes() {
        let mut pursuit = Pursuit::default();
        pursuit.look(T0 + 60, T0, true, true, false);
        assert_eq!(pursuit.tell(T0 + 359), None, "under five minutes");
        assert_eq!(
            pursuit.tell(T0 + 360),
            Some(Reason::NoAdvert),
            "five minutes"
        );
        assert_eq!(pursuit.tell(T0 + 420), None, "told once");
        pursuit.look(T0 + 500, T0, true, true, true);
        pursuit.attempted(T0 + 540, Err(Reason::ConnectFailed));
        for at in (600..=840).step_by(5) {
            pursuit.look(T0 + at, T0, true, true, true);
        }
        assert_eq!(pursuit.tell(T0 + 839), None, "a failure under five minutes");
        assert_eq!(
            pursuit.tell(T0 + 840),
            Some(Reason::ConnectFailed),
            "another reason, told too"
        );
        pursuit.look(T0 + 900, T0, true, true, false);
        pursuit.look(T0 + 2000, T0, true, true, false);
        assert_eq!(
            pursuit.tell(T0 + 2000),
            None,
            "no advertisement again: told already"
        );
    }

    #[test]
    fn the_reason_sent_with_the_left_message_is_not_told_again() {
        let mut pursuit = Pursuit::default();
        pursuit.look(T0 + 60, T0, true, true, false);
        pursuit.told_now();
        assert_eq!(
            pursuit.tell(T0 + 3600),
            None,
            "went out with the left message"
        );
    }

    #[test]
    fn the_meter_has_its_adapter_back_between_commissionings() {
        let due = [(19, Some(T0)), (20, None), (21, Some(T0 - 600))];
        assert_eq!(
            pick(T0 + 59, &due, Some(T0)),
            None,
            "under a minute since one ended"
        );
        assert_eq!(
            pick(T0 + 60, &due, Some(T0)),
            Some(20),
            "never tried comes first"
        );
        assert_eq!(pick(T0, &due[..1], None), Some(19), "only one");
        assert_eq!(pick(T0, &[], None), None, "none due");
    }

    #[test]
    fn the_meter_never_goes_stale_however_often_a_light_fails() {
        // A light that advertises all the time and always fails, tried as soon as allowed: each
        // commissioning holds the adapter for at most ATTEMPT_TIMEOUT, and the meter has it back
        // for METER_GAP before the next. Every reading stands FRESH_FOR, and the meter is heard
        // within seconds of discovery starting, so a reading is never older than one hold.
        let mut pursuit = heard(T0);
        let mut held: Vec<(i64, i64)> = Vec::new();
        let mut last_ended = None;
        let mut at = T0;
        while at < T0 + 6 * 3600 {
            pursuit.look(at, T0, true, true, true);
            let due = pursuit.due(at).then_some((19, pursuit.attempted_at));
            if pick(at, due.as_slice(), last_ended).is_some() {
                let ended = at + ATTEMPT_TIMEOUT;
                held.push((at, ended));
                pursuit.attempted(ended, Err(Reason::ConnectFailed));
                last_ended = Some(ended);
                at = ended;
            }
            at += 5;
        }
        assert!(held.len() > 50, "tried all along: {}", held.len());
        for pair in held.windows(2) {
            assert!(pair[1].0 - pair[0].1 >= METER_GAP, "a gap between {pair:?}");
        }
        assert!(
            held.iter()
                .all(|(start, end)| end - start < crate::meter::FRESH_FOR),
            "no hold outlasts a reading"
        );
    }

    #[test]
    fn leaves_count_this_one_and_the_ones_before() {
        assert_eq!(
            Leaves::of(&[], T0),
            Leaves {
                count: 1,
                previous_at: None
            },
            "the first time"
        );
        assert_eq!(
            Leaves::of(&[T0 - 5 * 86_400, T0 - 86_400, T0], T0),
            Leaves {
                count: 3,
                previous_at: Some(T0 - 86_400)
            },
            "this one is not counted twice"
        );
    }

    #[test]
    fn a_failed_commissioning_is_sorted_by_its_error_code() {
        assert_eq!(
            Reason::of_failure("wrong_code"),
            Reason::WrongCode,
            "refused"
        );
        assert_eq!(
            Reason::of_failure("no_setup_code"),
            Reason::NoCode,
            "no code"
        );
        assert_eq!(
            Reason::of_failure("bluetooth_unavailable"),
            Reason::NoBluetooth,
            "matterjs-server without Bluetooth"
        );
        for code in [
            "device_not_found",
            "commission_timeout",
            "commission_failed",
            "thread_failed",
            "matter_server_unreachable",
        ] {
            assert_eq!(Reason::of_failure(code), Reason::ConnectFailed, "{code}");
        }
    }

    #[test]
    fn the_ble_proxy_is_beside_the_websocket_api() {
        assert_eq!(
            proxy_url("ws://127.0.0.1:5580/ws"),
            "ws://127.0.0.1:5580/ble",
            "the same server"
        );
        assert_eq!(
            proxy_url("ws://matter:5580/ws/"),
            "ws://matter:5580/ble",
            "a trailing slash"
        );
        assert_eq!(
            proxy_url("ws://matter:5580"),
            "ws://matter:5580/ble",
            "no path"
        );
    }

    #[test]
    fn reasons_round_trip_their_names() {
        for reason in [
            Reason::NoAdvert,
            Reason::ConnectFailed,
            Reason::WrongCode,
            Reason::NoCode,
            Reason::NoBluetooth,
        ] {
            assert_eq!(Reason::parse(reason.as_str()), Some(reason), "{reason:?}");
        }
        assert_eq!(Reason::parse("other"), None, "unknown");
    }
}
