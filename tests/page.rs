//! The admin screen in a real browser, for what only a browser engine shows: how each screen
//! lies out at a desk's and a phone's width in both themes. What the screens say and what they
//! ask of home-link is the jsdom tests' (`client/src/pages/page.*.test.ts`).
//!
//! Its own runner (`harness = false`): one home-link and one chromedriver with one session,
//! each on a port the system gives, and the tests one after another in the order written. The
//! words given to `cargo test` pick the tests whose titles hold one of them.
//! home-link runs the built binary on a ledger in a temporary directory, with no
//! matterjs-server and the outside services pointed at a closed port. What it would only answer
//! with real devices (the meter, the air conditioners, the light watch, what asks for the user,
//! the PCs) the page is given by a script that answers those requests in the page.
#[path = "support/browser.rs"]
mod browser;

use browser::{Page, Scratch, TRIES, free_port};
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    process::{Child, Command, ExitCode, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// home-link, started from the built binary on a ledger of its own.
struct Home {
    child: Child,
    url: String,
    db: std::path::PathBuf,
    http: reqwest::Client,
    _scratch: Scratch,
}

impl Home {
    async fn start() -> Self {
        let scratch = Scratch::new("page");
        let db = scratch.path().join("home-link.db");
        let http = reqwest::Client::new();
        for _ in 0..TRIES {
            let port = free_port();
            // a port the system gave and no one listens on: the outside services never answer
            let closed = format!("http://127.0.0.1:{}", free_port());
            let mut child = Command::new(env!("CARGO_BIN_EXE_home-link"))
                // nothing of the machine's own: no matterjs-server, no meter, no SESAME
                .env_clear()
                .env("PORT", port.to_string())
                .env("DATABASE_PATH", &db)
                .env("LOG_LEVEL", "off")
                .env("GSI_URL", format!("{closed}/gsi"))
                .env("OPEN_METEO_URL", format!("{closed}/meteo"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("home-link starts");
            let url = format!("http://127.0.0.1:{port}");
            let deadline = Instant::now() + Duration::from_secs(10);
            let up = loop {
                if http
                    .get(format!("{url}/healthz"))
                    .send()
                    .await
                    .is_ok_and(|answer| answer.status().is_success())
                {
                    break true;
                }
                if child.try_wait().ok().flatten().is_some() || Instant::now() > deadline {
                    break false;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            if up {
                return Self {
                    child,
                    url,
                    db,
                    http,
                    _scratch: scratch,
                };
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        panic!("home-link did not start");
    }

    /// Sends `body` (none when null) with `method` to `path` of home-link's API and answers
    /// with the JSON it sends back.
    async fn call(&self, method: reqwest::Method, path: &str, body: &Value) -> Value {
        let mut request = self
            .http
            .request(method.clone(), format!("{}{path}", self.url));
        if !body.is_null() {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let answer = request
            .send()
            .await
            .unwrap_or_else(|error| panic!("{method} {path}: {error}"));
        let status = answer.status();
        let bytes = answer.bytes().await.unwrap_or_default();
        assert!(
            status.is_success(),
            "{method} {path}: {status} {}",
            String::from_utf8_lossy(&bytes)
        );
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What every test is given.
struct Ctx {
    page: Page,
    home: Home,
    /// The label "作業", fixed and scheduled, which the first two devices are under.
    label: i64,
    /// The first device of the ledger.
    device: i64,
    /// The scripts a test had the page run first, stopped after it.
    scripts: Mutex<Vec<String>>,
}

impl Ctx {
    /// Opens `path` of home-link.
    async fn open(&self, path: &str) {
        self.page.open(&format!("{}{path}", self.home.url)).await;
    }

    /// Has every document from now on answer the requests `stubs` names (`METHOD /path`) with
    /// its JSON, and pass the others to home-link.
    async fn stub(&self, stubs: &Value) {
        let script = format!(
            "(() => {{ const stubs = {stubs}; const real = window.fetch.bind(window); \
             window.fetch = async (input, init = {{}}) => {{ \
               const url = new URL(String(input), location.href); \
               const key = `${{init.method ?? 'GET'}} ${{decodeURIComponent(url.pathname)}}`; \
               return key in stubs \
                 ? new Response(JSON.stringify(stubs[key]), {{ headers: {{ 'content-type': 'application/json' }} }}) \
                 : real(input, init); }}; }})();"
        );
        let id = self.page.first(&script).await;
        self.scripts.lock().expect("unpoisoned").push(id);
    }

    /// Waits until the screen has read what it shows.
    async fn read(&self, path: &str) {
        self.page
            .until(
                &format!("{path} has read what it shows"),
                "return /success|empty/.test(document.querySelector('section.content')?.dataset.state ?? '');",
                Value::Null,
            )
            .await;
    }

    /// Clicks what `selector` finds, as a script does.
    async fn click(&self, selector: &str) {
        self.page
            .until(
                &format!("{selector} is there to click"),
                "const el = document.querySelector(arguments[0]); if (!el) return false; el.click(); return true;",
                json!(selector),
            )
            .await;
    }

    /// How far the page runs off to the side, its background, and whether anything in it
    /// scrolls on its own instead of the page.
    async fn layout(&self) -> Value {
        self.page
            .js(
                "return { \
                   overflow: document.documentElement.scrollWidth - document.documentElement.clientWidth, \
                   background: getComputedStyle(document.body).backgroundColor, \
                   innerScroll: [...document.querySelectorAll('main, main *')].some((el) => \
                     el.scrollHeight > el.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(el).overflowY)), \
                 };",
                Value::Null,
            )
            .await
    }

    /// How far the page runs off to the side.
    async fn overflow(&self) -> i64 {
        self.layout().await["overflow"]
            .as_i64()
            .expect("a width is a number")
    }

    /// The box of what `selector` finds: `[x, y, width, height]`.
    async fn bounds(&self, selector: &str) -> [f64; 4] {
        let found = self
            .page
            .until(
                &format!("{selector} is shown"),
                "const el = document.querySelector(arguments[0]); if (!el) return null; \
                 const box = el.getBoundingClientRect(); \
                 return box.width > 0 ? [box.x, box.y, box.width, box.height] : null;",
                json!(selector),
            )
            .await;
        let n = |i: usize| found[i].as_f64().expect("a box is numbers");
        [n(0), n(1), n(2), n(3)]
    }
}

/// The looks every screen is measured in: the theme, the window, and the background of the
/// body the theme gives.
const LOOKS: [(&str, u32, u32, &str); 4] = [
    ("light", 1280, 800, "rgb(250, 246, 239)"),
    ("dark", 1280, 800, "rgb(25, 25, 25)"),
    ("light", 390, 844, "rgb(250, 246, 239)"),
    ("dark", 390, 844, "rgb(25, 25, 25)"),
];

const ROUTINE: &str = "e2e-routine";
const PHONES: [(&str, &str); 2] = [
    ("3f2b6c1e-9a4d-4c55-8d1f-0e6a7b9c2d10", "Pixel 9"),
    (
        "7a1c0d9e-2b3f-4a5b-9c8d-1e2f3a4b5c6d",
        "家族が共用で使っている古いタブレット（リビングの棚に置いてあるもの）",
    ),
];
const HTTP_STEP: &str = "http: POST http://127.0.0.1:1/a/very/long/path/that/must/wrap/on/a/phone/screen/without/running/off/to/the/side";
const THREAD_DOWN: &str = "Threadの機器が応答していません（Threadの基地を確かめてください）";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
}

/// The home the screens show: six devices, three labels, four routines, two phones and the
/// home's pin.
async fn furnish(home: &Home) -> (i64, i64) {
    let post = reqwest::Method::POST;
    let label = home
        .call(
            post.clone(),
            "/api/labels",
            &json!({"name": "作業", "override_level": 254, "override_kelvin": 5000,
                "schedule": [{"time": "07:00", "on": true}, {"time": "23:30", "on": false}]}),
        )
        .await["id"]
        .as_i64()
        .expect("a label has an id");
    let ambient = home
        .call(
            post.clone(),
            "/api/labels",
            &json!({"name": "ambient", "auto": true}),
        )
        .await["id"]
        .clone();
    let routine = home
        .call(post, "/api/labels", &json!({"name": ROUTINE}))
        .await["id"]
        .clone();
    {
        // devices are recorded by connecting them, which the app does
        let db = rusqlite::Connection::open(&home.db).expect("the ledger opens");
        for (name, vendor, serial, qr) in [
            ("居間の照明", Some("Tapo"), Some("E2E-LIGHT-1"), None),
            ("", Some("Tapo"), Some("E2E-PLUG-1"), None),
            ("押入れ1", None, None, Some("MT:Y.K9042C00KA0648G00")),
            (
                "寝室の天井の照明（窓側）と書斎の机の上のスタンドライトをまとめて呼ぶための長い名前",
                Some("Tapo"),
                Some("E2E-GONE"),
                None,
            ),
            ("エアコン", Some("Airco"), Some("E2E-AIRCON-1"), None),
            ("寝室・温湿度計", Some("Aqara"), Some("E2E-SENSOR-1"), None),
        ] {
            db.execute(
                "INSERT INTO devices (name, vendor, serial_number, qr_payload) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![name, vendor, serial, qr],
            )
            .expect("a device is recorded");
        }
    }
    let devices = home
        .call(reqwest::Method::GET, "/api/devices", &Value::Null)
        .await;
    let devices = devices.as_array().expect("the devices are a list");
    for device in &devices[..2] {
        home.call(
            reqwest::Method::PATCH,
            &format!("/api/devices/{}", device["id"]),
            &json!({"label_ids": [label, ambient, routine]}),
        )
        .await;
    }
    let yaml = format!(
        "# 操作ごとに、上から順に実行するステップを書く。\nおやすみ:\n  - off: {ROUTINE}\n  - door: lock\n\
         おはよう:\n  - on: {ROUTINE}\n外出:\n  - off: {ROUTINE}\n  - door: lock\n  - on: 無いlabel\n\
         帰宅:\n  - on: {ROUTINE}\n  - {HTTP_STEP}\n  - near: 100\n    wait: 30\n  - door: unlock\n"
    );
    home.call(reqwest::Method::PUT, "/api/scenes", &json!({"yaml": yaml}))
        .await;
    for (id, name) in PHONES {
        home.call(
            reqwest::Method::PUT,
            &format!("/api/phones/{id}"),
            &json!({"name": name}),
        )
        .await;
    }
    home.call(
        reqwest::Method::PUT,
        "/api/location",
        &json!({"pin": {"latitude": 34.25, "longitude": 132.5}}),
    )
    .await;
    (
        label,
        devices[0]["id"].as_i64().expect("a device has an id"),
    )
}

/// What home-link answers with real devices: a `SwitchBot` meter, two air conditioners and the
/// outside, lights down and one that left the fabric, what asks for the user, three PCs, and
/// how the routines went.
fn with_devices(label: i64) -> Value {
    let now = now();
    let minutes = |n: i64| now - n * 60;
    let down = json!({"node_id": 6, "endpoint": 1, "name": "廊下の天井の照明（階段の上の吹き抜けの側）",
        "network": "wifi", "started_at": minutes(25), "ended_at": null, "found_by": "unavailable",
        "reconnects": 3, "reconnect_result": "no_answer", "reachable": false});
    let left = json!({"node_id": 52, "endpoint": 1, "name": "物置き1", "network": "thread",
        "started_at": minutes(5 * 1440), "ended_at": null, "found_by": "left_fabric", "reconnects": 0,
        "reconnect_result": null, "reachable": null, "device_id": 19, "left_at": minutes(5 * 1440),
        "left_by": "event",
        "rejoin": {"attempts": 4, "attempted_at": minutes(3), "reason": "no_advert",
            "reason_since": minutes(2), "advert_at": minutes(10), "rejoined": false},
        "leaves": {"count": 2, "previous_at": minutes(9 * 1440)}});
    let server = json!({"node_id": null, "endpoint": null, "name": "matterjs-server", "network": null,
        "started_at": minutes(8), "ended_at": null, "found_by": "unreachable", "reconnects": 0,
        "reconnect_result": null, "reachable": null});
    let light = json!({"node_id": 5, "endpoint": 1, "device_id": 3, "name": "玄関", "network": "wifi",
        "connected": true, "since": minutes(90), "last_answer": minutes(2), "outage": null});
    let unit = json!({"id": 5, "name": "エアコン", "state": "connected", "on": true, "mode": "cool",
        "setpoint": 25, "sensor": {"id": 6, "name": "寝室・温湿度計"},
        "reading": {"temperature": 26.7, "humidity": 57},
        "reason": "室温が26.7℃で26℃を超えたため、冷房25℃にする"});
    let ok = |step: String| json!({"step": step, "result": "ok"});
    json!({
        "GET /api/meter": {"address": "AA:BB:CC:DD:EE:01", "name": null,
            "labels": [{"id": label, "name": "作業"}], "on": true,
            "reading": {"temperature": 26.7, "humidity": 57, "co2": 717, "battery": 74},
            "received_at": now - 3, "fresh_for": 300,
            "adapter": {"present": true, "powered": true, "discovering": true},
            "powered_on_at": null, "power_cycled_at": null},
        "GET /api/aircon": {
            "aircons": [unit, {"id": 999, "name": "リビングのエアコン", "state": "connected", "on": false,
                "mode": "cool", "setpoint": 25, "sensor": {"id": null, "name": "リビング"},
                "reading": {"temperature": 24.1, "humidity": 61},
                "reason": "室温が24.1℃で22〜26℃のため、変えない"}],
            "outdoor": {"temperature": 30.2, "at": "2026-10-24T16:15:00+09:00",
                "fetched_at": "2026-10-24T16:20:03+09:00", "fresh": true},
            "location": {"name": "東京都新宿区西新宿二丁目８番", "latitude": 35.689_629,
                "longitude": 139.691_772, "registered": true}},
        "GET /api/lights/watch": {"checked_at": now, "server_reachable": false,
            "server_since": minutes(8), "server_outage": server, "wifi_all_down": false,
            "lights": [light,
                {"node_id": 6, "endpoint": 1, "device_id": null, "name": down["name"], "network": "wifi",
                 "connected": false, "since": minutes(25), "last_answer": minutes(27), "outage": down},
                {"node_id": 7, "endpoint": 1, "device_id": 7, "name": "寝室", "network": "thread",
                 "connected": true, "since": minutes(600), "last_answer": minutes(2), "outage": null},
                {"node_id": null, "endpoint": null, "device_id": 19, "name": "物置き1", "network": null,
                 "connected": false, "state": "left_fabric", "since": minutes(5 * 1440),
                 "last_answer": null, "outage": left}]},
        "GET /api/lights/outages": {"outages": [
            {"node_id": 5, "endpoint": 1, "name": "玄関", "network": "wifi", "started_at": minutes(300),
             "ended_at": minutes(293), "found_by": "unavailable", "reconnects": 1,
             "reconnect_result": "accepted", "reachable": false},
            left, down]},
        "GET /api/attention": {"items": [
            {"kind": "server_down", "since": minutes(8)},
            {"kind": "power_cycle", "name": "廊下の天井の照明（階段の上の吹き抜けの側）",
             "device_id": 2, "since": minutes(40)},
            {"kind": "left_fabric", "name": "物置き1", "device_id": 19, "left_at": minutes(5 * 1440),
             "left_by": "event", "reason": "wrong_code"},
            {"kind": "meter_lost", "since": minutes(20), "trouble": "no_adapter", "battery": 55}]},
        "GET /api/pcs": [
            {"name": "ゲーム用のデスクトップPC（リビングのテレビの横の棚の中）", "sleeping": true},
            {"name": "ノート", "sleeping": null},
            {"name": "書斎", "sleeping": false}],
        "POST /api/scenes/おはよう/run": {"scene": "おはよう", "status": "done", "failed": 0,
            "steps": [ok(format!("on: {ROUTINE}"))]},
        "POST /api/scenes/外出/run": {"scene": "外出", "status": "done", "failed": 2,
            "steps": [ok(format!("off: {ROUTINE}")),
                {"step": "door: lock", "result": "failed", "message": "SESAMEが設定されていません"},
                {"step": "on: 無いlabel", "result": "failed", "message": "labelの「無いlabel」がありません"}]},
        "POST /api/scenes/帰宅/run": {"scene": "帰宅", "status": "waiting", "failed": 1,
            "steps": [ok(format!("on: {ROUTINE}")),
                {"step": HTTP_STEP, "result": "failed", "message": "接続できませんでした"}],
            "next": 2, "wait": 30, "reason": "no_phone"},
    })
}

async fn every_screen(ctx: Arc<Ctx>) {
    ctx.stub(&with_devices(ctx.label)).await;
    let screens = [
        "/".to_owned(),
        format!("/devices/{}", ctx.device),
        "/labels".to_owned(),
        format!("/labels/{}", ctx.label),
        "/meter".to_owned(),
        "/aircon".to_owned(),
        "/lights".to_owned(),
        "/pcs".to_owned(),
        "/scenes".to_owned(),
    ];
    for (theme, width, height, background) in LOOKS {
        ctx.page.look(theme, width, height).await;
        let expected = json!({"overflow": 0, "background": background, "innerScroll": false});
        for path in &screens {
            ctx.open(path).await;
            ctx.read(path).await;
            assert_eq!(
                ctx.layout().await,
                expected,
                "{path} at {width}px in the {theme}"
            );
        }
        // What asks for the user on the home: each link is tall enough for a finger.
        ctx.open("/").await;
        let heights = ctx
            .page
            .until(
                "the home lists what asks for the user",
                "const links = [...document.querySelectorAll('[aria-label=\"要対応\"] a')]; \
                 return links.length === 4 && links.map((a) => a.getBoundingClientRect().height);",
                Value::Null,
            )
            .await;
        for height in heights.as_array().expect("heights are a list") {
            assert!(
                height.as_f64().is_some_and(|h| h >= 44.0),
                "a link of what asks for the user is {height}px tall at {width}px in the {theme}"
            );
        }
        // The air conditioner's settings, the light watch's history and the routines' runs and
        // definition, opened.
        ctx.open("/aircon").await;
        ctx.click("details.settings summary").await;
        ctx.page
            .until(
                "the settings are open",
                "return document.querySelector('details.settings').open;",
                Value::Null,
            )
            .await;
        assert_eq!(
            ctx.layout().await,
            expected,
            "/aircon with its settings at {width}px in the {theme}"
        );
        ctx.open("/lights").await;
        ctx.click("details.history summary").await;
        ctx.page
            .until(
                "the history is open",
                "return document.querySelector('details.history').open;",
                Value::Null,
            )
            .await;
        assert_eq!(
            ctx.overflow().await,
            0,
            "/lights with its history at {width}px in the {theme}"
        );
        ctx.open("/scenes").await;
        ctx.read("/scenes").await;
        for name in ["おはよう", "外出", "帰宅"] {
            ctx.click(&format!("button[aria-label=\"{name}を実行\"]"))
                .await;
        }
        ctx.page
            .until(
                "each run is told",
                "return document.querySelectorAll('li.card .told').length === 3;",
                Value::Null,
            )
            .await;
        ctx.click("details.definition summary").await;
        assert_eq!(
            ctx.layout().await,
            expected,
            "/scenes run and opened at {width}px in the {theme}"
        );
    }
}

async fn header_at_a_phone(ctx: Arc<Ctx>) {
    ctx.page.look("light", 390, 844).await;
    ctx.open("/labels").await;
    ctx.read("/labels").await;
    let held = ctx
        .page
        .js(
            "return document.querySelectorAll('header a, header button').length;",
            Value::Null,
        )
        .await;
    assert_eq!(
        held,
        json!(2),
        "the header holds the name and the menu alone"
    );
    let [x, y, w, h] = ctx.bounds("button[aria-label=\"メニュー\"]").await;
    ctx.page.click_at(x + w / 2.0, y + h / 2.0).await;
    let menu = ctx.bounds("nav.menu").await;
    assert!(
        ((menu[0] + menu[2]) - (x + w)).abs() <= 1.0,
        "the menu ends at {} and its button at {}",
        menu[0] + menu[2],
        x + w
    );
    assert_eq!(ctx.overflow().await, 0, "the menu stays on screen");
}

async fn confirmation_at_a_phone(ctx: Arc<Ctx>) {
    for theme in ["light", "dark"] {
        ctx.page.look(theme, 390, 844).await;
        ctx.open(&format!("/labels/{}", ctx.label)).await;
        ctx.read("/labels").await;
        // An input error under the field.
        ctx.page
            .js("const field = document.querySelector('#label-percent'); field.focus(); field.select();", Value::Null)
            .await;
        ctx.page.type_text("101").await;
        ctx.page.key("Tab").await;
        ctx.page
            .until(
                "the error is shown",
                "return document.querySelector('[role=alert]');",
                Value::Null,
            )
            .await;
        assert_eq!(ctx.overflow().await, 0, "the input error in the {theme}");
        // The confirmation, within the window.
        let delete = ctx
            .page
            .until(
                "the delete button is shown",
                "const button = [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === '削除'); \
                 if (!button) return null; button.scrollIntoView({ block: 'center' }); \
                 const box = button.getBoundingClientRect(); \
                 return [box.x + box.width / 2, box.y + box.height / 2];",
                Value::Null,
            )
            .await;
        let at = |i: usize| delete[i].as_f64().expect("a point is numbers");
        ctx.page.click_at(at(0), at(1)).await;
        let [x, _, width, _] = ctx.bounds("[role=dialog]").await;
        assert!(
            x >= 0.0 && x + width <= 390.0,
            "the confirmation lies from {x} to {} in the {theme}",
            x + width
        );
        ctx.page.key("Escape").await;
        ctx.page
            .until(
                "the confirmation is gone",
                "return !document.querySelector('[role=dialog]');",
                Value::Null,
            )
            .await;
    }
}

async fn devices_not_answering(ctx: Arc<Ctx>) {
    // The ledger while its first two devices, on Thread, do not answer, and a label's switch
    // that names them.
    let label = ctx.label;
    let script = format!(
        "(() => {{ const real = window.fetch.bind(window); \
         window.fetch = async (input, init = {{}}) => {{ \
           const url = new URL(String(input), location.href); const method = init.method ?? 'GET'; \
           if (method === 'GET' && url.pathname === '/api/devices') {{ \
             const devices = await (await real(input, init)).json(); \
             devices.forEach((d, i) => {{ d.network = i < 2 ? 'thread' : 'wifi'; if (i < 2) d.state = 'no_response'; }}); \
             return new Response(JSON.stringify(devices), {{ headers: {{ 'content-type': 'application/json' }} }}); }} \
           if (method === 'POST' && url.pathname === '/api/labels/{label}/toggle') \
             return new Response(JSON.stringify({{ on: true, switched: 1, no_response: 2, failed: 0, missing: 0, \
               devices: [{{ name: '書斎', result: 'switched', network: 'wifi' }}, \
                 {{ name: '居間の照明', result: 'no_response', network: 'thread' }}, \
                 {{ name: '寝室の天井の照明（窓側）', result: 'no_response', network: 'thread' }}] }}), \
               {{ headers: {{ 'content-type': 'application/json' }} }}); \
           return real(input, init); }}; }})();"
    );
    let id = ctx.page.first(&script).await;
    ctx.scripts.lock().expect("unpoisoned").push(id);
    for (theme, width, height, _) in LOOKS {
        ctx.page.look(theme, width, height).await;
        ctx.open("/").await;
        ctx.page
            .until(
                "the Thread base is pointed to",
                "return document.querySelector('[role=alert]')?.textContent.trim() === arguments[0];",
                json!(THREAD_DOWN),
            )
            .await;
        assert_eq!(
            ctx.overflow().await,
            0,
            "the home at {width}px in the {theme}"
        );
        ctx.open(&format!("/labels/{label}")).await;
        ctx.read("/labels").await;
        ctx.click("button[aria-label=\"入り切り\"]").await;
        ctx.page
            .until(
                "the devices that did not answer are named",
                "return document.querySelector('[role=alert]')?.textContent.includes('応答なし 2');",
                Value::Null,
            )
            .await;
        assert_eq!(
            ctx.overflow().await,
            0,
            "the label at {width}px in the {theme}"
        );
    }
}

async fn nothing_set_up(ctx: Arc<Ctx>) {
    let home = &ctx.home;
    home.call(reqwest::Method::PUT, "/api/scenes", &json!({"yaml": ""}))
        .await;
    for (id, _) in PHONES {
        home.call(
            reqwest::Method::DELETE,
            &format!("/api/phones/{id}"),
            &Value::Null,
        )
        .await;
    }
    home.call(reqwest::Method::PUT, "/api/location", &json!({"pin": null}))
        .await;
    for (width, height) in [(1280, 800), (390, 844)] {
        ctx.page.look("light", width, height).await;
        ctx.open("/scenes").await;
        ctx.read("/scenes").await;
        ctx.page
            .until(
                "no routine is told",
                "return document.body.textContent.includes('操作がありません');",
                Value::Null,
            )
            .await;
        assert_eq!(ctx.overflow().await, 0, "/scenes with nothing at {width}px");
        ctx.open("/pcs").await;
        ctx.read("/pcs").await;
        ctx.page
            .until(
                "no PC is told",
                "return document.body.textContent.includes('つながっているPCはありません');",
                Value::Null,
            )
            .await;
        assert_eq!(ctx.overflow().await, 0, "/pcs with none at {width}px");
    }
}

type Test = fn(Arc<Ctx>) -> Pin<Box<dyn Future<Output = ()> + Send>>;

macro_rules! test {
    ($title:expr, $run:ident) => {
        ($title, (|ctx| Box::pin($run(ctx))) as Test)
    };
}

/// In the order they run; the last ones change what home-link holds.
const TESTS: &[(&str, Test)] = &[
    test!(
        "every screen keeps to the width, scrolls as a page and takes the theme",
        every_screen
    ),
    test!(
        "at 390px the header holds only the name and the menu, and the menu stays on screen",
        header_at_a_phone
    ),
    test!(
        "the delete confirmation and an input error stay on a phone screen",
        confirmation_at_a_phone
    ),
    test!(
        "devices not answering keep to the width at each look",
        devices_not_answering
    ),
    test!(
        "with nothing set up the routines and the PCs keep to the width",
        nothing_set_up
    ),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let chosen: Vec<_> = TESTS
        .iter()
        .filter(|(title, _)| words.is_empty() || words.iter().any(|w| title.contains(w.as_str())))
        .collect();
    if args.iter().any(|a| a == "--list") {
        for (title, _) in &chosen {
            println!("{title}: test");
        }
        return ExitCode::SUCCESS;
    }
    if chosen.is_empty() {
        println!("running 0 tests");
        return ExitCode::SUCCESS;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let started = Instant::now();
        let home = Home::start().await;
        let (label, device) = furnish(&home).await;
        let page = Page::start().await;
        let ctx = Arc::new(Ctx {
            page,
            home,
            label,
            device,
            scripts: Mutex::new(Vec::new()),
        });
        let total = chosen.len();
        println!("running {total} tests");
        let mut failed = Vec::new();
        for (title, run) in chosen {
            let began = Instant::now();
            let outcome = tokio::spawn(run(Arc::clone(&ctx))).await;
            let took = began.elapsed().as_secs_f64();
            if outcome.is_ok() {
                println!("test {title} ... ok ({took:.2}s)");
            } else {
                println!("test {title} ... FAILED ({took:.2}s)");
                failed.push(*title);
            }
            let scripts: Vec<String> = ctx.scripts.lock().expect("unpoisoned").drain(..).collect();
            for id in scripts {
                ctx.page.forget(&id).await;
            }
        }
        let ctx = Arc::into_inner(ctx).expect("the tests have ended");
        ctx.page.close().await;
        drop(ctx.home);
        let took = started.elapsed().as_secs_f64();
        if failed.is_empty() {
            println!("\ntest result: ok. {total} passed; finished in {took:.2}s");
            ExitCode::SUCCESS
        } else {
            println!("\nfailures:");
            for title in &failed {
                println!("    {title}");
            }
            println!(
                "\ntest result: FAILED. {} failed; finished in {took:.2}s",
                failed.len()
            );
            ExitCode::FAILURE
        }
    })
}
