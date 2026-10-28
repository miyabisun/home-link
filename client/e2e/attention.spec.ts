import { expect, test, type Page } from "@playwright/test";

// What asks for the user, at the top of the home and of its own page, at a desk and a phone
// width in both themes. Images go to test-results/screens.
const widths = [
  { width: 1280, height: 800 },
  { width: 390, height: 844 },
];
const schemes = ["light", "dark"] as const;

const now = Math.floor(Date.now() / 1000);
const minutes = (n: number) => now - n * 60;

const items = [
  { kind: "server_down", since: minutes(8) },
  {
    kind: "power_cycle",
    name: "廊下の天井の照明（階段の上の吹き抜けの側）",
    device_id: 2,
    since: minutes(40),
  },
  {
    kind: "left_fabric",
    name: "物置き1",
    device_id: 19,
    left_at: minutes(5 * 1440),
    left_by: "event",
    reason: "wrong_code",
  },
  {
    kind: "meter_lost",
    since: minutes(20),
    trouble: "no_adapter",
    battery: 55,
  },
];

async function asking(page: Page, listed: object[]) {
  await page.route("/api/attention", (route) =>
    route.fulfill({ json: { items: listed } }),
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
    test(`what asks for the user on the home at ${size.width}px in ${scheme}`, async ({
      page,
    }) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: scheme });
      await asking(page, items);
      await page.goto("/");

      const links = page
        .getByRole("list", { name: "要対応" })
        .getByRole("link");
      await expect(links).toHaveCount(4);
      await expect(links.nth(0)).toContainText(
        "からつながりません。照明を操作できません。",
      );
      await expect(links.nth(1)).toContainText(
        "壁のスイッチで電源を入れ直してください。",
      );
      await expect(links.nth(2)).toContainText(
        "アプリで登録し直してください。",
      );
      await expect(links.nth(3)).toContainText(
        "UB500が外れています。挿し直してください。",
      );
      for (const link of await links.all()) {
        expect((await link.boundingBox())?.height).toBeGreaterThanOrEqual(44);
      }
      await noSideScroll(page);
      await page.screenshot({
        path: `test-results/screens/attention-home-${size.width}-${scheme}.png`,
        fullPage: true,
      });
    });
  }
}

test("nothing asking leaves the home as it was", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("section.content")).not.toHaveAttribute(
    "data-state",
    "loading",
  );
  await expect(page.getByRole("list", { name: "要対応" })).toHaveCount(0);
});

test("each item leads to the page that shows more of it", async ({ page }) => {
  await asking(page, items);
  await page.goto("/");
  const links = page.getByRole("list", { name: "要対応" }).getByRole("link");
  await links.nth(1).click();
  await expect(page.locator(".sub-header h1")).toHaveText("照明の接続");
  await page.goto("/");
  await links.nth(3).click();
  await expect(page.locator(".sub-header h1")).toHaveText("温度計");
});

test("the meter's page shows what of the meter asks for the user", async ({
  page,
}) => {
  await asking(page, items);
  await page.route("/api/meter", (route) =>
    route.fulfill({
      json: {
        address: "AA:BB:CC:DD:EE:01",
        reading: null,
        received_at: minutes(20),
        fresh_for: 300,
        adapter: { present: false, powered: false, discovering: false },
        powered_on_at: null,
        power_cycled_at: null,
      },
    }),
  );
  await page.goto("/meter");
  const links = page.getByRole("list", { name: "要対応" }).getByRole("link");
  await expect(links).toHaveCount(1);
  await expect(links.nth(0)).toContainText("温度計を");
  await noSideScroll(page);
  await page.screenshot({
    path: "test-results/screens/attention-meter-1280-light.png",
    fullPage: true,
  });
});
