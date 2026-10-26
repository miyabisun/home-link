//! Bringing back lights that left the fabric, at work: while any is out, home-link listens
//! for its Bluetooth advertisement on the host's adapter, and once heard has matterjs-server
//! commission it again with its stored code over home-link's own BLE proxy. [`crate::rejoin`]
//! and [`crate::watch`] decide; this module only carries the decisions out.

use std::time::Duration;

use futures_util::StreamExt;

use crate::{
    AppState, UNPOISONED,
    ble_proxy::{self, Concern, Radio, Signals},
    rejoin::{self, ATTEMPT_TIMEOUT, COMMISSION_WAIT, MATTER_SERVICE, Reason, Reopen, Sought},
    watcher,
};

/// How long one look listens for advertisements; each look is judged by [`crate::watch::Watch::look`].
const LOOK: Duration = Duration::from_secs(5);
/// How long to wait before looking again for lights that left, while none has.
const IDLE: Duration = Duration::from_secs(10);
/// How long to wait before reaching `BlueZ` again after it failed.
const RADIO_AGAIN: Duration = Duration::from_secs(30);

/// The adapter, listened to for the lights' advertisements.
struct Ear {
    radio: Radio,
    signals: Signals,
    /// Since when none of the lights looked for was heard: when it opened, else last heard.
    quiet_since: i64,
}

impl Ear {
    async fn open(adapter: Option<&str>, at: i64) -> zbus::Result<Self> {
        let radio = Radio::open(adapter).await?;
        radio.scan().await?;
        let signals = radio.signals().await?;
        Ok(Self {
            radio,
            signals,
            quiet_since: at,
        })
    }

    /// The ledger rows of `sought` whose advertisements are heard within [`LOOK`].
    async fn listen(
        &mut self,
        sought: &[(i64, Option<Sought>)],
        first: bool,
    ) -> zbus::Result<Vec<i64>> {
        let mut heard = Vec::new();
        let mut take = |seen: &ble_proxy::Heard| {
            let Some(advert) = seen.data(MATTER_SERVICE).and_then(rejoin::advert) else {
                return;
            };
            for (device, sought) in sought {
                if sought.is_some_and(|sought| sought.matches(&advert)) && !heard.contains(device) {
                    tracing::info!(
                        device_id = device,
                        address = seen.address,
                        rssi = seen.rssi,
                        "rejoin: light heard"
                    );
                    heard.push(*device);
                }
            }
        };
        if first {
            for seen in self.radio.known().await? {
                take(&seen);
            }
        }
        let until = tokio::time::Instant::now() + LOOK;
        loop {
            let signal = match tokio::time::timeout_at(until, self.signals.next()).await {
                Err(_) => return Ok(heard),
                Ok(None) => return Err(zbus::Error::Failure("the bus closed".into())),
                Ok(Some(signal)) => signal?,
            };
            let path = match self.radio.concern(&signal) {
                Concern::Device(path) => path,
                Concern::Adapter => return Err(zbus::Error::Failure("the adapter went".into())),
                Concern::Gone(_) | Concern::Other => continue,
            };
            match self.radio.heard(&path).await {
                Ok(Some(seen)) => take(&seen),
                Ok(None) => {}
                // BlueZ forgot the device between its signal and this question.
                Err(zbus::Error::MethodError(name, _, _))
                    if name.as_str() == "org.freedesktop.DBus.Error.UnknownObject" => {}
                Err(error) => return Err(error),
            }
        }
    }
}

/// The lights that left, by ledger row, with what to look for from their stored codes.
fn sought(state: &AppState, devices: &[i64]) -> Vec<(i64, Option<Sought>)> {
    let db = state.db.lock().expect(UNPOISONED);
    devices
        .iter()
        .map(|&id| {
            let code = crate::find_device(&db, id, true)
                .ok()
                .and_then(|device| device.qr_payload.or(device.manual_code));
            (id, code.as_deref().and_then(Sought::from_code))
        })
        .collect()
}

/// Why the search opens its adapter again, for its log line.
#[derive(Clone, Copy)]
enum Why {
    /// After a try, which held the adapter: it may have gone meanwhile, as the UB500 does when
    /// btusb resets it, and come back under another name.
    Tried,
    Reopen(Reopen),
}

impl Why {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tried => "tried",
            Self::Reopen(reopen) => reopen.as_str(),
        }
    }
}

/// Opens the adapter to listen on, with one line in the log; `again` says why it is opened again
/// and is cleared once it opens.
async fn open_ear(state: &AppState, again: &mut Option<(Why, String)>) -> Option<Ear> {
    let opened = Ear::open(state.bluetooth_adapter.as_deref(), (state.clock)()).await;
    let opened = opened
        .inspect_err(|error| tracing::warn!(%error, "rejoin: Bluetooth unavailable"))
        .ok()?;
    if let Some((why, previous)) = again.take() {
        tracing::info!(
            adapter = opened.radio.name(),
            previous,
            why = why.as_str(),
            "rejoin: adapter opened again"
        );
    } else {
        tracing::info!(
            adapter = opened.radio.name(),
            "rejoin: looking for lights that left"
        );
    }
    Some(opened)
}

/// Looks for the lights that left and commissions each again once heard, forever.
pub(crate) async fn run(state: &AppState) {
    let Some(matter_url) = state.matter_url.clone() else {
        return std::future::pending().await;
    };
    let proxy = state
        .ble_proxy_url
        .clone()
        .unwrap_or_else(|| rejoin::proxy_url(&matter_url));
    let mut ear: Option<Ear> = None;
    // Why the adapter is to be opened again, and the one used before, until it opens.
    let mut again: Option<(Why, String)> = None;
    let mut last_ended: Option<i64> = None;
    loop {
        let pursued = state.watch.lock().expect(UNPOISONED).pursued();
        if pursued.is_empty() {
            if let Some(ear) = ear.take() {
                ear.radio.stop_scan().await;
                tracing::info!("rejoin: no light to look for");
            }
            again = None;
            tokio::time::sleep(IDLE).await;
            continue;
        }
        let sought = sought(state, &pursued);
        if let Some(Ear {
            radio, quiet_since, ..
        }) = &ear
            && let Some(reopen) =
                rejoin::reopen((state.clock)(), radio.present().await, *quiet_since)
        {
            again = Some((Why::Reopen(reopen), radio.name().to_owned()));
        }
        let first = ear.is_none() || again.is_some();
        if first {
            // The one before goes once this one listens, so discovery never pauses.
            ear = open_ear(state, &mut again).await;
        }
        let heard = if let Some(open) = ear.as_mut() {
            match open.listen(&sought, first).await {
                Ok(heard) => {
                    if !heard.is_empty() {
                        open.quiet_since = (state.clock)();
                    }
                    heard
                }
                Err(error) => {
                    // Opened again at once: this look counts neither way.
                    tracing::debug!(%error, "rejoin: adapter lost while listening");
                    again = Some((Why::Reopen(Reopen::Gone), open.radio.name().to_owned()));
                    ear = None;
                    continue;
                }
            }
        } else {
            tokio::time::sleep(RADIO_AGAIN).await;
            Vec::new()
        };
        let at = (state.clock)();
        let due = {
            let mut watch = state.watch.lock().expect(UNPOISONED);
            for (device, sought) in &sought {
                watch.look(
                    *device,
                    at,
                    sought.is_some(),
                    ear.is_some(),
                    heard.contains(device),
                );
            }
            watch.due(at)
        };
        // A commissioning started from the app or the admin screen has the adapter.
        let busy = *state.meter.watch_busy().borrow() > 0;
        let Some(device) = rejoin::pick(at, &due, last_ended).filter(|_| !busy) else {
            continue;
        };
        let Some(Ear { radio, .. }) = ear.take() else {
            continue;
        };
        let result = attempt(state, &radio, &proxy, device).await;
        again = Some((Why::Tried, radio.name().to_owned()));
        let ended = (state.clock)();
        last_ended = Some(ended);
        let change = state
            .watch
            .lock()
            .expect(UNPOISONED)
            .attempted(device, ended, result);
        if let Some(change) = change {
            watcher::keep(state, &change);
        }
    }
}

/// Commissions the light that left as ledger row `device` again with its stored code, over
/// home-link's BLE proxy at `proxy` on `radio`, within [`ATTEMPT_TIMEOUT`]. The meter's
/// discovery stops meanwhile when the meter listens on the same adapter.
async fn attempt(state: &AppState, radio: &Radio, proxy: &str, device: i64) -> Result<(), Reason> {
    tracing::info!(
        device_id = device,
        adapter = radio.name(),
        "rejoin: commissioning again"
    );
    // The meter's discovery stops only while the proxy is open, at most ATTEMPT_TIMEOUT.
    let hold = radio.is_first().await.then(|| state.meter.hold());
    let wait = Duration::from_secs(COMMISSION_WAIT.unsigned_abs());
    let (done, finished) = tokio::sync::oneshot::channel::<()>();
    let serve = async {
        let open = Duration::from_secs(ATTEMPT_TIMEOUT.unsigned_abs());
        let served = ble_proxy::serve(proxy, radio, async {
            let _ = tokio::time::timeout(open, finished).await;
        })
        .await;
        drop(hold);
        served
    };
    let work = async {
        let result = crate::recommission(state, device, wait).await;
        let _ = done.send(());
        result
            .map(|_| ())
            .map_err(|error| Reason::of_failure(error.1))
    };
    // The try ends as soon as the adapter goes; nothing more comes over it.
    let lost = radio.gone();
    tokio::pin!(serve, work, lost);
    let result = tokio::time::timeout(wait + Duration::from_secs(5), async {
        tokio::select! {
            biased;
            () = &mut lost => Err(Reason::AdapterLost),
            result = &mut work => {
                let _ = (&mut serve).await;
                result
            }
            served = &mut serve => if let Err(error) = served {
                tracing::warn!(%error, "rejoin: no BLE proxy");
                Err(Reason::NoBluetooth)
            } else {
                // matterjs-server answers once its own discovery ends, after the proxy closed.
                tokio::select! {
                    biased;
                    () = &mut lost => Err(Reason::AdapterLost),
                    result = &mut work => result,
                }
            },
        }
    })
    .await
    .unwrap_or(Err(Reason::ConnectFailed));
    let result = rejoin::ended(result, radio.present().await);
    match &result {
        Ok(()) => tracing::info!(device_id = device, "rejoin: light commissioned again"),
        Err(reason) => tracing::warn!(
            device_id = device,
            adapter = radio.name(),
            reason = reason.as_str(),
            "rejoin: not commissioned"
        ),
    }
    result
}
