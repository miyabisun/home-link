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

async function watching(
  page: Page,
  { notify, serverDown }: { notify: boolean; serverDown: boolean },
) {
  await page.route("/api/lights/watch", (route) =>
    route.fulfill({
      json: {
        checked_at: minutes(0),
        server_reachable: !serverDown,
        server_since: serverDown ? server.started_at : minutes(600),
        server_outage: serverDown ? server : null,
        wifi_all_down: false,
        notify,
        lights: [light, down, thread],
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
          down.outage,
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
      await watching(page, { notify: true, serverDown: true });
      await page.goto("/lights");

      const cards = page.locator(".card");
      await expect(cards).toHaveCount(3);
      await expect(cards.nth(0)).toContainText("玄関");
      await expect(cards.nth(0)).toContainText("から・応答");
      await expect(cards.nth(0).locator(".badge")).toHaveCount(0);
      await expect(cards.nth(1).locator(".badge")).toHaveText("切断");
      await expect(cards.nth(1)).toContainText("つなぎ直し3回・網で届かない");
      await expect(page.getByRole("alert")).toContainText(
        "matterjs-serverに届きません",
      );
      await noSideScroll(page);
      await page.screenshot({
        path: `test-results/screens/lights-${size.width}-${scheme}.png`,
        fullPage: true,
      });

      const history = page.locator("details.history");
      await expect(history.locator("summary")).toHaveText("14日の履歴（2件）");
      await history.locator("summary").click();
      // Newest first; the one still going says so.
      await expect(history.locator("li").nth(0)).toContainText("続いている");
      await expect(history.locator("li").nth(1)).toContainText("（7分）");
      await noSideScroll(page);
      await page.screenshot({
        path: `test-results/screens/lights-history-${size.width}-${scheme}.png`,
        fullPage: true,
      });
    });
  }
}

test("a test message is sent from the page, and refused without a destination", async ({
  page,
}) => {
  await watching(page, { notify: true, serverDown: false });
  let posted = 0;
  await page.route("/api/lights/watch/test", (route) => {
    posted += 1;
    return route.fulfill({ json: { sent: true } });
  });
  await page.goto("/lights");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.getByRole("button", { name: "試しに知らせる" }).click();
  await expect(page.getByRole("status")).toHaveText("送りました");
  expect(posted).toBe(1);

  // The server under test has no destination.
  await page.unroute("/api/lights/watch");
  await page.unroute("/api/lights/outages");
  await page.goto("/lights");
  await expect(page.getByRole("status")).toHaveText(
    "知らせの送り先が未設定です",
  );
  await expect(
    page.getByRole("button", { name: "試しに知らせる" }),
  ).toBeDisabled();
});

test("the menu leads to the light watch", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("link", { name: "照明の接続" }).click();
  await expect(page.locator(".sub-header h1")).toHaveText("照明の接続");
});
