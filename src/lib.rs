pub mod aircon;
pub mod commission;
pub mod discord;
pub mod link;
pub mod matter;
mod mcp;
pub mod meter;
pub mod onboarding;
pub mod outside;
pub mod schedule;
pub mod sesame;
pub mod watch;
mod watcher;

use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;

const MAX_NAME_CHARS: usize = 100;
const MAX_QR_CHARS: usize = 512;

/// Opens (or creates) the `SQLite` database and applies the schema.
///
/// # Errors
/// Fails when the file cannot be opened or the schema cannot be applied.
pub fn open_db(path: &str) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    create_schema(&db, version)?;
    migrate_to_v7(&db, version)?;
    migrate_to_v8(&db, version)?;
    if version < 9 {
        migrate_to_labels_alone(&db)?;
    }
    if version < 10 {
        migrate_without_rooms(&db)?;
    }
    if version < 11 {
        migrate_to_the_home_location(&db)?;
    }
    if version < 12 {
        watcher::migrate(&db)?;
    }
    Ok(db)
}

/// Creates the first schema, then carries it to version 3.
fn create_schema(db: &Connection, version: i64) -> rusqlite::Result<()> {
    if version < 1 {
        // The first schema, which the steps below carry to the current one.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS rooms (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );
        CREATE TABLE IF NOT EXISTS devices (
            id INTEGER PRIMARY KEY,
            room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
            qr_payload TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );",
        )?;
    }
    // The user's last "all on"/"all off" and the light schedule, as JSON by key.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    if version < 1 {
        // Version 1 makes the setup code optional and adds the device's own identifiers.
        db.execute_batch(
            "BEGIN;
            CREATE TABLE devices_v1 (
                id INTEGER PRIMARY KEY,
                room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
                qr_payload TEXT UNIQUE,
                vendor TEXT,
                serial_number TEXT,
                mac TEXT UNIQUE,
                name TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (vendor, serial_number),
                CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
            );
            INSERT INTO devices_v1 (id, room_id, qr_payload, name, created_at)
                SELECT id, room_id, qr_payload, name, created_at FROM devices;
            DROP TABLE devices;
            ALTER TABLE devices_v1 RENAME TO devices;
            PRAGMA user_version = 1;
            COMMIT;",
        )?;
    }
    if version < 2 {
        // Version 2 adds each light's lowest colour temperature for the schedule.
        db.execute_batch(
            "BEGIN;
            ALTER TABLE devices ADD COLUMN min_kelvin INTEGER;
            PRAGMA user_version = 2;
            COMMIT;",
        )?;
    }
    if version < 3 {
        // Version 3 moves each light's floor to a label holding only its warm white.
        db.execute_batch(
            "BEGIN;
            CREATE TABLE labels (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                day_level INTEGER,
                night_level INTEGER,
                cool_kelvin INTEGER,
                warm_kelvin INTEGER
            );
            INSERT INTO labels (name, warm_kelvin)
                SELECT DISTINCT '夜' || min_kelvin || 'K', min_kelvin FROM devices
                WHERE min_kelvin IS NOT NULL ORDER BY min_kelvin;
            ALTER TABLE devices ADD COLUMN label_id INTEGER
                REFERENCES labels(id) ON DELETE SET NULL;
            UPDATE devices SET label_id =
                (SELECT id FROM labels WHERE warm_kelvin = devices.min_kelvin)
                WHERE min_kelvin IS NOT NULL;
            ALTER TABLE devices DROP COLUMN min_kelvin;
            PRAGMA user_version = 3;
            COMMIT;",
        )?;
    }
    Ok(())
}

/// Carries the schema from version 3 to 7.
fn migrate_to_v7(db: &Connection, version: i64) -> rusqlite::Result<()> {
    if version < 4 {
        // Version 4 lets a light that will never answer be ignored.
        db.execute_batch(
            "BEGIN;
            ALTER TABLE devices ADD COLUMN ignored INTEGER NOT NULL DEFAULT 0;
            PRAGMA user_version = 4;
            COMMIT;",
        )?;
    }
    if version < 5 {
        // Version 5 lets a light carry several labels, and a label be boosted.
        // The devices table is rebuilt, as a column with a foreign key cannot be dropped.
        db.execute_batch(
            "PRAGMA foreign_keys = OFF;
            BEGIN;
            CREATE TABLE device_labels (
                device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
                label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
                PRIMARY KEY (device_id, label_id)
            );
            INSERT INTO device_labels SELECT id, label_id FROM devices WHERE label_id IS NOT NULL;
            CREATE TABLE devices_v5 (
                id INTEGER PRIMARY KEY,
                room_id INTEGER NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
                qr_payload TEXT UNIQUE,
                vendor TEXT,
                serial_number TEXT,
                mac TEXT UNIQUE,
                name TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ignored INTEGER NOT NULL DEFAULT 0,
                UNIQUE (vendor, serial_number),
                CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
            );
            INSERT INTO devices_v5
                SELECT id, room_id, qr_payload, vendor, serial_number, mac, name, created_at, ignored
                FROM devices;
            DROP TABLE devices;
            ALTER TABLE devices_v5 RENAME TO devices;
            ALTER TABLE labels ADD COLUMN boost TEXT;
            PRAGMA user_version = 5;
            COMMIT;
            PRAGMA foreign_keys = ON;",
        )?;
    }
    if version < 6 {
        // Version 6 lets a label's lights light only while it is boosted.
        db.execute_batch(
            "BEGIN;
            ALTER TABLE labels ADD COLUMN dedicated INTEGER NOT NULL DEFAULT 0;
            PRAGMA user_version = 6;
            COMMIT;",
        )?;
    }
    if version < 7 {
        // Version 7 keeps the Wi-Fi networks handed to devices, and which one they get.
        db.execute_batch(
            "BEGIN;
            CREATE TABLE IF NOT EXISTS wifi_networks (
                id INTEGER PRIMARY KEY,
                ssid TEXT NOT NULL UNIQUE,
                password TEXT NOT NULL,
                for_devices INTEGER NOT NULL DEFAULT 0
            );
            PRAGMA user_version = 7;
            COMMIT;",
        )?;
    }
    Ok(())
}

/// Carries the schema from version 7 to 8.
fn migrate_to_v8(db: &Connection, version: i64) -> rusqlite::Result<()> {
    if version < 8 {
        // Version 8 lets a device enter the ledger by its code alone, before it has a
        // room, and keeps the vendor and product its QR code names.
        db.execute_batch(
            "PRAGMA foreign_keys = OFF;
            BEGIN;
            CREATE TABLE devices_v8 (
                id INTEGER PRIMARY KEY,
                room_id INTEGER REFERENCES rooms(id) ON DELETE RESTRICT,
                qr_payload TEXT UNIQUE,
                vendor TEXT,
                serial_number TEXT,
                mac TEXT UNIQUE,
                name TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ignored INTEGER NOT NULL DEFAULT 0,
                vendor_id INTEGER,
                product_id INTEGER,
                UNIQUE (vendor, serial_number),
                CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
            );
            INSERT INTO devices_v8
                (id, room_id, qr_payload, vendor, serial_number, mac, name, created_at, ignored)
                SELECT id, room_id, qr_payload, vendor, serial_number, mac, name, created_at, ignored
                FROM devices;
            DROP TABLE devices;
            ALTER TABLE devices_v8 RENAME TO devices;",
        )?;
        let codes = db
            .prepare("SELECT id, qr_payload FROM devices WHERE qr_payload IS NOT NULL")?
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, code) in codes {
            if let Some((vendor_id, product_id)) = onboarding::ids(&code) {
                db.execute(
                    "UPDATE devices SET vendor_id = ?1, product_id = ?2 WHERE id = ?3",
                    params![vendor_id, product_id, id],
                )?;
            }
        }
        db.execute_batch(
            "PRAGMA user_version = 8;
            COMMIT;
            PRAGMA foreign_keys = ON;",
        )?;
    }
    Ok(())
}

/// Version 10 drops rooms, and which room each device was in, for labels alone.
fn migrate_without_rooms(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "PRAGMA foreign_keys = OFF;
        BEGIN;
        CREATE TABLE devices_v10 (
            id INTEGER PRIMARY KEY,
            qr_payload TEXT UNIQUE,
            vendor TEXT,
            serial_number TEXT,
            mac TEXT UNIQUE,
            name TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            ignored INTEGER NOT NULL DEFAULT 0,
            vendor_id INTEGER,
            product_id INTEGER,
            UNIQUE (vendor, serial_number),
            CHECK (qr_payload IS NOT NULL OR serial_number IS NOT NULL)
        );
        INSERT INTO devices_v10
            SELECT id, qr_payload, vendor, serial_number, mac, name, created_at, ignored,
                vendor_id, product_id
            FROM devices;
        DROP TABLE devices;
        ALTER TABLE devices_v10 RENAME TO devices;
        DROP TABLE rooms;
        PRAGMA user_version = 10;
        COMMIT;
        PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}

/// Version 11 keeps the air conditioner's runs for two weeks, and moves the light schedule's
/// coordinates, when they were changed, to the home's location, their one place since.
fn migrate_to_the_home_location(db: &Connection) -> rusqlite::Result<()> {
    let schedule: Option<String> = db
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [SCHEDULE],
            |row| row.get(0),
        )
        .optional()?;
    let place = schedule
        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
        .and_then(|value| Some((value["latitude"].as_f64()?, value["longitude"].as_f64()?)));
    let tokyo = outside::Location::default();
    db.execute_batch("BEGIN;")?;
    if let Some((latitude, longitude)) =
        place.filter(|&place| place != (tokyo.latitude, tokyo.longitude))
    {
        store(
            db,
            LOCATION,
            &outside::Location {
                latitude,
                longitude,
                ..tokyo
            },
        )?;
    }
    db.execute_batch(
        "UPDATE settings SET value = json_remove(value, '$.latitude', '$.longitude')
            WHERE key = 'light_schedule';
        CREATE TABLE IF NOT EXISTS aircon_runs (
            at INTEGER NOT NULL,
            run TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS aircon_runs_at ON aircon_runs (at);
        PRAGMA user_version = 11;
        COMMIT;",
    )
}

/// Version 9 makes labels the one way to switch, hold and adjust lights, keeping how
/// they behave: each label lights at its day's values while on, as boosting did, and
/// boosted labels stay on; a dedicated one lights at its own. Every device outside a
/// dedicated label, which the schedule adjusted, goes under a new auto label that is
/// off only when "all off" was pressed last. Each label's own values, being dedicated
/// and the last "all on" or "all off" go.
fn migrate_to_labels_alone(db: &Connection) -> rusqlite::Result<()> {
    let settings = load::<schedule::Settings>(db, SCHEDULE)?;
    let intent = load::<Option<serde_json::Value>>(db, "lights_intent")?
        .map(|intent| intent["action"] != "off");
    db.execute_batch("PRAGMA foreign_keys = OFF; BEGIN;")?;
    db.execute_batch(
        "CREATE TABLE labels_v9 (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            auto INTEGER NOT NULL DEFAULT 0,
            override_level INTEGER,
            override_kelvin INTEGER,
            \"on\" INTEGER
        );
        CREATE TABLE label_schedules (
            label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
            minute INTEGER NOT NULL,
            \"on\" INTEGER NOT NULL,
            PRIMARY KEY (label_id, minute)
        );",
    )?;
    db.execute(
        "INSERT INTO labels_v9 (id, name, override_level, override_kelvin, \"on\")
            SELECT id, name, coalesce(day_level, ?1), coalesce(cool_kelvin, ?2),
                CASE WHEN boost IS NOT NULL THEN 1 END
            FROM labels",
        params![settings.day_level, settings.cool_kelvin],
    )?;
    let adjusted: Vec<i64> = db
        .prepare(
            "SELECT id FROM devices WHERE id NOT IN (SELECT device_id FROM device_labels
                JOIN labels ON labels.id = label_id WHERE dedicated) ORDER BY id",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    db.execute_batch(
        "DROP TABLE labels;
        ALTER TABLE labels_v9 RENAME TO labels;",
    )?;
    if !adjusted.is_empty() {
        let taken = |name: &str| -> rusqlite::Result<bool> {
            db.query_row(
                "SELECT EXISTS (SELECT 1 FROM labels WHERE name = ?1)",
                [name],
                |row| row.get(0),
            )
        };
        let mut name = String::from("自動調光");
        while taken(&name)? {
            name.push('_');
        }
        db.execute(
            "INSERT INTO labels (name, auto, \"on\") VALUES (?1, 1, ?2)",
            params![name, intent],
        )?;
        let label = db.last_insert_rowid();
        for device in adjusted {
            db.execute(
                "INSERT INTO device_labels (device_id, label_id) VALUES (?1, ?2)",
                [device, label],
            )?;
        }
    }
    db.execute_batch(
        "DELETE FROM settings WHERE key = 'lights_intent';
        PRAGMA user_version = 9;
        COMMIT;
        PRAGMA foreign_keys = ON;",
    )
}

struct AppState {
    db: Mutex<Connection>,
    matter_url: Option<String>,
    /// The latest adjustments, newest first; kept in memory only.
    runs: Mutex<VecDeque<serde_json::Value>>,
    /// What the schedule last sent each light, by node and endpoint; kept in memory only.
    sent: Mutex<HashMap<(u64, u16), schedule::Target>>,
    /// How long one commissioning may take, Bluetooth discovery to Wi-Fi join.
    commission_timeout: Duration,
    wifi_move: commission::MoveTimes,
    /// The current time in Unix seconds for "all on" and its intent, and for signing a door command.
    clock: fn() -> i64,
    /// The front door's SESAME, or why it cannot be unlocked or locked.
    sesame: Result<sesame::Config, &'static str>,
    /// The minute, in Unix minutes, whose label schedules last ran.
    last_schedule: Mutex<Option<i64>>,
    /// How long a scheduled switch waits before sending again to the devices that missed it.
    schedule_retry: Duration,
    /// Held by each label's scheduled switches in turn, so they run in their minutes' order.
    schedule_locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// The room's `SwitchBot` CO2 meter, if one is configured.
    meter: meter::Meter,
    /// Held through one run so a switch and the loop do not send at once.
    aircon_busy: tokio::sync::Mutex<()>,
    /// GSI's address search and Open-Meteo, unless the outside is not reached.
    outside: Option<Outside>,
    /// The latest temperature outside; kept in memory only.
    outdoor: Mutex<Option<Outdoor>>,
    /// The tasks run forever under [`Home::supervise`], by name.
    tasks: Mutex<std::collections::BTreeMap<&'static str, Task>>,
    /// The watch over each light's connection; its history is in the database.
    watch: Mutex<watch::Watch>,
    /// Where the watch's messages go, if anywhere.
    discord: Option<discord::Discord>,
    /// The watch's messages not delivered yet, to send again.
    unsent: Mutex<Vec<watch::Notice>>,
}

/// How a task run forever under [`Home::supervise`] is doing.
struct Task {
    running: bool,
    /// How often it was started again after a panic.
    restarts: u32,
    /// When it last panicked, in Unix seconds.
    last_panic: Option<i64>,
}

/// How long the air conditioner's toggle waits for the send: under the 20 s the app waits.
const TOGGLE_WAIT: Duration = Duration::from_secs(12);
/// How long a task that panicked waits before it is started again.
const RESTART_AFTER: Duration = Duration::from_secs(1);

/// Where the home's place and the temperature outside are asked for.
#[derive(Clone, Debug)]
pub struct Outside {
    pub gsi_url: String,
    pub open_meteo_url: String,
}

/// A temperature outside, the Unix second it is for, and when it was fetched.
#[derive(Clone, Copy, Debug)]
struct Outdoor {
    celsius: f64,
    at: i64,
    fetched_at: i64,
}

const COMMISSION_TIMEOUT: Duration = Duration::from_mins(5);
/// How long a scheduled switch waits before sending again to the devices that missed it.
const SCHEDULE_RETRY: Duration = Duration::from_secs(30);
/// How many times a scheduled switch is sent again to the devices that missed it.
const SCHEDULE_RETRIES: usize = 2;

type Db = Arc<AppState>;

/// Every lock in `AppState` guards plain data, and no code panics while holding one.
const UNPOISONED: &str = "a lock is poisoned only by a panic while holding it";

/// A day of runs every ten minutes.
const RUNS_KEPT: usize = 144;

/// Builds the router; `matter_url` is the matterjs-server WebSocket API used by the status API.
pub fn app(db: Connection, matter_url: Option<String>) -> Router {
    Home::new(db, matter_url).router()
}

/// The API and the light adjustment every ten minutes over one ledger.
#[derive(Clone)]
pub struct Home(Db);

impl Home {
    #[must_use]
    pub fn new(db: Connection, matter_url: Option<String>) -> Self {
        Self(Arc::new(AppState {
            db: Mutex::new(db),
            matter_url,
            runs: Mutex::new(VecDeque::new()),
            sent: Mutex::new(HashMap::new()),
            commission_timeout: COMMISSION_TIMEOUT,
            wifi_move: commission::MOVE_TIMES,
            clock: now,
            sesame: sesame::Config::new(None, None, None, sesame::API_URL),
            last_schedule: Mutex::new(None),
            schedule_retry: SCHEDULE_RETRY,
            schedule_locks: Mutex::new(HashMap::new()),
            tasks: Mutex::new(std::collections::BTreeMap::new()),
            meter: meter::Meter::new(None),
            aircon_busy: tokio::sync::Mutex::new(()),
            outside: None,
            outdoor: Mutex::new(None),
            watch: Mutex::new(watch::Watch::default()),
            discord: None,
            unsent: Mutex::new(Vec::new()),
        }))
    }

    /// Sets where the light watch's messages go.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_discord(mut self, discord: Option<discord::Discord>) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .discord = discord;
        self
    }

    /// Sets the `SwitchBot` CO2 meter, by its Bluetooth address, that [`Self::receive_meter`] listens to.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_meter(mut self, address: String) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .meter = meter::Meter::new(Some(address));
        self
    }

    /// Keeps `reading` as if the meter sent it at `at` Unix seconds.
    pub fn record_meter(&self, reading: meter::Reading, at: i64) {
        self.0.meter.record(reading, at);
    }

    /// Receives the meter's advertisements through `BlueZ` forever; returns at once without a meter.
    pub async fn receive_meter(self) {
        meter::receive(&self.0.meter, self.0.clock).await;
    }

    /// Sets the SESAME that `POST /api/door/unlock` and `/lock` act on, or why there is none.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_sesame(mut self, sesame: Result<sesame::Config, &'static str>) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .sesame = sesame;
        self
    }

    /// Sets where the home's address is searched and the temperature outside asked for.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_outside(mut self, outside: Outside) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .outside = Some(outside);
        self
    }

    /// Replaces the system clock that "all on" and "all off" read.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_clock(mut self, clock: fn() -> i64) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .clock = clock;
        self
    }

    /// Replaces how long a scheduled switch waits before sending again to devices that missed it.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_schedule_retry(mut self, wait: Duration) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .schedule_retry = wait;
        self
    }

    /// Replaces the five-minute limit on one commissioning.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_commission_timeout(mut self, timeout: Duration) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .commission_timeout = timeout;
        self
    }

    /// Replaces how long each step of moving a device to another Wi-Fi may take.
    ///
    /// # Panics
    /// Panics once the router or a clone shares this `Home`.
    #[must_use]
    pub fn with_wifi_move_times(mut self, times: commission::MoveTimes) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("set before the Home is shared")
            .wifi_move = times;
        self
    }

    pub fn router(&self) -> Router {
        router(self.0.clone())
    }

    /// Adjusts the lights that are on for `unix` seconds and records the run.
    pub async fn adjust(&self, unix: i64, trigger: &'static str) {
        adjust(&self.0, unix, trigger, None, link::budget()).await;
    }

    /// Sends the air conditioner what the meter's reading at `unix` calls for, if anything.
    pub async fn adjust_aircon(&self, unix: i64) {
        adjust_aircon(&self.0, unix, "scheduled").await;
    }

    /// Switches the labels whose schedules fall on `unix`'s minute in Asia/Tokyo, once.
    pub async fn run_schedules(&self, unix: i64) {
        run_schedules(&self.0, unix).await;
    }

    /// Runs `run` forever as the task `name`: started again a second after it panics, with the
    /// panic logged, and shown by `/healthz`. A task that ends is not started again, and
    /// `/healthz` answers 503 from then on.
    ///
    /// # Panics
    /// Panics only when a lock is poisoned, by a panic while holding it.
    pub fn supervise<F, R>(&self, name: &'static str, run: F)
    where
        F: Fn(Self) -> R + Send + 'static,
        R: Future<Output = ()> + Send + 'static,
    {
        let home = self.clone();
        let set = move |running: bool, panicked: bool| {
            let mut tasks = home.0.tasks.lock().expect(UNPOISONED);
            let task = tasks.entry(name).or_insert(Task {
                running,
                restarts: 0,
                last_panic: None,
            });
            task.running = running;
            if panicked {
                task.restarts += 1;
                task.last_panic = Some(now());
            }
        };
        set(true, false);
        let home = self.clone();
        tokio::spawn(async move {
            loop {
                match tokio::spawn(run(home.clone())).await {
                    Ok(()) => {
                        tracing::error!(task = name, "task ended; it is not started again");
                        set(false, false);
                        return;
                    }
                    Err(error) => {
                        let panic = error.try_into_panic().ok();
                        let message = panic
                            .as_ref()
                            .and_then(|p| {
                                p.downcast_ref::<&str>()
                                    .map(|s| (*s).to_owned())
                                    .or_else(|| p.downcast_ref::<String>().cloned())
                            })
                            .unwrap_or_default();
                        tracing::error!(
                            task = name,
                            panic = message,
                            "task panicked; starting it again"
                        );
                        set(true, true);
                        tokio::time::sleep(RESTART_AFTER).await;
                    }
                }
            }
        });
    }

    /// Starts the tasks home-link runs forever, each under [`Self::supervise`]: the meter's
    /// receiver when a meter is set, and with matterjs-server the light watch, the adjustments
    /// every ten minutes and the label schedules every minute.
    pub fn start(&self) {
        if self.0.meter.address().is_some() {
            self.supervise("meter", Self::receive_meter);
        }
        if self.0.matter_url.is_some() {
            self.supervise("watch_lights", Self::watch_lights);
            self.supervise("every_ten_minutes", Self::every_ten_minutes);
            self.supervise("every_minute", Self::every_minute);
        }
    }

    /// Runs the label schedules at the start of every minute, forever: each minute's run apart,
    /// so one that takes longer than a minute holds up no later one.
    pub async fn every_minute(self) {
        loop {
            let wait = 60 - now().rem_euclid(60);
            tokio::time::sleep(Duration::from_secs(wait.unsigned_abs())).await;
            let home = self.clone();
            tokio::spawn(async move { home.run_schedules(now()).await });
        }
    }

    /// Checks every light's connection as at `at`, on the minute, and does what that calls for.
    pub async fn check_lights(&self, at: i64) {
        watcher::check(&self.0, at).await;
    }

    /// Takes back the light watch's stretches left open when home-link last stopped.
    pub fn resume_watch(&self) {
        watcher::resume(&self.0);
    }

    /// Checks the lights at the start of every minute, forever, after taking back the
    /// stretches left open when home-link last stopped.
    pub async fn watch_lights(self) {
        self.resume_watch();
        loop {
            let wait = 60 - now().rem_euclid(60);
            tokio::time::sleep(Duration::from_secs(wait.unsigned_abs())).await;
            let at = now();
            self.check_lights(at - at.rem_euclid(60)).await;
        }
    }

    /// Adjusts the lights and the air conditioner every ten minutes on the clock, forever.
    pub async fn every_ten_minutes(self) {
        loop {
            let wait = 600 - now().rem_euclid(600);
            tokio::time::sleep(Duration::from_secs(wait.unsigned_abs())).await;
            self.adjust(now(), "scheduled").await;
            self.adjust_aircon(now()).await;
        }
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn router(state: Db) -> Router {
    let tasks = state.clone();
    let api = Router::new()
        .route("/health", get(health))
        .route("/devices", get(list_devices).post(create_device))
        .route(
            "/devices/{id}",
            get(get_device).patch(update_device).delete(delete_device),
        )
        .route(
            "/devices/{id}/commission",
            axum::routing::post(commission_device),
        )
        .route("/devices/{id}/wifi", axum::routing::post(move_wifi))
        .route("/devices/{id}/on", axum::routing::post(device_on))
        .route("/devices/{id}/off", axum::routing::post(device_off))
        .route("/labels", get(list_labels).post(create_label))
        .route(
            "/labels/{id}",
            axum::routing::patch(update_label).delete(delete_label),
        )
        .route("/labels/{id}/on", axum::routing::post(label_on))
        .route("/labels/{id}/off", axum::routing::post(label_off))
        .route("/labels/{id}/toggle", axum::routing::post(label_toggle))
        .route("/wifi", get(list_wifi).post(save_wifi))
        .route("/wifi/devices", axum::routing::put(choose_wifi))
        .route("/wifi/{id}", axum::routing::delete(delete_wifi))
        .route("/commission", axum::routing::post(commission))
        .route("/thread", get(thread))
        .route("/meter", get(meter_reading))
        .route("/aircon", get(get_aircon).put(update_aircon))
        .route("/aircon/toggle", axum::routing::post(toggle_aircon))
        .route("/aircon/runs", get(aircon_runs))
        .route("/location", get(get_location).put(update_location))
        .route("/status", get(status))
        .route("/lights", get(light_states))
        .route("/lights/on", axum::routing::post(lights_on))
        .route("/lights/off", axum::routing::post(lights_off))
        .route("/lights/boost", axum::routing::post(boost))
        .route("/lights/schedule", get(get_schedule).put(update_schedule))
        .route("/lights/watch", get(watcher::status))
        .route(
            "/lights/watch/test",
            axum::routing::post(watcher::test_notice),
        )
        .route("/lights/outages", get(watcher::outages))
        .route("/lights/connected", get(watcher::connected))
        .route("/door/unlock", axum::routing::post(unlock_door))
        .route("/door/lock", axum::routing::post(lock_door))
        .fallback(not_found)
        // ponytail: one connection behind a global lock; a pool if writes ever contend.
        .with_state(state);
    let tools = Router::new().nest("/api", api.clone());

    Router::new()
        .route("/healthz", get(healthz).with_state(tasks))
        .route("/api", axum::routing::any(not_found))
        .route("/api/", axum::routing::any(not_found))
        .nest("/api", api)
        .route(
            "/mcp",
            axum::routing::post(move |body: axum::body::Bytes| mcp::handle(tools.clone(), body)),
        )
        .fallback_service(get(admin_screen))
        .layer(axum::middleware::from_fn(log_request))
}

/// Logs each request on one line once answered: method, path without its query, the peer,
/// status and milliseconds. The container's health check polls `/healthz`, logged at debug.
async fn log_request(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map_or_else(|| "-".to_owned(), |info| info.0.ip().to_string());
    let started = std::time::Instant::now();
    let response = next.run(request).await;
    let status = response.status().as_u16();
    let ms = started.elapsed().as_millis();
    if path == "/healthz" {
        tracing::debug!(%method, %path, %peer, status, ms, "request");
    } else {
        tracing::info!(%method, %path, %peer, status, ms, "request");
    }
    response
}

/// The admin screen built into `client/dist`.
static UI: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/client/dist");

/// A built file, or the screen's `index.html` for its own routes.
async fn admin_screen(uri: axum::http::Uri) -> Response {
    let file = UI
        .get_file(uri.path().trim_start_matches('/'))
        .or_else(|| UI.get_file("index.html"))
        .expect("the build requires client/dist/index.html");
    let content_type = mime_guess::from_path(file.path()).first_or_octet_stream();
    (
        [(axum::http::header::CONTENT_TYPE, content_type.as_ref())],
        file.contents(),
    )
        .into_response()
}

struct ApiError(StatusCode, &'static str, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1, "message": self.2 }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        tracing::error!(%error, "database error");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "サーバーでエラーが発生しました".into(),
        )
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn is_unique_violation(error: &rusqlite::Error) -> bool {
    error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation)
}

/// Trims `name` and checks its length; `allow_empty` permits a blank result.
fn clean_name(name: &str, allow_empty: bool) -> Option<String> {
    let name = name.trim();
    let ok = (allow_empty || !name.is_empty()) && name.chars().count() <= MAX_NAME_CHARS;
    ok.then(|| name.to_owned())
}

async fn unlock_door(
    state: State<Db>,
    peer: ConnectInfo<SocketAddr>,
) -> ApiResult<Json<serde_json::Value>> {
    door(state, peer, sesame::Command::Unlock).await
}

async fn lock_door(
    state: State<Db>,
    peer: ConnectInfo<SocketAddr>,
) -> ApiResult<Json<serde_json::Value>> {
    door(state, peer, sesame::Command::Lock).await
}

/// Unlocks or locks the front door; `peer` is only logged.
async fn door(
    State(state): State<Db>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    command: sesame::Command,
) -> ApiResult<Json<serde_json::Value>> {
    let (name, verb) = match command {
        sesame::Command::Unlock => ("unlock", "解錠"),
        sesame::Command::Lock => ("lock", "施錠"),
    };
    let config = state.sesame.as_ref().map_err(|message| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "sesame_not_configured",
            (*message).into(),
        )
    })?;
    match sesame::send(config, command, (state.clock)()).await {
        Ok(()) => {
            tracing::info!(peer = %peer.ip(), "{name} accepted");
            Ok(Json(json!({ "status": "accepted" })))
        }
        Err(sesame::Failure::Rejected(status)) => {
            tracing::warn!(status, "{name} rejected by CANDY HOUSE");
            Err(ApiError(
                StatusCode::BAD_GATEWAY,
                "sesame_rejected",
                format!("SESAMEのAPIが{verb}を受け付けませんでした（HTTP {status}）"),
            ))
        }
        Err(sesame::Failure::Unreachable(error)) => {
            tracing::warn!(%error, "CANDY HOUSE unreachable");
            Err(ApiError(
                StatusCode::BAD_GATEWAY,
                "sesame_unreachable",
                "SESAMEのAPIに接続できませんでした".into(),
            ))
        }
    }
}

/// `ok` with each supervised task on a line of its own, running or stopped and how often it
/// was started again after a panic; 503 once a task has stopped.
async fn healthz(State(state): State<Db>) -> (StatusCode, String) {
    use std::fmt::Write as _;
    let tasks = state.tasks.lock().expect(UNPOISONED);
    let stopped = tasks.values().any(|task| !task.running);
    let mut text = String::from(if stopped { "stopped\n" } else { "ok\n" });
    for (name, task) in tasks.iter() {
        let panicked = task
            .last_panic
            .map(|at| format!(" last_panic={}", schedule::timestamp(at)))
            .unwrap_or_default();
        let _ = writeln!(
            text,
            "{name} {} restarts={}{panicked}",
            if task.running { "running" } else { "stopped" },
            task.restarts,
        );
    }
    let status = if stopped {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (status, text)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn not_found() -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        "not_found",
        "APIが見つかりません".into(),
    )
}

/// A device as listed: its setup code holds the setup passcode, so it is
/// returned only by the single-device endpoint.
#[derive(Serialize)]
struct Device {
    id: i64,
    name: String,
    vendor: Option<String>,
    serial_number: Option<String>,
    mac: Option<String>,
    /// From the QR code; a manual pairing code carries neither.
    vendor_id: Option<u16>,
    product_id: Option<u16>,
    /// `unconnected`, `connected` or `no_response` on matterjs-server; `null` when it cannot be read.
    state: Option<&'static str>,
    /// As the connected device reports it.
    product_name: Option<String>,
    /// `light`, `plug`, `aircon` or `other` by its Matter device types; `null` until it is connected.
    kind: Option<&'static str>,
    /// The On/Off last read of a device that switches and is not a light.
    on: Option<bool>,
    /// The Wi-Fi network the device is on, as it reports it.
    wifi_ssid: Option<String>,
    /// `wifi`, `thread` or `ethernet` as the connected node runs on; a device behind a bridge is on its bridge's.
    network: Option<&'static str>,
    /// The labels whose values the schedule uses for this light.
    labels: Vec<LabelRef>,
    /// Left out of switching, adjusting and the counts.
    ignored: bool,
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    qr_payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manual_code: Option<String>,
}

/// A label as a device lists it.
#[derive(Serialize, Deserialize)]
struct LabelRef {
    id: i64,
    name: String,
}

const DEVICE_SELECT: &str = "SELECT devices.id, devices.name, created_at,
    qr_payload, vendor, serial_number, mac,
    (SELECT json_group_array(json_object('id', labels.id, 'name', labels.name) ORDER BY labels.id)
        FROM device_labels JOIN labels ON labels.id = label_id WHERE device_id = devices.id),
    ignored, vendor_id, product_id
    FROM devices";

/// The `qr_payload` column stores either form; a QR payload starts with `MT:`.
fn device_row(row: &rusqlite::Row, with_payload: bool) -> rusqlite::Result<Device> {
    let code: Option<String> = if with_payload { row.get(3)? } else { None };
    let (qr_payload, manual_code) = match code {
        Some(code) if !code.starts_with("MT:") => (None, Some(code)),
        code => (code, None),
    };
    Ok(Device {
        id: row.get(0)?,
        name: row.get(1)?,
        vendor: row.get(4)?,
        serial_number: row.get(5)?,
        mac: row.get(6)?,
        labels: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
        ignored: row.get(8)?,
        vendor_id: row.get(9)?,
        product_id: row.get(10)?,
        state: None,
        product_name: None,
        kind: None,
        on: None,
        wifi_ssid: None,
        network: None,
        created_at: row.get(2)?,
        qr_payload,
        manual_code,
    })
}

fn find_device(db: &Connection, id: i64, with_payload: bool) -> ApiResult<Device> {
    db.query_row(
        &format!("{DEVICE_SELECT} WHERE devices.id = ?1"),
        [id],
        |row| device_row(row, with_payload),
    )
    .optional()?
    .ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            "device_not_found",
            "機器が見つかりません".into(),
        )
    })
}

async fn list_devices(State(state): State<Db>) -> ApiResult<Json<Vec<Device>>> {
    let mut devices = ledger(&state.db.lock().expect(UNPOISONED))?;
    observe(&state, &mut devices).await;
    Ok(Json(devices))
}

async fn get_device(State(state): State<Db>, Path(id): Path<i64>) -> ApiResult<Json<Device>> {
    let device = find_device(&state.db.lock().expect(UNPOISONED), id, true)?;
    Ok(Json(observed(&state, device).await))
}

/// Sets each device's state and product name from matterjs-server's nodes;
/// left `null` when they cannot be read.
// ponytail: reads every node per request; cache with subscriptions if polled often.
async fn observe(state: &AppState, devices: &mut [Device]) {
    let Some(url) = &state.matter_url else {
        return;
    };
    let nodes = match matter::fetch_nodes(url, link::budget()).await {
        Ok(nodes) => nodes,
        Err(error) => {
            tracing::warn!(%error, "matterjs-server unreachable; device states unknown");
            return;
        }
    };
    for device in devices {
        let (state, product) = match place(device, &nodes) {
            None => ("unconnected", None),
            Some((node, endpoint)) => match matter::describe(&nodes, node, endpoint) {
                (true, product) => ("connected", product),
                (false, product) => ("no_response", product),
            },
        };
        device.state = Some(state);
        device.product_name = product;
        if let Some((kind, on_off)) =
            place(device, &nodes).and_then(|(node, endpoint)| matter::kind(&nodes, node, endpoint))
        {
            device.kind = Some(match kind {
                matter::Kind::Light => "light",
                matter::Kind::Plug => "plug",
                matter::Kind::Aircon => "aircon",
                matter::Kind::Other => "other",
            });
            // The air conditioner is switched on its own page, not as a plug.
            if kind != matter::Kind::Aircon {
                device.on = on_off.and_then(|(_, on)| on);
            }
        }
        device.wifi_ssid = wifi_of(device, &nodes).map(|(_, wifi)| wifi.ssid);
        device.network = place(device, &nodes).and_then(|(node, _)| matter::network(&nodes, node));
    }
}

/// The node of a device on Wi-Fi itself, not behind a bridge, and its network.
fn wifi_of(device: &Device, nodes: &[serde_json::Value]) -> Option<(u64, commission::WifiNetwork)> {
    let (node_id, 0) = place(device, nodes)? else {
        return None;
    };
    let node = nodes.iter().find(|node| node["node_id"] == node_id)?;
    commission::wifi_network(&node["attributes"]).map(|wifi| (node_id, wifi))
}

/// Moves a ledger device onto the Wi-Fi for devices without resetting it; one already on it stays.
async fn move_wifi(
    State(state): State<Db>,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let (device, (ssid, password)) = {
        let db = state.db.lock().expect(UNPOISONED);
        (find_device(&db, id, false)?, wifi_for_devices(&db)?)
    };
    let url = matter_url(&state)?;
    let nodes = matter::fetch_nodes(url, link::budget())
        .await
        .map_err(matter_unreachable)?;
    let (node_id, from) = wifi_of(&device, &nodes).ok_or_else(|| {
        ApiError(
            StatusCode::CONFLICT,
            "not_on_wifi",
            "Wi-Fiでつながっている機器ではありません".into(),
        )
    })?;
    if from.ssid != ssid {
        let moved =
            commission::move_wifi(url, node_id, &from.id, &ssid, &password, state.wifi_move)
                .await
                .map_err(matter_unreachable)?;
        tracing::info!(node_id, device = id, ?moved, "wifi move ended");
        match moved {
            commission::Move::Done => {}
            commission::Move::Reverted => {
                return Err(ApiError(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "wifi_reverted",
                    format!("機器がWi-Fi「{ssid}」につながらず、元のWi-Fiに戻りました"),
                ));
            }
            commission::Move::Reverting => {
                return Err(ApiError(
                    StatusCode::GATEWAY_TIMEOUT,
                    "wifi_reverting",
                    format!(
                        "機器がWi-Fi「{ssid}」につながらず、応答がありません。元のWi-Fiへ戻ります"
                    ),
                ));
            }
        }
    }
    let mut device = observed(&state, device).await;
    // matterjs-server may not have heard of the new network yet.
    device.wifi_ssid = Some(ssid);
    Ok(Json(json!({ "device": device })))
}

async fn device_on(state: State<Db>, id: Path<i64>) -> ApiResult<Json<serde_json::Value>> {
    switch_device(state, id, true).await
}

async fn device_off(state: State<Db>, id: Path<i64>) -> ApiResult<Json<serde_json::Value>> {
    switch_device(state, id, false).await
}

/// Switches one device that is not a light, such as a plug: `on`, `off` or `no_response`
/// with why, sent and sent again as a label's devices are. Lights are left to labels.
async fn switch_device(
    State(state): State<Db>,
    Path(id): Path<i64>,
    on: bool,
) -> ApiResult<Json<serde_json::Value>> {
    let device = find_device(&state.db.lock().expect(UNPOISONED), id, false)?;
    let url = matter_url(&state)?;
    let deadline = link::budget();
    let nodes = matter::fetch_nodes(url, deadline)
        .await
        .map_err(matter_unreachable)?;
    let (node_id, endpoint) = place(&device, &nodes).ok_or_else(|| {
        ApiError(
            StatusCode::CONFLICT,
            "not_connected",
            "機器がつながっていません".into(),
        )
    })?;
    if matter::outlet(&nodes, node_id, endpoint).is_none() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "not_switchable",
            "入り切りできない機器です".into(),
        ));
    }
    let outlet = |nodes: &[serde_json::Value]| {
        matter::outlet(nodes, node_id, endpoint)
            .into_iter()
            .collect()
    };
    let switched = matter::switch_among(url, deadline, on, outlet, |_, _| false)
        .await
        .map_err(matter_unreachable)?;
    let outcome = switched
        .results
        .into_iter()
        .next()
        .map_or(matter::Outcome::NotSent, |(_, outcome)| outcome);
    tracing::info!(
        device = id,
        on,
        result = outcome.result(),
        "device switched"
    );
    match outcome {
        matter::Outcome::Accepted => Ok(Json(json!({ "state": if on { "on" } else { "off" } }))),
        matter::Outcome::Refused(_) => Err(ApiError(
            StatusCode::BAD_GATEWAY,
            "switch_failed",
            "機器が入り切りを受け付けませんでした".into(),
        )),
        missed => Ok(Json(json!({
            "state": "no_response",
            "reason": missed.reason(),
            "details": missed.details(),
        }))),
    }
}

async fn observed(state: &AppState, mut device: Device) -> Device {
    observe(state, std::slice::from_mut(&mut device)).await;
    device
}

/// A device's setup code, as read from its QR code or label.
#[derive(Deserialize)]
struct CodeInput {
    qr_payload: Option<String>,
    manual_code: Option<String>,
}

/// Enters a device into the ledger by its code alone, before it has a name or is
/// connected; a device already there by its code is returned as it is.
async fn create_device(
    State(state): State<Db>,
    Json(input): Json<CodeInput>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let code = setup_code(input.qr_payload.as_ref(), input.manual_code.as_ref())?;
    let (vendor_id, product_id) = onboarding::ids(&code).unzip();
    let (registered, device) = {
        let db = state.db.lock().expect(UNPOISONED);
        if let Some(id) = registered_id(&db, &code)? {
            db.execute(
                "UPDATE devices SET vendor_id = coalesce(vendor_id, ?1),
                    product_id = coalesce(product_id, ?2) WHERE id = ?3",
                params![vendor_id, product_id, id],
            )?;
            (false, find_device(&db, id, false)?)
        } else {
            db.execute(
                "INSERT INTO devices (qr_payload, vendor_id, product_id) VALUES (?1, ?2, ?3)",
                params![code, vendor_id, product_id],
            )?;
            (true, find_device(&db, db.last_insert_rowid(), false)?)
        }
    };
    let status = if registered {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let device = observed(&state, device).await;
    Ok((
        status,
        Json(json!({ "registered": registered, "device": device })),
    ))
}

/// Validates the QR payload or manual pairing code, exactly one of them, and
/// returns the form to store.
fn setup_code(qr_payload: Option<&String>, manual_code: Option<&String>) -> ApiResult<String> {
    match (qr_payload, manual_code) {
        (Some(payload), None) => {
            let payload = payload.trim();
            (payload.len() <= MAX_QR_CHARS && onboarding::is_valid(payload))
                .then(|| payload.to_owned())
                .ok_or_else(|| {
                    ApiError(
                        StatusCode::BAD_REQUEST,
                        "invalid_qr_payload",
                        "MatterのQRコード（MT:で始まる）ではありません".into(),
                    )
                })
        }
        (None, Some(code)) => onboarding::normalize_manual(code.trim()).ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_manual_code",
                "Matterの手動ペアリングコード（11桁の数字）として正しくありません".into(),
            )
        }),
        _ => Err(ApiError(
            StatusCode::BAD_REQUEST,
            "missing_setup_code",
            "QRコードか手動ペアリングコードのどちらか一方を送ってください".into(),
        )),
    }
}

fn device_name(name: &str) -> ApiResult<String> {
    clean_name(name, true).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_device_name",
            format!("機器名は{MAX_NAME_CHARS}文字以内で入力してください"),
        )
    })
}

/// The stored device sharing a passcode and short discriminator with `code`.
// ponytail: scans every device per insert; a key column if the ledger outgrows a home.
fn registered_id(db: &Connection, code: &str) -> rusqlite::Result<Option<i64>> {
    let new = onboarding::keys(code).unwrap_or_default();
    let mut statement = db.prepare("SELECT id, qr_payload FROM devices ORDER BY id")?;
    let stored = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(stored.into_iter().find_map(|(id, code)| {
        onboarding::keys(code.as_deref()?)
            .unwrap_or_default()
            .iter()
            .any(|key| new.contains(key))
            .then_some(id)
    }))
}

/// A ledger change to a device: only the fields given change; `label_ids`
/// replaces its labels.
async fn update_device(
    State(state): State<Db>,
    Path(id): Path<i64>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<Device>> {
    let invalid = || {
        ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_device_update",
            "変更は label_ids（labelのidの配列）・ignored（trueかfalse）・name（文字列）のうち、変える項目だけを送ってください".into(),
        )
    };
    let fields = input
        .as_object()
        .filter(|fields| {
            !fields.is_empty()
                && fields
                    .keys()
                    .all(|key| ["label_ids", "ignored", "name"].contains(&key.as_str()))
        })
        .ok_or_else(invalid)?;
    let label_ids = fields
        .get("label_ids")
        .map(|ids| serde_json::from_value::<Vec<i64>>(ids.clone()).map_err(|_| invalid()))
        .transpose()?;
    let ignored = fields
        .get("ignored")
        .map(|ignored| ignored.as_bool().ok_or_else(invalid))
        .transpose()?;
    let name = fields
        .get("name")
        .map(|name| device_name(name.as_str().ok_or_else(invalid)?))
        .transpose()?;
    let device = {
        let db = state.db.lock().expect(UNPOISONED);
        find_device(&db, id, false)?;
        if let Some(mut ids) = label_ids {
            ids.sort_unstable();
            ids.dedup();
            for label in &ids {
                find_label(&db, *label)?;
            }
            db.execute("DELETE FROM device_labels WHERE device_id = ?1", [id])?;
            for label in ids {
                db.execute(
                    "INSERT INTO device_labels (device_id, label_id) VALUES (?1, ?2)",
                    [id, label],
                )?;
            }
        }
        if let Some(ignored) = ignored {
            db.execute(
                "UPDATE devices SET ignored = ?1 WHERE id = ?2",
                params![ignored, id],
            )?;
        }
        if let Some(name) = name {
            db.execute(
                "UPDATE devices SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;
        }
        find_device(&db, id, false)?
    };
    Ok(Json(observed(&state, device).await))
}

/// A label: what it does to its devices, its daily switches and its state.
#[derive(Clone, Serialize)]
struct Label {
    id: i64,
    name: String,
    #[serde(flatten)]
    traits: schedule::Label,
    schedule: Vec<Switch>,
    device_count: i64,
    /// Holding its lights at its override: what the 0.1.36 widget reads as boosted.
    boosted: bool,
}

/// A daily switch of a label at `time`, `HH:MM` in Asia/Tokyo.
#[derive(Clone, Serialize)]
struct Switch {
    time: String,
    on: bool,
}

const LABEL_SELECT: &str = "SELECT id, name, auto, override_level, override_kelvin, \"on\",
    (SELECT count(*) FROM device_labels WHERE label_id = labels.id),
    (SELECT json_group_array(json_array(minute, \"on\")) FROM
        (SELECT minute, \"on\" FROM label_schedules WHERE label_id = labels.id ORDER BY minute))
    FROM labels";

fn label_row(row: &rusqlite::Row) -> rusqlite::Result<Label> {
    let traits = schedule::Label {
        auto: row.get(2)?,
        override_level: row.get(3)?,
        override_kelvin: row.get(4)?,
        on: row.get(5)?,
    };
    let schedule: Vec<(u32, u8)> =
        serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default();
    Ok(Label {
        id: row.get(0)?,
        name: row.get(1)?,
        boosted: traits.overriding().is_some(),
        traits,
        schedule: schedule
            .into_iter()
            .map(|(minute, on)| Switch {
                time: schedule::clock(minute),
                on: on != 0,
            })
            .collect(),
        device_count: row.get(6)?,
    })
}

fn labels(db: &Connection) -> rusqlite::Result<Vec<Label>> {
    let mut statement = db.prepare(&format!("{LABEL_SELECT} ORDER BY id"))?;
    statement.query_map([], label_row)?.collect()
}

fn label_not_found() -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        "label_not_found",
        "labelが見つかりません".into(),
    )
}

fn find_label(db: &Connection, id: i64) -> ApiResult<Label> {
    db.query_row(&format!("{LABEL_SELECT} WHERE id = ?1"), [id], label_row)
        .optional()?
        .ok_or_else(label_not_found)
}

/// What a label is made of: its name, traits and daily switches by minute.
struct LabelInput {
    name: String,
    traits: schedule::Label,
    schedule: Vec<(u32, bool)>,
}

/// The label in `input`, checked.
fn label_input(input: &serde_json::Value) -> ApiResult<LabelInput> {
    let invalid =
        |message: &str| ApiError(StatusCode::BAD_REQUEST, "invalid_label", message.into());
    let fields = input
        .as_object()
        .ok_or_else(|| invalid("labelはJSONのオブジェクトで送ってください"))?;
    if let Some(key) = fields.keys().find(|key| {
        ![
            "name",
            "auto",
            "override_level",
            "override_kelvin",
            "schedule",
        ]
        .contains(&key.as_str())
    }) {
        return Err(invalid(&format!(
            "{key}は使えません。name・auto・override_level・override_kelvin・scheduleを送ってください"
        )));
    }
    let name = fields
        .get("name")
        .and_then(|name| clean_name(name.as_str()?, false))
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_label_name",
                format!("label名は1〜{MAX_NAME_CHARS}文字で入力してください"),
            )
        })?;
    let auto = match fields.get("auto") {
        None => false,
        Some(serde_json::Value::Bool(auto)) => *auto,
        Some(_) => return Err(invalid("autoはtrueかfalseで送ってください")),
    };
    let number = |key: &str, range: std::ops::RangeInclusive<u64>| match fields.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|v| range.contains(v))
            .map(Some)
            .ok_or_else(|| {
                invalid("override_levelは1〜254、override_kelvinは1000〜10000で送ってください")
            }),
    };
    let (level, kelvin) = (
        number("override_level", 1..=254)?,
        number("override_kelvin", 1000..=10000)?,
    );
    if level.is_some() != kelvin.is_some() {
        return Err(invalid(
            "override_levelとoverride_kelvinは両方を送るか、両方をnullにしてください",
        ));
    }
    if auto && level.is_some() {
        return Err(invalid("上書きの値を持つlabelは自動調光に連動できません"));
    }
    let bad_schedule = || {
        invalid(
            "scheduleは [{\"time\": \"00:00\", \"on\": true}, …] の形で、時刻を重ねずに送ってください",
        )
    };
    let mut schedule = Vec::new();
    for entry in fields
        .get("schedule")
        .map(|s| s.as_array().ok_or_else(bad_schedule))
        .transpose()?
        .into_iter()
        .flatten()
    {
        let minute = entry["time"].as_str().and_then(schedule::minute_of);
        let (Some(minute), Some(on), Some(2)) = (
            minute,
            entry["on"].as_bool(),
            entry.as_object().map(serde_json::Map::len),
        ) else {
            return Err(bad_schedule());
        };
        if schedule.iter().any(|(m, _)| *m == minute) {
            return Err(bad_schedule());
        }
        schedule.push((minute, on));
    }
    Ok(LabelInput {
        name,
        traits: schedule::Label {
            auto,
            override_level: level.and_then(|v| u8::try_from(v).ok()),
            override_kelvin: kelvin.and_then(|v| u16::try_from(v).ok()),
            on: None,
        },
        schedule,
    })
}

/// Saves the label at `id`, or a new one, keeping its state.
fn save_label(db: &Connection, id: Option<i64>, input: &LabelInput) -> ApiResult<i64> {
    let traits = &input.traits;
    let result = db.execute(
        "INSERT INTO labels (id, name, auto, override_level, override_kelvin)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, auto = excluded.auto,
            override_level = excluded.override_level, override_kelvin = excluded.override_kelvin",
        params![
            id,
            input.name,
            traits.auto,
            traits.override_level,
            traits.override_kelvin
        ],
    );
    let id = match result {
        Err(error) if is_unique_violation(&error) => {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "duplicate_label_name",
                "同じ名前のlabelがあります".into(),
            ));
        }
        result => {
            result?;
            id.unwrap_or_else(|| db.last_insert_rowid())
        }
    };
    db.execute("DELETE FROM label_schedules WHERE label_id = ?1", [id])?;
    for (minute, on) in &input.schedule {
        db.execute(
            "INSERT INTO label_schedules (label_id, minute, \"on\") VALUES (?1, ?2, ?3)",
            params![id, minute, on],
        )?;
    }
    Ok(id)
}

async fn list_labels(State(db): State<Db>) -> ApiResult<Json<Vec<Label>>> {
    Ok(Json(labels(&db.db.lock().expect(UNPOISONED))?))
}

async fn create_label(
    State(db): State<Db>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<(StatusCode, Json<Label>)> {
    let input = label_input(&input)?;
    let db = db.db.lock().expect(UNPOISONED);
    let id = save_label(&db, None, &input)?;
    Ok((StatusCode::CREATED, Json(find_label(&db, id)?)))
}

/// Merges the given fields into the label; `null` takes its override away.
async fn update_label(
    State(db): State<Db>,
    Path(id): Path<i64>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<Label>> {
    let db = db.db.lock().expect(UNPOISONED);
    let label = find_label(&db, id)?;
    let mut merged = json!(label.traits);
    merged["name"] = json!(label.name);
    merged["schedule"] = json!(label.schedule);
    if let Some(map) = merged.as_object_mut() {
        map.remove("on");
    }
    if let Some(fields) = input.as_object() {
        for (key, value) in fields {
            merged[key] = value.clone();
        }
    } else {
        merged = input;
    }
    save_label(&db, Some(id), &label_input(&merged)?)?;
    Ok(Json(find_label(&db, id)?))
}

/// Deletes the label; its devices keep their other labels.
async fn delete_label(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let db = db.db.lock().expect(UNPOISONED);
    find_label(&db, id)?;
    db.execute("DELETE FROM labels WHERE id = ?1", [id])?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_device(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let db = db.db.lock().expect(UNPOISONED);
    find_device(&db, id, false)?;
    db.execute("DELETE FROM devices WHERE id = ?1", [id])?;
    Ok(StatusCode::NO_CONTENT)
}

/// A saved Wi-Fi network; its password is write-only and never leaves the ledger but to matterjs-server.
#[derive(Serialize)]
struct Wifi {
    id: i64,
    ssid: String,
    for_devices: bool,
}

#[derive(Deserialize)]
struct WifiInput {
    ssid: String,
    password: String,
    #[serde(default)]
    for_devices: bool,
}

#[derive(Deserialize)]
struct WifiChoice {
    id: i64,
}

fn find_wifi(db: &Connection, id: i64) -> ApiResult<Wifi> {
    db.query_row(
        "SELECT id, ssid, for_devices FROM wifi_networks WHERE id = ?1",
        [id],
        |row| {
            Ok(Wifi {
                id: row.get(0)?,
                ssid: row.get(1)?,
                for_devices: row.get(2)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            "wifi_not_found",
            "Wi-Fiが見つかりません".into(),
        )
    })
}

/// Makes `id` the network handed to devices, the only one.
fn hand_to_devices(db: &Connection, id: i64) -> rusqlite::Result<()> {
    db.execute("UPDATE wifi_networks SET for_devices = (id = ?1)", [id])
        .map(|_| ())
}

async fn list_wifi(State(db): State<Db>) -> ApiResult<Json<Vec<Wifi>>> {
    let db = db.db.lock().expect(UNPOISONED);
    let mut statement = db.prepare("SELECT id FROM wifi_networks ORDER BY id")?;
    let ids = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(Json(
        ids.into_iter()
            .map(|id| find_wifi(&db, id))
            .collect::<ApiResult<_>>()?,
    ))
}

/// Saves a network, replacing the password of one with the same name. The first
/// network, or one sent `for_devices`, becomes the one handed to devices.
async fn save_wifi(
    State(db): State<Db>,
    Json(input): Json<WifiInput>,
) -> ApiResult<(StatusCode, Json<Wifi>)> {
    // SSIDs are up to 32 bytes; WPA passphrases 8–63 characters or 64 hex digits.
    if !(1..=32).contains(&input.ssid.len()) || !(1..=64).contains(&input.password.chars().count())
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_wifi",
            "Wi-Fiの名前（32バイト以内）とパスワード（64文字以内）を送ってください".into(),
        ));
    }
    let db = db.db.lock().expect(UNPOISONED);
    let existing: Option<i64> = db
        .query_row(
            "SELECT id FROM wifi_networks WHERE ssid = ?1",
            [&input.ssid],
            |row| row.get(0),
        )
        .optional()?;
    let id = if let Some(id) = existing {
        db.execute(
            "UPDATE wifi_networks SET password = ?1 WHERE id = ?2",
            params![input.password, id],
        )?;
        id
    } else {
        db.execute(
            "INSERT INTO wifi_networks (ssid, password) VALUES (?1, ?2)",
            params![input.ssid, input.password],
        )?;
        db.last_insert_rowid()
    };
    let none: bool = db.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM wifi_networks WHERE for_devices = 1)",
        [],
        |row| row.get(0),
    )?;
    if input.for_devices || none {
        hand_to_devices(&db, id)?;
    }
    let status = if existing.is_some() {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(find_wifi(&db, id)?)))
}

/// Makes the network the one handed to devices.
async fn choose_wifi(State(db): State<Db>, Json(input): Json<WifiChoice>) -> ApiResult<Json<Wifi>> {
    let db = db.db.lock().expect(UNPOISONED);
    find_wifi(&db, input.id)?;
    hand_to_devices(&db, input.id)?;
    Ok(Json(find_wifi(&db, input.id)?))
}

/// Deletes the network; devices then get the first one left.
async fn delete_wifi(State(db): State<Db>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let db = db.db.lock().expect(UNPOISONED);
    let wifi = find_wifi(&db, id)?;
    db.execute("DELETE FROM wifi_networks WHERE id = ?1", [id])?;
    if wifi.for_devices
        && let Some(first) = db.query_row("SELECT min(id) FROM wifi_networks", [], |row| {
            row.get::<_, Option<i64>>(0)
        })?
    {
        hand_to_devices(&db, first)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The SSID and password handed to devices, if one is chosen.
fn saved_wifi(db: &Connection) -> rusqlite::Result<Option<(String, String)>> {
    db.query_row(
        "SELECT ssid, password FROM wifi_networks WHERE for_devices = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

fn wifi_for_devices(db: &Connection) -> ApiResult<(String, String)> {
    saved_wifi(db)?.ok_or_else(|| {
        ApiError(
            StatusCode::CONFLICT,
            "wifi_not_set",
            "機器に渡すWi-Fiが未登録です".into(),
        )
    })
}

/// A new device to commission over the phone's Bluetooth and record, onto the
/// Wi-Fi saved for devices or onto Thread.
#[derive(Deserialize)]
struct CommissionInput {
    qr_payload: Option<String>,
    manual_code: Option<String>,
    #[serde(default)]
    name: String,
    /// `wifi` (the default) or `thread`.
    network: Option<String>,
}

/// Has matterjs-server commission the device over the BLE proxy the phone holds
/// open, onto the Wi-Fi for devices, then records it: a device already in the ledger
/// (by identifiers, else by code) keeps its entry and gains missing identifiers.
async fn commission(
    State(state): State<Db>,
    Json(input): Json<CommissionInput>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let code = setup_code(input.qr_payload.as_ref(), input.manual_code.as_ref())?;
    let thread = match input.network.as_deref() {
        None | Some("wifi") => false,
        Some("thread") => true,
        Some(_) => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_network",
                "networkは wifi か thread で指定してください".into(),
            ));
        }
    };
    let name = device_name(&input.name)?;
    let url = matter_url(&state)?;
    let (ssid, password) = {
        let db = state.db.lock().expect(UNPOISONED);
        if thread {
            Default::default()
        } else {
            wifi_for_devices(&db)?
        }
    };
    let network = if thread {
        commission::Network::Thread
    } else {
        commission::Network::Wifi {
            ssid: &ssid,
            password: &password,
        }
    };
    let bluetooth = state.meter.hold();
    let commissioned = commission::commission(url, &code, &network, state.commission_timeout)
        .await
        .map_err(|error| match error {
            commission::Error::Unreachable(error) => matter_unreachable(error),
            commission::Error::Failed(failure) => commission_failed(failure, &ssid),
        })?;
    drop(bluetooth);
    let db = state.db.lock().expect(UNPOISONED);
    let (registered, device) = record(&db, &name, &code, &commissioned)?;
    tracing::info!(
        node_id = commissioned.node_id,
        registered,
        device = device.id,
        "device commissioned"
    );
    Ok((
        if registered {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({
            "node_id": commissioned.node_id,
            "registered": registered,
            "device": device,
        })),
    ))
}

/// Has matterjs-server commission the ledger device `id` with its stored code over
/// the BLE proxy the phone holds open, handing over the saved Wi-Fi for devices when
/// there is one and leaving Wi-Fi or Thread to the device, then fills in the row's
/// identifiers. A device the ledger already holds under another row keeps that row.
async fn commission_device(
    State(state): State<Db>,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let (code, wifi) = {
        let db = state.db.lock().expect(UNPOISONED);
        let device = find_device(&db, id, true)?;
        let code = device.qr_payload.or(device.manual_code).ok_or_else(|| {
            ApiError(
                StatusCode::CONFLICT,
                "no_setup_code",
                "この機器にはQRコードも手動ペアリングコードもありません".into(),
            )
        })?;
        (code, saved_wifi(&db)?)
    };
    let url = matter_url(&state)?;
    let network = commission::Network::Any(
        wifi.as_ref()
            .map(|(ssid, password)| (ssid.as_str(), password.as_str())),
    );
    let ssid = wifi.as_ref().map_or("", |(ssid, _)| ssid.as_str());
    let bluetooth = state.meter.hold();
    let commissioned = commission::commission(url, &code, &network, state.commission_timeout)
        .await
        .map_err(|error| match error {
            commission::Error::Unreachable(error) => matter_unreachable(error),
            commission::Error::Failed(failure) => commission_failed(failure, ssid),
        })?;
    drop(bluetooth);
    let device = connect_row(&state.db.lock().expect(UNPOISONED), id, &commissioned)?;
    tracing::info!(
        node_id = commissioned.node_id,
        device = device.id,
        "device commissioned"
    );
    let device = observed(&state, device).await;
    Ok(Json(json!({
        "node_id": commissioned.node_id,
        "device": device,
    })))
}

/// Fills row `id` with the commissioned device's identifiers. When another row
/// already holds them, that row stays the device's, taking this row's code, and
/// and its name where it has none, and this row goes.
fn connect_row(
    db: &Connection,
    id: i64,
    commissioned: &commission::Commissioned,
) -> ApiResult<Device> {
    let (vendor, serial) = commissioned.identity.clone().unzip();
    let mac = &commissioned.mac;
    let tx = db.unchecked_transaction()?;
    let other: Option<i64> = tx
        .query_row(
            "SELECT id FROM devices
             WHERE id != ?4 AND ((vendor = ?1 AND serial_number = ?2 COLLATE NOCASE) OR mac = ?3)
             ORDER BY id LIMIT 1",
            params![vendor, serial, mac, id],
            |row| row.get(0),
        )
        .optional()?;
    let target = if let Some(other) = other {
        let scanned = find_device(&tx, id, true)?;
        tx.execute("DELETE FROM devices WHERE id = ?1", [id])?;
        tx.execute(
            "UPDATE devices SET qr_payload = coalesce(qr_payload, ?1),
                name = CASE name WHEN '' THEN ?2 ELSE name END,
                vendor_id = coalesce(vendor_id, ?3), product_id = coalesce(product_id, ?4)
             WHERE id = ?5",
            params![
                scanned.qr_payload.or(scanned.manual_code),
                scanned.name,
                scanned.vendor_id,
                scanned.product_id,
                other
            ],
        )?;
        other
    } else {
        id
    };
    tx.execute(
        "UPDATE devices SET vendor = coalesce(?1, vendor),
            serial_number = coalesce(?2, serial_number), mac = coalesce(?3, mac)
         WHERE id = ?4",
        params![vendor, serial, mac, target],
    )?;
    let device = find_device(&tx, target, false)?;
    tx.commit()?;
    Ok(device)
}

fn commission_failed(failure: commission::Failure, ssid: &str) -> ApiError {
    use commission::Failure;
    tracing::warn!(?failure, "commissioning failed");
    let (status, code, message) = match failure {
        Failure::NotFound => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "device_not_found",
            "ペアリング待ちの機器がBluetoothで見つかりませんでした".to_owned(),
        ),
        Failure::WrongCode => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "wrong_code",
            "機器がコードを受け付けませんでした".to_owned(),
        ),
        Failure::Wifi => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "wifi_failed",
            if ssid.is_empty() {
                "機器がWi-Fiに接続できませんでした".to_owned()
            } else {
                format!("機器がWi-Fi「{ssid}」に接続できませんでした")
            },
        ),
        Failure::Thread => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "thread_failed",
            "機器がThread網に参加できませんでした".to_owned(),
        ),
        Failure::ThreadNotReady => (
            StatusCode::SERVICE_UNAVAILABLE,
            "thread_not_ready",
            "Thread網が未準備です".to_owned(),
        ),
        Failure::Timeout => (
            StatusCode::GATEWAY_TIMEOUT,
            "commission_timeout",
            "時間内に登録が終わりませんでした".to_owned(),
        ),
        Failure::BluetoothDisabled => (
            StatusCode::SERVICE_UNAVAILABLE,
            "bluetooth_unavailable",
            "matterjs-serverでBluetoothが有効になっていません".to_owned(),
        ),
        Failure::Other(details) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "commission_failed",
            format!("登録できませんでした: {details}"),
        ),
    };
    ApiError(status, code, message)
}

/// Whether Thread devices can be commissioned: matterjs-server holds a dataset.
/// The meter's latest reading while it is fresh, and when it was received.
async fn meter_reading(State(state): State<Db>) -> Json<serde_json::Value> {
    Json(state.meter.view((state.clock)()))
}

async fn thread(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let ready = commission::thread_ready(matter_url(&state)?, THREAD_TIMEOUT)
        .await
        .map_err(matter_unreachable)?;
    Ok(Json(json!({ "ready": ready })))
}

const THREAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Records a commissioned device; `false` with the existing entry when the
/// ledger already holds it.
fn record(
    db: &Connection,
    name: &str,
    code: &str,
    commissioned: &commission::Commissioned,
) -> ApiResult<(bool, Device)> {
    let (vendor, serial) = commissioned.identity.clone().unzip();
    let mac = &commissioned.mac;
    let by_identifier: Option<i64> = db
        .query_row(
            "SELECT id FROM devices WHERE (vendor = ?1 AND serial_number = ?2) OR mac = ?3
             ORDER BY id LIMIT 1",
            params![vendor, serial, mac],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = by_identifier.or(registered_id(db, code)?) {
        db.execute(
            "UPDATE devices SET vendor = coalesce(vendor, ?1),
                serial_number = coalesce(serial_number, ?2), mac = coalesce(mac, ?3)
             WHERE id = ?4",
            params![vendor, serial, mac, id],
        )?;
        return Ok((false, find_device(db, id, false)?));
    }
    db.execute(
        "INSERT INTO devices (qr_payload, vendor, serial_number, mac, name)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![code, vendor, serial, mac, name],
    )?;
    Ok((true, find_device(db, db.last_insert_rowid(), false)?))
}

fn matter_url(state: &AppState) -> ApiResult<&str> {
    state.matter_url.as_deref().ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "matter_server_not_configured",
            "matterjs-serverの接続先（MATTER_SERVER_URL）が設定されていません".into(),
        )
    })
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "taken by value to be passed to `map_err` by name"
)]
fn matter_unreachable(fault: link::Fault) -> ApiError {
    tracing::warn!(%fault, "matterjs-server unreachable");
    let message = match fault {
        link::Fault::Unreachable(_) => "matterjs-serverに接続できませんでした",
        link::Fault::Timeout(_) => "matterjs-serverが時間内に応答しませんでした",
        link::Fault::Dropped(_) => "matterjs-serverとの接続が途中で切れました",
        link::Fault::Failed(_) => "matterjs-serverから機器を読めませんでした",
    };
    ApiError(
        StatusCode::BAD_GATEWAY,
        "matter_server_unreachable",
        message.into(),
    )
}

/// A ledger device and where matterjs-server currently serves it.
#[derive(Serialize)]
struct DeviceStatus {
    id: i64,
    name: String,
    visible: bool,
    node_id: Option<u64>,
    endpoint: Option<u16>,
}

/// Locates every ledger device on matterjs-server by its vendor and serial number.
/// Devices it does not serve, or without identifiers, are not visible and need
/// registering again.
// ponytail: reads every node per request; cache with subscriptions if polled often.
async fn status(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let nodes = matter::fetch_nodes(matter_url(&state)?, link::budget())
        .await
        .map_err(matter_unreachable)?;
    let db = state.db.lock().expect(UNPOISONED);
    let mut statement = db.prepare(&format!("{DEVICE_SELECT} ORDER BY devices.id"))?;
    let devices = statement
        .query_map([], |row| device_row(row, false))?
        .map(|device| {
            let device = device?;
            let place = place(&device, &nodes);
            Ok(DeviceStatus {
                id: device.id,
                name: device.name,
                visible: place.is_some(),
                node_id: place.map(|(node, _)| node),
                endpoint: place.map(|(_, endpoint)| endpoint),
            })
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "devices": devices,
    })))
}

/// A ledger device matterjs-server does not serve.
#[derive(Serialize)]
struct MissingDevice {
    id: i64,
    name: String,
}

/// A light's ledger name (else its product name) and whether it is ignored.
type LightName = (Option<String>, bool);

fn ledger(db: &Connection) -> rusqlite::Result<Vec<Device>> {
    let mut statement = db.prepare(&format!("{DEVICE_SELECT} ORDER BY devices.id"))?;
    statement
        .query_map([], |row| device_row(row, false))?
        .collect()
}

fn place(device: &Device, nodes: &[serde_json::Value]) -> Option<(u64, u16)> {
    device
        .vendor
        .as_deref()
        .zip(device.serial_number.as_deref())
        .and_then(|(vendor, serial)| matter::locate(nodes, vendor, serial))
}

/// The ledger devices that are `light`, latest first: a device located on
/// endpoint 0 is the lights of its node that are not behind a bridge; a bridged
/// device the light on its own endpoint.
fn devices_of<'a>(
    devices: &'a [Device],
    nodes: &'a [serde_json::Value],
    light: &'a matter::Light,
) -> impl Iterator<Item = &'a Device> {
    devices.iter().rev().filter(move |device| {
        place(device, nodes).is_some_and(|(node, endpoint)| {
            light.node_id == node
                && if light.bridged {
                    light.endpoint == endpoint
                } else {
                    endpoint == 0
                }
        })
    })
}

/// Names `lights` from the ledger and lists the ledger devices `nodes` lack.
fn match_ledger(
    devices: &[Device],
    nodes: &[serde_json::Value],
    lights: &[matter::Light],
) -> (Vec<LightName>, Vec<MissingDevice>) {
    let names = lights
        .iter()
        .map(|light| {
            let name = devices_of(devices, nodes, light)
                .find(|device| !device.name.is_empty())
                .map_or(light.product.clone(), |device| Some(device.name.clone()));
            (name, is_ignored(devices, nodes, light))
        })
        .collect();
    let missing = devices
        .iter()
        .filter(|device| !device.ignored && is_missing(device, nodes))
        .map(|device| MissingDevice {
            id: device.id,
            name: device.name.clone(),
        })
        .collect();
    (names, missing)
}

/// A device once connected, by its identifiers, that matterjs-server no longer serves.
/// A device never connected has none and is not missing.
fn is_missing(device: &Device, nodes: &[serde_json::Value]) -> bool {
    device.serial_number.is_some() && place(device, nodes).is_none()
}

fn is_ignored(devices: &[Device], nodes: &[serde_json::Value], light: &matter::Light) -> bool {
    devices_of(devices, nodes, light).any(|device| device.ignored)
}

/// Each light with its ledger name (else its product name), and `field`'s value.
fn light_entries(
    nodes: &[serde_json::Value],
    lights: &[matter::Light],
    names: Vec<LightName>,
    values: &[&str],
    field: &str,
) -> Vec<serde_json::Value> {
    lights
        .iter()
        .zip(names)
        .zip(values)
        .map(|((light, (name, _)), value)| {
            json!({
                "node_id": light.node_id,
                "endpoint": light.endpoint,
                "name": name,
                "network": matter::network(nodes, light.node_id),
                field: value,
            })
        })
        .collect()
}

fn count(values: &[&str], value: &str) -> usize {
    values.iter().filter(|v| **v == value).count()
}

/// Counts the lights on, off and not responding, as matterjs-server last read them,
/// and the ledger devices it does not serve.
// ponytail: reads every node per request; cache with subscriptions if polled often.
async fn light_states(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let nodes = matter::fetch_nodes(matter_url(&state)?, link::budget())
        .await
        .map_err(matter_unreachable)?;
    let lights = matter::lights(&nodes);
    let (names, missing) = match_ledger(
        &ledger(&state.db.lock().expect(UNPOISONED))?,
        &nodes,
        &lights,
    );
    let states: Vec<&str> = lights
        .iter()
        .zip(&names)
        .map(
            |(light, (_, ignored))| match (ignored, light.reachable, light.on) {
                (true, ..) => "ignored",
                (_, true, Some(true)) => "on",
                (_, true, Some(false)) => "off",
                _ => "no_response",
            },
        )
        .collect();
    Ok(Json(json!({
        "on": count(&states, "on"),
        "off": count(&states, "off"),
        "no_response": count(&states, "no_response"),
        "missing": missing.len(),
        "lights": light_entries(&nodes, &lights, names, &states, "state"),
        "missing_devices": missing,
    })))
}

/// Switches every light on, as the app's "all on" did until it moves to labels: each label
/// without an override is on, and a light under override labels alone is left to them.
/// When the schedule is on, the lights are brought to the current values.
async fn lights_on(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let deadline = link::budget();
    let (response, switched) = switch_lights(&state, true, deadline).await?;
    if load::<schedule::Settings>(&state.db.lock().expect(UNPOISONED), SCHEDULE)?.enabled {
        adjust(
            &state,
            (state.clock)(),
            "lights_on",
            Some(&switched),
            deadline,
        )
        .await;
    }
    Ok(response)
}

/// Switches every light off and every label with it, as the app's "all off" did.
async fn lights_off(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    Ok(switch_lights(&state, false, link::budget()).await?.0)
}

/// The label whose lights to boost or release.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoostInput {
    label: String,
}

/// Toggles the label named in the body, for the 0.1.36 widget's 仕事 and 作業:
/// `boost` when it switched it on, `release` otherwise.
async fn boost(
    State(state): State<Db>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = serde_json::from_value::<BoostInput>(input)
        .map_err(|_| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_boost",
                "本文は {\"label\": \"labelの名前\"} の形で送ってください".into(),
            )
        })?
        .label;
    let id = labels(&state.db.lock().expect(UNPOISONED))?
        .into_iter()
        .find(|l| l.name == name.trim())
        .ok_or_else(label_not_found)?
        .id;
    let mut body = switch_label(&state, id, Op::Toggle)
        .await
        .map_err(|error| match error.1 {
            "label_has_no_devices" => ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                "label_has_no_lights",
                format!("labelの「{}」に照明が割り当てられていません", name.trim()),
            ),
            _ => error,
        })?;
    body["action"] = json!(if body["action"] == "on" {
        "boost"
    } else {
        "release"
    });
    Ok(Json(body))
}

/// What to do to a label's devices.
#[derive(Clone, Copy)]
enum Op {
    On,
    Off,
    Toggle,
}

async fn label_on(
    State(state): State<Db>,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    switch_label(&state, id, Op::On).await.map(Json)
}

async fn label_off(
    State(state): State<Db>,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    switch_label(&state, id, Op::Off).await.map(Json)
}

async fn label_toggle(
    State(state): State<Db>,
    Path(id): Path<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    switch_label(&state, id, Op::Toggle).await.map(Json)
}

/// Runs the label switches due at `unix`'s minute, and those of the minutes since the last run
/// that it missed, once each. Each label's switches run in their minutes' order, each label
/// apart from the others, so one that waits on a silent device holds up no other.
async fn run_schedules(state: &AppState, unix: i64) {
    let minute = unix.div_euclid(60);
    let minutes = {
        let mut last = state.last_schedule.lock().expect(UNPOISONED);
        let minutes = schedule::minutes_due(*last, minute);
        if minutes.is_empty() {
            return;
        }
        *last = Some(minute);
        minutes
    };
    let due = due_schedules(&state.db.lock().expect(UNPOISONED), minutes);
    let mut by_label: Vec<(i64, Vec<bool>)> = Vec::new();
    for (label, on) in due.unwrap_or_else(|error| {
        tracing::warn!(%error, "label schedules unreadable");
        Vec::new()
    }) {
        match by_label.iter_mut().find(|(l, _)| *l == label) {
            Some((_, ons)) => ons.push(on),
            None => by_label.push((label, vec![on])),
        }
    }
    futures_util::future::join_all(by_label.into_iter().map(|(label, ons)| async move {
        let lock = state
            .schedule_locks
            .lock()
            .expect(UNPOISONED)
            .entry(label)
            .or_default()
            .clone();
        let _turn = lock.lock().await;
        for on in ons {
            run_schedule(state, label, on).await;
        }
    }))
    .await;
}

/// The label switches of `minutes`, by label and in their minutes' order.
fn due_schedules(
    db: &Connection,
    minutes: std::ops::RangeInclusive<i64>,
) -> rusqlite::Result<Vec<(i64, bool)>> {
    let mut statement = db.prepare(
        "SELECT label_id, \"on\" FROM label_schedules WHERE minute = ?1 ORDER BY label_id",
    )?;
    let mut due = Vec::new();
    for minute in minutes {
        let (_, at) = schedule::local(minute * 60);
        for row in statement.query_map([at], |row| Ok((row.get(0)?, row.get(1)?)))? {
            due.push(row?);
        }
    }
    Ok(due)
}

/// Switches label `label` on or off for its schedule, then sends again to the devices that
/// did not answer, or to all when matterjs-server could not be reached, up to
/// `SCHEDULE_RETRIES` times: unless the label was switched otherwise meanwhile.
async fn run_schedule(state: &AppState, label: i64, on: bool) {
    let op = if on { Op::On } else { Op::Off };
    let mut only: Option<Vec<(u64, u16)>> = None;
    let mut left = None;
    for attempt in 0..=SCHEDULE_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(state.schedule_retry).await;
            let now = find_label(&state.db.lock().expect(UNPOISONED), label).map(|l| l.traits.on);
            if now.ok() != left {
                tracing::info!(
                    label,
                    on,
                    "label switched otherwise; schedule not sent again"
                );
                return;
            }
        }
        match switch_label_among(state, label, op, only.as_deref()).await {
            Ok(body) => {
                tracing::info!(label, on, attempt, switched = %body["switched"], "label switched on schedule");
                let missed = missed_devices(&body);
                if missed.is_empty() {
                    return;
                }
                left = serde_json::from_value(body["on"].clone()).ok();
                only = Some(missed);
            }
            Err(error) if error.1 == "matter_server_unreachable" => {
                tracing::warn!(label, on, attempt, error = error.1, "label schedule failed");
                left = find_label(&state.db.lock().expect(UNPOISONED), label)
                    .ok()
                    .map(|l| l.traits.on);
            }
            Err(error) => {
                tracing::warn!(label, on, attempt, error = error.1, "label schedule failed");
                return;
            }
        }
    }
    tracing::warn!(label, on, "label schedule left devices that never answered");
}

/// The node and endpoint of each device a switch's response says did not answer, or that
/// matterjs-server failed to reach: worth sending again, unlike a device's refusal.
fn missed_devices(body: &serde_json::Value) -> Vec<(u64, u16)> {
    body["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["result"] == "no_response" || d["reason"] == "server_error")
        .filter_map(|d| {
            Some((
                d["node_id"].as_u64()?,
                u16::try_from(d["endpoint"].as_u64()?).ok()?,
            ))
        })
        .collect()
}

/// Switches label `id`'s devices, lights and plugs, but those ignored.
///
/// On switches them on, lights under an override at its values, and keeps the label on
/// when one of them switched; when one is adjusted, the schedule then runs. Off switches
/// them off and ends every override of theirs. A toggle turns an override label that is
/// on back to what its lights' other labels say: held by another override, adjusted
/// under an auto label that is on, else off; any other label that is on, off; else on.
///
/// A label takes its new state as soon as the switch reaches one of its devices, before any
/// values are sent: values that cannot be sent are that light's result, not the request's.
async fn switch_label(state: &AppState, id: i64, op: Op) -> ApiResult<serde_json::Value> {
    switch_label_among(state, id, op, None).await
}

/// `switch_label` over the label's devices at `only`'s node and endpoint, or all of them.
async fn switch_label_among(
    state: &AppState,
    id: i64,
    op: Op,
    only: Option<&[(u64, u16)]>,
) -> ApiResult<serde_json::Value> {
    let switch = LabelSwitch::load(state, id, only)?;
    let url = matter_url(state)?;
    let fixed = switch.label.traits.override_level.is_some();
    let action = match op {
        Op::On => "on",
        Op::Toggle if switch.label.traits.on != Some(true) => "on",
        Op::Toggle if fixed => "release",
        Op::Off | Op::Toggle => "off",
    };
    let deadline = link::budget();
    let run = match action {
        "on" => switch.on(state, url, deadline).await?,
        "off" => switch.off(state, url, deadline).await?,
        _ => switch.release(state, url, deadline).await?,
    };
    switch.report(state, action, run)
}

/// What a label's switch needs: the ledger and labels as they were, the label and its devices.
struct LabelSwitch<'a> {
    settings: schedule::Settings,
    devices: Vec<Device>,
    labels: Vec<Label>,
    label: Label,
    /// The ids of the label's devices, but those ignored.
    members: Vec<i64>,
    only: Option<&'a [(u64, u16)]>,
}

/// What a label's switch did: the nodes read, each device's result, the values sent to its
/// lights, and whether matterjs-server answered none of its commands.
struct LabelRun {
    nodes: Vec<serde_json::Value>,
    results: Vec<(matter::Light, Done)>,
    tuned: Option<matter::Tuned>,
    none_answered: bool,
}

fn key(light: &matter::Light) -> (u64, u16) {
    (light.node_id, light.endpoint)
}

impl<'a> LabelSwitch<'a> {
    fn load(state: &AppState, id: i64, only: Option<&'a [(u64, u16)]>) -> ApiResult<Self> {
        let (settings, devices, labels) = {
            let db = state.db.lock().expect(UNPOISONED);
            (
                load::<schedule::Settings>(&db, SCHEDULE)?,
                ledger(&db)?,
                labels(&db)?,
            )
        };
        let label = labels
            .iter()
            .find(|l| l.id == id)
            .ok_or_else(label_not_found)?
            .clone();
        let members: Vec<i64> = devices
            .iter()
            .filter(|d| !d.ignored && d.labels.iter().any(|r| r.id == id))
            .map(|d| d.id)
            .collect();
        if members.is_empty() {
            return Err(ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                "label_has_no_devices",
                format!("labelの「{}」に機器が割り当てられていません", label.name),
            ));
        }
        Ok(Self {
            settings,
            devices,
            labels,
            label,
            members,
            only,
        })
    }

    /// The label's lights and plugs among `nodes`, once each.
    fn targets(&self, nodes: &[serde_json::Value]) -> Vec<matter::Light> {
        let is_member = |device: &Device| self.members.contains(&device.id);
        let mut targets: Vec<matter::Light> = matter::lights(nodes)
            .into_iter()
            .filter(|light| devices_of(&self.devices, nodes, light).any(is_member))
            .chain(
                self.devices
                    .iter()
                    .filter(|device| is_member(device))
                    .filter_map(|device| {
                        let (node, endpoint) = place(device, nodes)?;
                        matter::outlet(nodes, node, endpoint)
                    }),
            )
            .filter(|light| self.only.is_none_or(|only| only.contains(&key(light))))
            .collect();
        targets.sort_by_key(key);
        targets.dedup_by_key(|t| key(t));
        targets
    }

    /// What `light`'s labels say with the labels as `labels` holds them.
    fn rule(
        &self,
        nodes: &[serde_json::Value],
        light: &matter::Light,
        labels: &[Label],
    ) -> schedule::Rule {
        schedule::rule(
            labels_of(&self.devices, nodes, light, labels)
                .iter()
                .map(|l| &l.traits),
        )
    }

    /// The labels as they are once this label is `on`.
    fn after(&self, on: bool) -> Vec<Label> {
        let mut after = self.labels.clone();
        for l in after.iter_mut().filter(|l| l.id == self.label.id) {
            l.traits.on = Some(on);
        }
        after
    }

    async fn on(
        &self,
        state: &AppState,
        url: &str,
        deadline: tokio::time::Instant,
    ) -> ApiResult<LabelRun> {
        let after = self.after(true);
        let switched = matter::switch_among(url, deadline, true, |n| self.targets(n), |_, _| false)
            .await
            .map_err(matter_unreachable)?;
        let accepted = |o: &matter::Outcome| *o == matter::Outcome::Accepted;
        let reached = reached(&self.devices, &switched.nodes, &switched.results, accepted);
        set_labels(
            state,
            &settled([self.label.id], &self.devices, &reached),
            true,
        )?;
        let on: Vec<_> = switched
            .results
            .iter()
            .filter(|(_, o)| accepted(o))
            .map(|(light, _)| key(light))
            .collect();
        let fixed: Vec<_> = switched
            .results
            .iter()
            .filter(|(light, o)| {
                accepted(o)
                    && matches!(
                        self.rule(&switched.nodes, light, &after),
                        schedule::Rule::Fixed(..)
                    )
            })
            .map(|(light, _)| key(light))
            .collect();
        let tuned = if fixed.is_empty() {
            Ok(None)
        } else {
            matter::tune(
                url,
                deadline,
                &on,
                matter::TRANSITION,
                |nodes, light| match self.rule(nodes, light, &after) {
                    schedule::Rule::Fixed(level, kelvin) if fixed.contains(&key(light)) => {
                        Ok(schedule::fit(level, kelvin, light))
                    }
                    _ => Err(schedule::Skip::Unchanged),
                },
            )
            .await
            .map(Some)
        };
        let results = switched
            .results
            .into_iter()
            .map(|(light, switch)| {
                let done = match switch {
                    matter::Outcome::Accepted if fixed.contains(&key(&light)) => {
                        Done::after(&tuned, &light)
                    }
                    matter::Outcome::Accepted => Done::switched(),
                    missed => Done::of(missed),
                };
                (light, done)
            })
            .collect::<Vec<_>>();
        let adjusted = results.iter().any(|(light, _)| {
            on.contains(&key(light))
                && self.rule(&switched.nodes, light, &after) == schedule::Rule::Auto
        });
        if adjusted && self.settings.enabled {
            adjust(state, (state.clock)(), "label_on", Some(&on), deadline).await;
        }
        Ok(LabelRun {
            nodes: switched.nodes,
            results,
            tuned: tuned.ok().flatten(),
            none_answered: switched.none_answered,
        })
    }

    async fn off(
        &self,
        state: &AppState,
        url: &str,
        deadline: tokio::time::Instant,
    ) -> ApiResult<LabelRun> {
        let switched =
            matter::switch_among(url, deadline, false, |n| self.targets(n), |_, _| false)
                .await
                .map_err(matter_unreachable)?;
        let reached = reached(
            &self.devices,
            &switched.nodes,
            &switched.results,
            |o: &matter::Outcome| *o == matter::Outcome::Accepted,
        );
        // Every override holding a light that went off ends with it.
        let overrides = self
            .labels
            .iter()
            .filter(|l| l.traits.override_level.is_some())
            .map(|l| l.id);
        let ended = settled(
            std::iter::once(self.label.id).chain(overrides),
            &self.devices,
            &reached,
        );
        set_labels(state, &ended, false)?;
        Ok(LabelRun {
            results: switched
                .results
                .into_iter()
                .map(|(light, o)| (light, Done::of(o)))
                .collect(),
            nodes: switched.nodes,
            tuned: None,
            none_answered: switched.none_answered,
        })
    }

    async fn release(
        &self,
        state: &AppState,
        url: &str,
        deadline: tokio::time::Instant,
    ) -> ApiResult<LabelRun> {
        let after = self.after(false);
        let stays = |nodes: &[serde_json::Value], light: &matter::Light| {
            matches!(
                self.rule(nodes, light, &after),
                schedule::Rule::Fixed(..) | schedule::Rule::Auto
            )
        };
        let switched = matter::switch_among(url, deadline, false, |n| self.targets(n), stays)
            .await
            .map_err(matter_unreachable)?;
        // A light left to its other labels was released as much as one switched off.
        let reached = reached(
            &self.devices,
            &switched.nodes,
            &switched.results,
            |o: &matter::Outcome| matches!(o, matter::Outcome::Accepted | matter::Outcome::Ignored),
        );
        set_labels(
            state,
            &settled([self.label.id], &self.devices, &reached),
            false,
        )?;
        let auto: Vec<_> = switched
            .results
            .iter()
            .filter(|(light, _)| self.rule(&switched.nodes, light, &after) == schedule::Rule::Auto)
            .map(|(light, _)| key(light))
            .collect();
        let tuned = if auto.is_empty() {
            Ok(None)
        } else {
            let (day, minute) = schedule::local((state.clock)());
            let home = load::<outside::Location>(&state.db.lock().expect(UNPOISONED), LOCATION)?;
            let goal = schedule::target(
                minute,
                schedule::sun_times(day, home.latitude, home.longitude),
                &self.settings,
            );
            matter::tune(url, deadline, &[], 0, |nodes, light| {
                if auto.contains(&key(light))
                    && self.rule(nodes, light, &after) == schedule::Rule::Auto
                {
                    Ok(schedule::fit(goal.0, goal.1, light))
                } else {
                    Err(schedule::Skip::Unchanged)
                }
            })
            .await
            .map(Some)
        };
        let results = switched
            .results
            .into_iter()
            .map(|(light, o)| {
                let done = match self.rule(&switched.nodes, &light, &after) {
                    // Another override holds it: nothing was sent.
                    schedule::Rule::Fixed(..) => Done::held(),
                    schedule::Rule::Auto => Done::after(&tuned, &light),
                    _ => Done::of(o),
                };
                (light, done)
            })
            .collect();
        Ok(LabelRun {
            nodes: switched.nodes,
            results,
            tuned: tuned.ok().flatten(),
            none_answered: switched.none_answered,
        })
    }

    /// The response to the switch, logged on one line; the values sent are remembered.
    fn report(
        &self,
        state: &AppState,
        action: &str,
        run: LabelRun,
    ) -> ApiResult<serde_json::Value> {
        if let Some(tuned) = &run.tuned {
            let mut memory = state.sent.lock().expect(UNPOISONED);
            for (light, decision) in &tuned.lights {
                decide(&mut memory, light, decision, &mut json!({}));
            }
        }
        let label = find_label(&state.db.lock().expect(UNPOISONED), self.label.id)?;
        let nodes = run.nodes;
        let (lights, done): (Vec<_>, Vec<Done>) = run.results.into_iter().unzip();
        let outcomes: Vec<&str> = done.iter().map(|d| d.result).collect();
        let (names, _) = match_ledger(&self.devices, &nodes, &lights);
        let missing: Vec<_> = self
            .devices
            .iter()
            .filter(|d| self.members.contains(&d.id) && is_missing(d, &nodes))
            .map(|d| MissingDevice {
                id: d.id,
                name: d.name.clone(),
            })
            .collect();
        tracing::info!(
            label = label.name,
            action,
            switched = count(&outcomes, "switched"),
            no_response = count(&outcomes, "no_response"),
            failed = count(&outcomes, "failed"),
            missing = missing.len(),
            none_answered = run.none_answered,
            devices = lights.len(),
            "label switched"
        );
        let mut entries = light_entries(&nodes, &lights, names, &outcomes, "result");
        Done::explain(&mut entries, &done);
        Ok(json!({
            "label": label.name,
            "action": action,
            "on": label.traits.on,
            "boosted": label.boosted,
            "switched": count(&outcomes, "switched"),
            "no_response": count(&outcomes, "no_response"),
            "failed": count(&outcomes, "failed"),
            "missing": missing.len(),
            "none_answered": run.none_answered,
            "devices": entries,
            "missing_devices": missing,
        }))
    }
}

/// The ids of the ledger devices behind the `results` whose outcome `took` holds.
fn reached(
    devices: &[Device],
    nodes: &[serde_json::Value],
    results: &[(matter::Light, matter::Outcome)],
    took: impl Fn(&matter::Outcome) -> bool,
) -> Vec<i64> {
    results
        .iter()
        .filter(|(_, outcome)| took(outcome))
        .flat_map(|(light, _)| devices_of(devices, nodes, light).map(|d| d.id))
        .collect()
}

/// The labels among `candidates` that take a switch's new state: those with a device among
/// `reached`. A label none of whose devices the switch reached keeps its state, as its lights do.
fn settled(
    candidates: impl IntoIterator<Item = i64>,
    devices: &[Device],
    reached: &[i64],
) -> Vec<i64> {
    candidates
        .into_iter()
        .filter(|label| {
            devices
                .iter()
                .any(|d| reached.contains(&d.id) && d.labels.iter().any(|r| r.id == *label))
        })
        .collect()
}

/// Sets the labels `ids` on or off.
fn set_labels(state: &AppState, ids: &[i64], on: bool) -> rusqlite::Result<()> {
    let db = state.db.lock().expect(UNPOISONED);
    for id in ids {
        db.execute(
            "UPDATE labels SET \"on\" = ?1 WHERE id = ?2",
            params![on, id],
        )?;
    }
    Ok(())
}

/// One device's result as the API names it, and the outcome behind it when it was not
/// switched: why, and matterjs-server's details.
struct Done {
    result: &'static str,
    missed: Option<matter::Outcome>,
}

impl Done {
    fn switched() -> Self {
        Self {
            result: "switched",
            missed: None,
        }
    }

    fn of(outcome: matter::Outcome) -> Self {
        let result = outcome.result();
        Self {
            result,
            missed: outcome.reason().is_some().then_some(outcome),
        }
    }

    /// A switched light's result after the values it was then sent.
    fn tuned(decision: &matter::Decision) -> Self {
        match decision {
            matter::Decision::NoResponse(outcome) => Self {
                result: "no_response",
                missed: Some(outcome.clone()),
            },
            matter::Decision::Failed(outcome) => Self {
                result: "failed",
                missed: Some(outcome.clone()),
            },
            _ => Self::switched(),
        }
    }

    /// A light another override holds: nothing was sent it.
    fn held() -> Self {
        Self {
            result: "override",
            missed: None,
        }
    }

    /// A switched light's result after the values it was then to be sent: what became of
    /// them, or matterjs-server's failure to take them as the light's own.
    fn after(tuned: &Result<Option<matter::Tuned>, link::Fault>, light: &matter::Light) -> Self {
        match tuned {
            Ok(tuned) => tuned
                .iter()
                .flat_map(|t| &t.lights)
                .find(|(l, _)| key(l) == key(light))
                .map_or_else(Self::switched, |(_, d)| Self::tuned(d)),
            Err(fault) => {
                let outcome = matter::Outcome::ServerError(format!("matterjs-server: {fault}"));
                outcome.warn("tune", light.node_id, light.endpoint);
                Self {
                    result: outcome.result(),
                    missed: Some(outcome),
                }
            }
        }
    }

    /// Adds to each of `entries` why its device missed, and the details, as `reason` and `details`.
    fn explain(entries: &mut [serde_json::Value], done: &[Self]) {
        for (entry, done) in entries.iter_mut().zip(done) {
            let missed = done.missed.as_ref();
            entry["reason"] = json!(missed.and_then(matter::Outcome::reason));
            entry["details"] = json!(missed.and_then(matter::Outcome::details));
        }
    }
}

/// Switches every light matterjs-server serves on or off and reports each light's
/// result, with the node and endpoint of the switched ones: "all on" turns every label
/// without an override on, "all off" every label off, each once one of its devices switched.
/// Plugs are left alone. Only a light whose command was accepted counts as switched.
async fn switch_lights(
    state: &AppState,
    on: bool,
    deadline: tokio::time::Instant,
) -> ApiResult<(Json<serde_json::Value>, Vec<(u64, u16)>)> {
    let url = matter_url(state)?;
    let (devices, labels) = {
        let db = state.db.lock().expect(UNPOISONED);
        (ledger(&db)?, labels(&db)?)
    };
    // "All on" leaves a light under override labels alone to them.
    let held = |nodes: &[serde_json::Value], light: &matter::Light| {
        let mine = labels_of(&devices, nodes, light, &labels);
        on && !mine.is_empty() && mine.iter().all(|l| l.traits.override_level.is_some())
    };
    let switched = matter::switch(url, deadline, on, |nodes, light| {
        is_ignored(&devices, nodes, light) || held(nodes, light)
    })
    .await
    .map_err(matter_unreachable)?;
    let nodes = switched.nodes;
    let reached = reached(&devices, &nodes, &switched.results, |o| {
        *o == matter::Outcome::Accepted
    });
    let candidates = labels
        .iter()
        .filter(|l| !on || l.traits.override_level.is_none())
        .map(|l| l.id);
    set_labels(state, &settled(candidates, &devices, &reached), on)?;
    let (lights, outcomes): (Vec<_>, Vec<_>) = switched.results.into_iter().unzip();
    let on_now = lights
        .iter()
        .zip(&outcomes)
        .filter(|(_, outcome)| **outcome == matter::Outcome::Accepted)
        .map(|(light, _)| (light.node_id, light.endpoint))
        .collect();
    let done: Vec<Done> = lights
        .iter()
        .zip(outcomes)
        .map(|(light, outcome)| match outcome {
            matter::Outcome::Ignored if is_ignored(&devices, &nodes, light) => Done {
                result: "ignored",
                missed: None,
            },
            matter::Outcome::Ignored => Done {
                result: "override",
                missed: None,
            },
            outcome => Done::of(outcome),
        })
        .collect();
    let outcomes: Vec<&str> = done.iter().map(|d| d.result).collect();
    let (names, missing) = match_ledger(&devices, &nodes, &lights);
    tracing::info!(
        on,
        switched = count(&outcomes, "switched"),
        no_response = count(&outcomes, "no_response"),
        failed = count(&outcomes, "failed"),
        missing = missing.len(),
        none_answered = switched.none_answered,
        lights = lights.len(),
        "lights switched"
    );
    let mut entries = light_entries(&nodes, &lights, names, &outcomes, "result");
    Done::explain(&mut entries, &done);
    let response = Json(json!({
        "action": if on { "on" } else { "off" },
        "switched": count(&outcomes, "switched"),
        "no_response": count(&outcomes, "no_response"),
        "failed": count(&outcomes, "failed"),
        "missing": missing.len(),
        "none_answered": switched.none_answered,
        "lights": entries,
        "missing_devices": missing,
    }));
    Ok((response, on_now))
}

const SCHEDULE: &str = "light_schedule";

fn load<T: DeserializeOwned + Default>(db: &Connection, key: &str) -> rusqlite::Result<T> {
    let value: Option<String> = db
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(value
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

fn store<T: Serialize>(db: &Connection, key: &str, value: &T) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, serde_json::to_string(value).unwrap_or_default()],
    )?;
    Ok(())
}

/// Adjusts every light that is on under an auto label that is not off to the values
/// for `unix`, and records the run: nothing is written to a light held by an override,
/// under no auto label or only under auto labels that are off, nor while the schedule
/// is disabled, nor what was already sent unless `switched_on` holds the light: just
/// switched on, it gets all values.
async fn adjust(
    state: &AppState,
    unix: i64,
    trigger: &'static str,
    switched_on: Option<&[(u64, u16)]>,
    deadline: tokio::time::Instant,
) {
    let loaded = {
        let db = state.db.lock().expect(UNPOISONED);
        load::<schedule::Settings>(&db, SCHEDULE).and_then(|settings| {
            Ok((
                settings,
                ledger(&db)?,
                labels(&db)?,
                load::<outside::Location>(&db, LOCATION)?,
            ))
        })
    };
    let (day, minute) = schedule::local(unix);
    let mut run = json!({ "at": schedule::timestamp(unix), "trigger": trigger, "commands": 0 });
    let result = match (&loaded, &state.matter_url) {
        (Err(error), _) => Err(error.to_string()),
        (_, None) => Err("MATTER_SERVER_URL is not set".into()),
        (Ok((settings, devices, labels, home)), Some(url)) => {
            let sun = schedule::sun_times(day, home.latitude, home.longitude);
            let goal = schedule::target(minute, sun, settings);
            run["sunrise"] = json!(schedule::clock(sun.0));
            run["sunset"] = json!(schedule::clock(sun.1));
            run["level"] = json!(goal.0);
            run["kelvin"] = json!(goal.1);
            let sent = state.sent.lock().expect(UNPOISONED).clone();
            let goal_of = |nodes: &[serde_json::Value], light: &matter::Light| {
                let labels = labels_of(devices, nodes, light, labels);
                let rule = schedule::rule(labels.iter().map(|l| &l.traits));
                let names: Vec<_> = labels.iter().map(|l| l.name.clone()).collect();
                let target = match rule {
                    schedule::Rule::Fixed(level, kelvin) => (level, kelvin),
                    _ => goal,
                };
                (names, rule, target)
            };
            let just_on = switched_on.unwrap_or_default();
            matter::tune(
                url,
                deadline,
                just_on,
                matter::TRANSITION,
                |nodes, light| {
                    if is_ignored(devices, nodes, light) {
                        return Err(schedule::Skip::Ignored);
                    }
                    let key = (light.node_id, light.endpoint);
                    // After a switch, the lights it did not switch on wait for the schedule,
                    // so one that never answers holds up no request.
                    if switched_on.is_some_and(|on| !on.contains(&key)) {
                        return Err(schedule::Skip::Unchanged);
                    }
                    let last = sent.get(&key).copied().filter(|_| !just_on.contains(&key));
                    match goal_of(nodes, light).1 {
                        schedule::Rule::Fixed(..) => Err(schedule::Skip::Overridden),
                        schedule::Rule::AutoOff => Err(schedule::Skip::AllOff),
                        schedule::Rule::Manual => Err(schedule::Skip::Manual),
                        schedule::Rule::Auto => schedule::plan(light, settings, goal, last),
                    }
                },
            )
            .await
            .map_err(|fault| fault.to_string())
            .map(|tuned| {
                let goals: Vec<_> = tuned
                    .lights
                    .iter()
                    .map(|(light, _)| goal_of(&tuned.nodes, light))
                    .collect();
                (tuned, goals)
            })
        }
    };
    match result {
        Ok((tuned, goals)) => {
            let devices = loaded.map(|(_, devices, _, _)| devices).unwrap_or_default();
            let entries = adjusted(state, &devices, &tuned, goals);
            let sent = entries.iter().filter(|e| e["decision"] == "sent").count();
            tracing::info!(trigger, commands = tuned.commands, sent, "lights adjusted");
            run["commands"] = json!(tuned.commands);
            run["lights"] = json!(entries);
        }
        Err(error) => {
            tracing::warn!(trigger, %error, "light adjustment failed");
            run["error"] = json!(error);
            run["lights"] = json!([]);
        }
    }
    let mut runs = state.runs.lock().expect(UNPOISONED);
    runs.push_front(run);
    runs.truncate(RUNS_KEPT);
}

/// Each light of an adjustment as its run records it: name, labels, goal and decision, with
/// the values sent remembered.
fn adjusted(
    state: &AppState,
    devices: &[Device],
    tuned: &matter::Tuned,
    goals: Vec<(Vec<String>, schedule::Rule, (u8, u16))>,
) -> Vec<serde_json::Value> {
    let lights: Vec<_> = tuned.lights.iter().map(|(l, _)| l.clone()).collect();
    let (names, _) = match_ledger(devices, &tuned.nodes, &lights);
    let mut memory = state.sent.lock().expect(UNPOISONED);
    tuned
        .lights
        .iter()
        .zip(names)
        .zip(goals)
        .map(|(((light, decision), (name, _)), (labels, _, goal))| {
            let mut entry = json!({
                "node_id": light.node_id,
                "endpoint": light.endpoint,
                "name": name,
                "labels": labels,
                "target_level": goal.0,
                "target_kelvin": goal.1,
            });
            entry["decision"] = decide(&mut memory, light, decision, &mut entry);
            entry
        })
        .collect()
}

/// The labels of `light`'s ledger device among `labels`, in their order.
fn labels_of<'a>(
    devices: &[Device],
    nodes: &[serde_json::Value],
    light: &matter::Light,
    labels: &'a [Label],
) -> Vec<&'a Label> {
    devices_of(devices, nodes, light)
        .find(|d| !d.labels.is_empty())
        .map(|d| {
            labels
                .iter()
                .filter(|l| d.labels.iter().any(|r| r.id == l.id))
                .collect()
        })
        .unwrap_or_default()
}

/// `decision` for the run's `entry`, with why a light missed, remembering what was sent `light`.
fn decide(
    memory: &mut HashMap<(u64, u16), schedule::Target>,
    light: &matter::Light,
    decision: &matter::Decision,
    entry: &mut serde_json::Value,
) -> serde_json::Value {
    match decision {
        matter::Decision::Skipped(skip) => json!(skip),
        matter::Decision::Off => json!("off"),
        matter::Decision::NoResponse(outcome) | matter::Decision::Failed(outcome) => {
            entry["reason"] = json!(outcome.reason());
            entry["details"] = json!(outcome.details());
            json!(Done::tuned(decision).result)
        }
        matter::Decision::Sent(target) => {
            let last = memory
                .entry((light.node_id, light.endpoint))
                .or_insert(*target);
            last.level = target.level.or(last.level);
            last.mireds = target.mireds.or(last.mireds);
            entry["level"] = json!(target.level);
            entry["mireds"] = json!(target.mireds);
            json!("sent")
        }
    }
}

/// A ledger device with a label.
#[derive(Serialize)]
struct LabelledDevice {
    id: i64,
    name: String,
}

/// The schedule's settings, each label with its devices and the latest runs.
async fn get_schedule(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let (settings, labels) = {
        let db = state.db.lock().expect(UNPOISONED);
        let devices = ledger(&db)?;
        let labels: Vec<_> = labels(&db)?
            .into_iter()
            .map(|label| {
                let mut entry = json!(label);
                entry["devices"] = json!(
                    devices
                        .iter()
                        .filter(|d| d.labels.iter().any(|r| r.id == label.id))
                        .map(|d| LabelledDevice {
                            id: d.id,
                            name: d.name.clone(),
                        })
                        .collect::<Vec<_>>()
                );
                entry
            })
            .collect();
        (load::<schedule::Settings>(&db, SCHEDULE)?, labels)
    };
    let runs: Vec<_> = state
        .runs
        .lock()
        .expect(UNPOISONED)
        .iter()
        .cloned()
        .collect();
    Ok(Json(json!({
        "settings": settings,
        "labels": labels,
        "runs": runs,
    })))
}

/// Merges the given fields into the schedule's settings.
async fn update_schedule(
    State(state): State<Db>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let invalid = |message: String| ApiError(StatusCode::BAD_REQUEST, "invalid_schedule", message);
    {
        let db = state.db.lock().expect(UNPOISONED);
        let mut settings = json!(load::<schedule::Settings>(&db, SCHEDULE)?);
        let fields = input
            .as_object()
            .ok_or_else(|| invalid("設定はJSONのオブジェクトで送ってください".into()))?;
        for (key, value) in fields {
            settings[key] = value.clone();
        }
        let settings: schedule::Settings =
            serde_json::from_value(settings).map_err(|e| invalid(e.to_string()))?;
        if let Some(message) = settings.invalid() {
            return Err(invalid(message.into()));
        }
        store(&db, SCHEDULE, &settings)?;
    }
    get_schedule(State(state)).await
}

const AIRCON: &str = "aircon";
/// The command last accepted by the air conditioner, kept so a restart does not make it beep.
const AIRCON_SENT: &str = "aircon_sent";
/// What the adjustment remembers between runs: corrections, when it last sent, the room's answer.
const AIRCON_MEMORY: &str = "aircon_memory";
/// The home's place, for the temperature outside and the lights' sunrise and sunset.
const LOCATION: &str = "location";
/// Two weeks of runs every ten minutes are kept, and the latest day served with the state.
const AIRCON_RUNS_FOR: i64 = 14 * 86_400;
/// How long a temperature fetched from outside is reused before it is asked for again.
const OUTDOOR_REUSED_FOR: i64 = 5 * 60;

/// The ledger's air conditioner served by matterjs-server, with its node and the endpoint it is driven on.
// ponytail: the first one; a second room's air conditioner needs a meter of its own.
fn find_aircon<'a>(
    devices: &'a [Device],
    nodes: &[serde_json::Value],
) -> Option<(&'a Device, u64, u16)> {
    devices.iter().filter(|d| !d.ignored).find_map(|device| {
        let (node, endpoint) = place(device, nodes)?;
        match matter::kind(nodes, node, endpoint)? {
            (matter::Kind::Aircon, Some((driven, _))) => Some((device, node, driven)),
            _ => None,
        }
    })
}

/// The answer's body from `url` with `query`; the error leaves the URL out, as the query may hold the address.
async fn fetch_outside(url: &str, query: &[(&str, String)]) -> Result<String, String> {
    let response = outside::client()
        .get(url)
        .query(query)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.without_url().to_string())?;
    response
        .text()
        .await
        .map_err(|e| e.without_url().to_string())
}

/// The temperature outside the home for `unix`, fresh or `None`: asked for again once the last
/// fetch is five minutes old, the last one kept if that fails.
async fn outdoor(state: &AppState, unix: i64) -> Option<f64> {
    let outside = state.outside.as_ref()?;
    let cached = *state.outdoor.lock().expect(UNPOISONED);
    if cached.is_none_or(|o| !(0..OUTDOOR_REUSED_FOR).contains(&(unix - o.fetched_at))) {
        let home = load::<outside::Location>(&state.db.lock().expect(UNPOISONED), LOCATION);
        let fetched = match home {
            Ok(home) => fetch_outside(
                &outside.open_meteo_url,
                &[
                    ("latitude", home.latitude.to_string()),
                    ("longitude", home.longitude.to_string()),
                    ("current", "temperature_2m".into()),
                ],
            )
            .await
            .and_then(|body| outside::temperature(&body).ok_or_else(|| "unreadable answer".into())),
            Err(error) => Err(error.to_string()),
        };
        match fetched {
            Ok((celsius, at)) => {
                *state.outdoor.lock().expect(UNPOISONED) = Some(Outdoor {
                    celsius,
                    at,
                    fetched_at: unix,
                });
            }
            Err(error) => tracing::warn!(%error, "temperature outside unavailable"),
        }
    }
    let latest = *state.outdoor.lock().expect(UNPOISONED);
    latest
        .filter(|o| unix - o.at <= outside::FRESH_FOR)
        .map(|o| o.celsius)
}

/// Decides and sends the air conditioner's command for `unix`, logs the run on one line
/// (`aircon adjusted`, sent or not) and records it for two weeks; what became of it, and why.
async fn adjust_aircon(
    state: &AppState,
    unix: i64,
    trigger: &'static str,
) -> (&'static str, Option<String>) {
    let _busy = state.aircon_busy.lock().await;
    let reading = state
        .meter
        .reading(unix)
        .map(|r| (r.temperature, r.humidity));
    let outdoor = outdoor(state, unix).await;
    let mut run = json!({
        "at": schedule::timestamp(unix),
        "trigger": trigger,
        "temperature": reading.map(|r| r.0),
        "humidity": reading.map(|r| r.1),
        "outdoor": outdoor,
    });
    let result = match drive_aircon(state, unix, trigger, reading, outdoor, &mut run).await {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(trigger, %error, "aircon adjustment failed");
            run["error"] = json!(error);
            "failed"
        }
    };
    tracing::info!(
        trigger,
        result,
        command = %run["command"],
        temperature = %run["temperature"],
        humidity = %run["humidity"],
        outdoor = %run["outdoor"],
        reason = run["reason"].as_str().unwrap_or_default(),
        "aircon adjusted"
    );
    run["result"] = json!(result);
    let db = state.db.lock().expect(UNPOISONED);
    let kept = db
        .execute(
            "INSERT INTO aircon_runs (at, run) VALUES (?1, ?2)",
            params![unix, run.to_string()],
        )
        .and_then(|_| {
            db.execute(
                "DELETE FROM aircon_runs WHERE at < ?1",
                [unix - AIRCON_RUNS_FOR],
            )
        });
    if let Err(error) = kept {
        tracing::warn!(%error, "aircon run not recorded");
    }
    (result, run["reason"].as_str().map(str::to_owned))
}

/// Whether a run's `result` left the air conditioner as it was told to be.
fn aircon_delivered(result: &str) -> bool {
    !matches!(
        result,
        "no_aircon" | "no_response" | "refused" | "failed" | "pending"
    )
}

/// Why a run's `result` did not reach the air conditioner.
fn aircon_missed(result: &str) -> &'static str {
    match result {
        "no_aircon" => "台帳にエアコンがありません",
        "no_response" => "エアコンが応答しませんでした",
        "refused" => "エアコンが命令を受け付けませんでした",
        "pending" => "時間内に送信の結果が分かりませんでした",
        _ => "matterjs-serverへ送れませんでした",
    }
}

/// What became of one run: `sent`, `unchanged`, `held`, `no_reading`, `no_aircon`,
/// `no_response` or `refused`. The memory moves on as the command was accepted or not.
async fn drive_aircon(
    state: &AppState,
    unix: i64,
    trigger: &'static str,
    reading: Option<(f64, u8)>,
    outdoor: Option<f64>,
    run: &mut serde_json::Value,
) -> Result<&'static str, String> {
    let (settings, last, memory, devices) = {
        let db = state.db.lock().expect(UNPOISONED);
        let loaded = (|| {
            Ok::<_, rusqlite::Error>((
                load::<aircon::Settings>(&db, AIRCON)?,
                load::<Option<aircon::Command>>(&db, AIRCON_SENT)?,
                load::<aircon::Memory>(&db, AIRCON_MEMORY)?,
                ledger(&db)?,
            ))
        })();
        loaded.map_err(|e| e.to_string())?
    };
    let input = aircon::Input {
        now: unix,
        room: reading,
        outdoor,
        trigger: match trigger {
            "switch" => aircon::Trigger::Switch,
            "target" => aircon::Trigger::Target,
            _ => aircon::Trigger::Loop,
        },
    };
    let decision = aircon::decide(settings, &input, last, &memory);
    run["reason"] = json!(decision.to_string());
    run["command"] = json!(decision.command);
    let keep = |memory: &aircon::Memory| {
        store(&state.db.lock().expect(UNPOISONED), AIRCON_MEMORY, memory).map_err(|e| e.to_string())
    };
    let Some(command) = decision.send else {
        keep(&decision.kept)?;
        return Ok(match (decision.why, decision.hold) {
            (aircon::Why::NoReading, _) => "no_reading",
            (_, Some(_)) => "held",
            (_, None) => "unchanged",
        });
    };
    let result = send_aircon(state, &devices, command).await;
    if matches!(result, Ok("sent")) {
        let db = state.db.lock().expect(UNPOISONED);
        store(&db, AIRCON_SENT, &Some(command))
            .and_then(|()| store(&db, AIRCON_MEMORY, &decision.sent))
            .map_err(|e| e.to_string())?;
    } else {
        keep(&decision.kept)?;
    }
    result
}

/// Puts `command` on the ledger's air conditioner: `sent`, `no_aircon`, `no_response` or `refused`.
async fn send_aircon(
    state: &AppState,
    devices: &[Device],
    command: aircon::Command,
) -> Result<&'static str, String> {
    let url = state
        .matter_url
        .as_deref()
        .ok_or("MATTER_SERVER_URL is not set")?;
    let deadline = link::budget();
    let nodes = matter::fetch_nodes(url, deadline)
        .await
        .map_err(|fault| fault.to_string())?;
    let Some((_, node, endpoint)) = find_aircon(devices, &nodes) else {
        return Ok("no_aircon");
    };
    if !matter::describe(&nodes, node, endpoint).0 {
        return Ok("no_response");
    }
    let attributes = nodes
        .iter()
        .find(|n| n["node_id"] == node)
        .map(|n| &n["attributes"]);
    let cooling =
        attributes.and_then(|a| a[format!("{endpoint}/{}", aircon::COOLING_SETPOINT)].as_i64());
    let steps = aircon::steps(command, cooling);
    let outcome = matter::drive(url, deadline, node, endpoint, &steps)
        .await
        .map_err(|fault| fault.to_string())?;
    // matterjs-server's own error, such as the node not connected, is no answer from the device.
    Ok(match outcome {
        matter::Outcome::Accepted => "sent",
        matter::Outcome::Refused(_) => "refused",
        _ => "no_response",
    })
}

/// The runs recorded by `sql` with `param`, as JSON.
fn recorded_runs(
    db: &Connection,
    sql: &str,
    param: i64,
) -> rusqlite::Result<Vec<serde_json::Value>> {
    let runs = db
        .prepare(sql)?
        .query_map([param], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(runs
        .iter()
        .filter_map(|run| serde_json::from_str(run).ok())
        .collect())
}

#[derive(Deserialize)]
struct RunsQuery {
    since: Option<i64>,
}

/// The two weeks of runs, oldest first, from `since` (Unix seconds) if given.
async fn aircon_runs(
    State(state): State<Db>,
    axum::extract::Query(query): axum::extract::Query<RunsQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let runs = recorded_runs(
        &state.db.lock().expect(UNPOISONED),
        "SELECT run FROM aircon_runs WHERE at >= ?1 ORDER BY rowid",
        query.since.unwrap_or(0),
    )?;
    Ok(Json(json!({ "runs": runs })))
}

/// The air conditioner's switch, target and night, its state as matterjs-server serves it,
/// the meter's fresh reading, the outside and the home's place, the command last sent with
/// the latest reason, and the latest runs.
async fn get_aircon(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let (settings, sent, devices, home, runs) = {
        let db = state.db.lock().expect(UNPOISONED);
        (
            load::<aircon::Settings>(&db, AIRCON)?,
            load::<Option<aircon::Command>>(&db, AIRCON_SENT)?,
            ledger(&db)?,
            load::<outside::Location>(&db, LOCATION)?,
            recorded_runs(
                &db,
                "SELECT run FROM aircon_runs ORDER BY rowid DESC LIMIT ?1",
                i64::try_from(RUNS_KEPT).unwrap_or(i64::MAX),
            )?,
        )
    };
    let mut device = serde_json::Value::Null;
    if let Some(url) = &state.matter_url {
        match matter::fetch_nodes(url, link::budget()).await {
            Ok(nodes) => {
                if let Some((found, node, endpoint)) = find_aircon(&devices, &nodes) {
                    let attributes =
                        &nodes.iter().find(|n| n["node_id"] == node).expect("found")["attributes"];
                    let at = |path: &str| &attributes[format!("{endpoint}/{path}")];
                    let mode = at(aircon::SYSTEM_MODE).as_u64();
                    let setpoint = if mode == Some(4) {
                        aircon::HEATING_SETPOINT
                    } else {
                        aircon::COOLING_SETPOINT
                    };
                    #[allow(clippy::cast_precision_loss, reason = "0.01℃ steps of 16–32℃")]
                    let setpoint = at(setpoint).as_i64().map(|c| c as f64 / 100.0);
                    device = json!({
                        "id": found.id,
                        "name": found.name,
                        "state": if matter::describe(&nodes, node, endpoint).0 { "connected" } else { "no_response" },
                        "on": at("6/0").as_bool(),
                        "mode": match mode {
                            Some(0) => Some("off"),
                            Some(1) => Some("auto"),
                            Some(3) => Some("cool"),
                            Some(4) => Some("heat"),
                            Some(7) => Some("fan_only"),
                            Some(8) => Some("dry"),
                            _ => None,
                        },
                        "setpoint": setpoint,
                    });
                }
            }
            Err(error) => {
                tracing::warn!(%error, "matterjs-server unreachable; aircon state unknown");
            }
        }
    }
    let now = (state.clock)();
    let outdoor = state.outdoor.lock().expect(UNPOISONED).map(|o| {
        json!({
            "temperature": o.celsius,
            "at": schedule::timestamp(o.at),
            "fetched_at": schedule::timestamp(o.fetched_at),
            "fresh": now - o.at <= outside::FRESH_FOR,
        })
    });
    let setpoint = match sent {
        Some(aircon::Command::On { celsius, .. }) => Some(celsius),
        _ => None,
    };
    Ok(Json(json!({
        "on": settings.on,
        "target": settings.target,
        "night_from": settings.night_from,
        "night_until": settings.night_until,
        "device": device,
        "reading": state.meter.reading(now),
        "outdoor": outdoor,
        "location": {
            "name": home.name,
            "latitude": home.latitude,
            "longitude": home.longitude,
            "registered": home.name.is_some(),
        },
        "sent": sent,
        "setpoint": setpoint,
        "reason": runs.first().map(|run| run["reason"].clone()),
        "runs": runs,
    })))
}

/// Merges `on`, `target`, `night_from` and `night_until` into the settings and sends what
/// they call for at once.
async fn update_aircon(
    State(state): State<Db>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let invalid =
        |message: &str| ApiError(StatusCode::BAD_REQUEST, "invalid_aircon", message.into());
    let fields = input
        .as_object()
        .ok_or_else(|| invalid("設定はJSONのオブジェクトで送ってください"))?;
    let trigger = {
        let db = state.db.lock().expect(UNPOISONED);
        let mut settings = load::<aircon::Settings>(&db, AIRCON)?;
        if let Some(on) = fields.get("on") {
            settings.on = on
                .as_bool()
                .ok_or_else(|| invalid("on は true か false です"))?;
        }
        if let Some(target) = fields.get("target") {
            settings.target = target
                .as_u64()
                .and_then(|t| u8::try_from(t).ok())
                .filter(|t| aircon::TARGETS.contains(t))
                .ok_or_else(|| invalid("目標の温度は18〜30℃の整数です"))?;
        }
        for (key, end) in [
            ("night_from", &mut settings.night_from),
            ("night_until", &mut settings.night_until),
        ] {
            if let Some(minute) = fields.get(key) {
                *end = minute
                    .as_u64()
                    .and_then(|m| u16::try_from(m).ok())
                    .filter(|m| *m < aircon::MINUTES_A_DAY)
                    .ok_or_else(|| invalid("夜の時刻は0時からの分（0〜1439）です"))?;
            }
        }
        store(&db, AIRCON, &settings)?;
        if fields.contains_key("on") {
            "switch"
        } else if fields.contains_key("target") {
            "target"
        } else {
            "scheduled"
        }
    };
    // Only the night changed: nothing to send until the loop's next run.
    if trigger != "scheduled" {
        adjust_aircon(&state, (state.clock)(), trigger).await;
    }
    get_aircon(State(state)).await
}

/// Switches the air conditioner's control off if on, else on, as the widget does, and answers
/// once the air conditioner was sent the change, or `TOGGLE_WAIT` passed: `delivered` says
/// whether it reached the air conditioner, `result` and `reason` what became of it. The switch
/// is kept either way, and the loop sends it again.
async fn toggle_aircon(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let on = {
        let db = state.db.lock().expect(UNPOISONED);
        let mut settings = load::<aircon::Settings>(&db, AIRCON)?;
        settings.on = !settings.on;
        store(&db, AIRCON, &settings)?;
        settings.on
    };
    // Apart from the request, so the send goes on when the answer can wait no longer.
    let run = tokio::spawn({
        let state = state.clone();
        async move { adjust_aircon(&state, (state.clock)(), "switch").await }
    });
    let (result, reason) = match tokio::time::timeout(TOGGLE_WAIT, run).await {
        Ok(Ok(run)) => run,
        Ok(Err(_)) => ("failed", None),
        Err(_) => ("pending", None),
    };
    let delivered = aircon_delivered(result);
    let reason = if delivered {
        reason
    } else {
        Some(aircon_missed(result).to_owned())
    };
    Ok(Json(json!({
        "on": on,
        "result": result,
        "delivered": delivered,
        "reason": reason,
    })))
}

/// The home's place: the address registered, the place found for it and its coordinates.
async fn get_location(State(state): State<Db>) -> ApiResult<Json<serde_json::Value>> {
    let home = load::<outside::Location>(&state.db.lock().expect(UNPOISONED), LOCATION)?;
    let registered = home.name.is_some();
    let mut view = json!(home);
    view["registered"] = json!(registered);
    Ok(Json(view))
}

/// The longest address searched for, in characters.
const MAX_ADDRESS_CHARS: usize = 200;

/// Registers the home's `address` as the first place GSI's address search finds for it; the
/// last place stays when none is found or the search is not reached.
async fn update_location(
    State(state): State<Db>,
    Json(input): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let address = input["address"]
        .as_str()
        .map(str::trim)
        .filter(|a| !a.is_empty() && a.chars().count() <= MAX_ADDRESS_CHARS)
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "invalid_location",
                format!("住所は1〜{MAX_ADDRESS_CHARS}文字で入力してください"),
            )
        })?;
    let outside = state.outside.as_ref().ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "outside_unavailable",
            "住所検索の接続先が設定されていません".into(),
        )
    })?;
    let body = fetch_outside(&outside.gsi_url, &[("q", address.to_owned())])
        .await
        .map_err(|error| {
            tracing::warn!(%error, "address search unreachable");
            ApiError(
                StatusCode::BAD_GATEWAY,
                "address_search_unreachable",
                "住所検索（国土地理院）に接続できませんでした。前の位置のままです".into(),
            )
        })?;
    let (name, latitude, longitude) = outside::place(&body).ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            "address_not_found",
            "住所が見つかりませんでした。前の位置のままです".into(),
        )
    })?;
    store(
        &state.db.lock().expect(UNPOISONED),
        LOCATION,
        &outside::Location {
            address: Some(address.to_owned()),
            name: Some(name),
            latitude,
            longitude,
        },
    )?;
    // The next run asks for the new place's temperature.
    *state.outdoor.lock().expect(UNPOISONED) = None;
    get_location(State(state)).await
}
