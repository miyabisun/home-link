import { expect, test, type Page } from "@playwright/test";

// The light watch's page at a desk and a phone width in both themes, with a light down, a
// long name and matterjs-server out of reach. Images go to test-results/screens.
const widths = [
  { width: 1280, height: 800 },
  { width: 390, height: 844 },
];
const schemes = ["light", "dark"] as const;

const now = Math.floor(Date.now() / 1000);
const minutes = (n: number) => now - n * 60;
/** The day of `unix` in Tokyo, as the admin screen writes it (`10/17`). */
const day = (unix: number) =>
  new Intl.DateTimeFormat("ja-JP", {
    timeZone: "Asia/Tokyo",
    month: "numeric",
    day: "numeric",
  }).format(unix * 1000);

const light = {
  node_id: 5,
  endpoint: 1,
  name: "玄関",
  network: "wifi",
  connected: true,
  since: minutes(90),
  last_answer: minutes(2),
  outage: null,
};
const down = {
  ...light,
  node_id: 6,
  name: "廊下の天井の照明（階段の上の吹き抜けの側）",
  connected: false,
  since: minutes(25),
  last_answer: minutes(27),
  outage: {
    node_id: 6,
    endpoint: 1,
    name: "廊下の天井の照明（階段の上の吹き抜けの側）",
    network: "wifi",
    started_at: minutes(25),
    ended_at: null,
    found_by: "unavailable",
    reconnects: 3,
    reconnect_result: "no_answer",
    reachable: false,
  },
};
const thread = {
  ...light,
  node_id: 7,
  name: "寝室",
  network: "thread",
  since: minutes(600),
};
// A light that left the fabric five days ago, as matterjs-server sent.
const left = {
  node_id: null,
  endpoint: null,
  device_id: 19,
  name: "物置き1",
  network: null,
  connected: false,
  state: "left_fabric",
  since: minutes(5 * 1440),
  last_answer: null,
  outage: {
    ...down.outage,
    node_id: 52,
    name: "物置き1",
    network: "thread",
    started_at: minutes(5 * 1440),
    found_by: "left_fabric",
    reconnects: 0,
    reconnect_result: null,
    reachable: null,
    device_id: 19,
    left_at: minutes(5 * 1440),
    left_by: "event",
    rejoin: {
      attempts: 4,
      attempted_at: minutes(3),
      reason: "no_advert",
      reason_since: minutes(2),
      advert_at: minutes(10),
      rejoined: false,
    },
    leaves: { count: 2, previous_at: minutes(9 * 1440) },
  },
};
const server = {
  ...down.outage,
  node_id: null,
  endpoint: null,
  name: "matterjs-server",
  network: null,
  started_at: minutes(8),
  found_by: "unreachable",
  reconnects: 0,
  reconnect_result: null,
  reachable: null,
};

async function watching(page: Page, { serverDown }: { serverDown: boolean }) {
  await page.route("/api/lights/watch", (route) =>
    route.fulfill({
      json: {
        checked_at: minutes(0),
        server_reachable: !serverDown,
        server_since: serverDown ? server.started_at : minutes(600),
        server_outage: serverDown ? server : null,
        wifi_all_down: false,
        lights: [light, down, thread, left],
      },
    }),
  );
  await page.route("/api/lights/outages", (route) =>
    route.fulfill({
      json: {
        outages: [
          {
            ...down.outage,
            node_id: 5,
            name: "玄関",
            started_at: minutes(300),
            ended_at: minutes(293),
            reconnects: 1,
            reconnect_result: "accepted",
          },
          left.outage,
          down.outage,
        ],
      },
    }),
  );
}

// What the server lists as asking for the user: matterjs-server has its own banner here.
async function asking(page: Page) {
  await page.route("/api/attention", (route) =>
    route.fulfill({
      json: {
        items: [
          { kind: "server_down", since: server.started_at },
          {
            kind: "power_cycle",
            name: down.name,
            device_id: null,
            since: down.outage.started_at,
          },
          {
            kind: "left_fabric",
            name: "物置き1",
            device_id: 19,
            left_at: left.outage.left_at,
            left_by: "event",
            reason: "no_advert",
          },
          { kind: "meter_battery", battery: 19 },
        ],
      },
    }),
  );
}

async function noSideScroll(page: Page) {
  expect(
    await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    ),
  ).toBe(0);
}

for (const size of widths) {
  for (const scheme of schemes) {
    test(`the light watch at ${size.width}px in ${scheme}`, async ({
      page,
    }) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: scheme });
      await watching(page, { serverDown: true });
      await asking(page);
      await page.goto("/lights");

      const attention = page
        .getByRole("list", { name: "要対応" })
        .getByRole("link");
      await expect(attention).toHaveCount(2);
      await expect(attention.nth(0)).toContainText(
        "からつながっていません。壁のスイッチで電源を入れ直してください。",
      );
      await expect(attention.nth(1)).toContainText("物置き1の登録が");

      const cards = page.locator(".card");
      await expect(cards).toHaveCount(4);
      await expect(cards.nth(0)).toContainText("玄関");
      await expect(cards.nth(0)).toContainText("から・応答");
      await expect(cards.nth(0).locator(".badge")).toHaveCount(0);
      await expect(cards.nth(1).locator(".badge")).toHaveText("切断");
      await expect(cards.nth(1)).toContainText("つなぎ直し3回・網で届かない");
      await expect(cards.nth(3).locator(".badge")).toHaveText("登録が外れた");
      await expect(cards.nth(3)).toContainText(
        `から・自動で登録し直し中・登録を待っていない・4回試した・14日で2回目（前回 ${day(minutes(9 * 1440))} `,
      );
      await expect(page.getByRole("alert")).toContainText(
        "matterjs-serverに届きません",
      );
      await noSideScroll(page);
      await page.screenshot({
        path: `test-results/screens/lights-${size.width}-${scheme}.png`,
        fullPage: true,
      });

      const history = page.locator("details.history");
      await expect(history.locator("summary")).toHaveText("14日の履歴（3件）");
      await history.locator("summary").click();
      // Newest first; the one still going says so.
      await expect(history.locator("li").nth(0)).toContainText("続いている");
      await expect(history.locator("li").nth(1)).toContainText("登録が外れた");
      await expect(history.locator("li").nth(2)).toContainText("（7分）");
      await noSideScroll(page);
      await page.screenshot({
        path: `test-results/screens/lights-history-${size.width}-${scheme}.png`,
        fullPage: true,
      });
    });
  }
}

test("the menu leads to the light watch", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("link", { name: "照明の接続" }).click();
  await expect(page.locator(".sub-header h1")).toHaveText("照明の接続");
});
