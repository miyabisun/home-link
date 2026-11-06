import { expect, test, type Page } from "@playwright/test";

// Every screen at a desk and a phone width, in both themes: nothing runs off to the side,
// the page itself scrolls, and the theme follows the OS. Images go to test-results/screens.
const widths = [
  { width: 1280, height: 800 },
  { width: 390, height: 844 },
];
const schemes = [
  { scheme: "light", background: "rgb(250, 246, 239)" },
  { scheme: "dark", background: "rgb(25, 25, 25)" },
] as const;

let labelId = 0;
let deviceId = 0;

// The E2E home-link has no meter; these stand in for what GET /api/meter returns with one.
const address = "AA:BB:CC:DD:EE:01";
const now = () => Math.floor(Date.now() / 1000);
const adapter = { present: true, powered: true, discovering: true };
const mended = { powered_on_at: null, power_cycled_at: null };
const meters = {
  fresh: () => ({
    address,
    reading: { temperature: 26.7, humidity: 57, co2: 717, battery: 74 },
    received_at: now() - 3,
    fresh_for: 300,
    adapter,
    ...mended,
  }),
  stale: () => ({
    address,
    reading: null,
    received_at: now() - 3600,
    fresh_for: 300,
    adapter,
    ...mended,
  }),
  waiting: () => ({
    address,
    reading: null,
    received_at: null,
    fresh_for: 300,
    adapter,
    ...mended,
  }),
  // Stale while the adapter stays off, after the server powered it on.
  off: () => ({
    address,
    reading: null,
    received_at: now() - 3600,
    fresh_for: 300,
    adapter: { present: true, powered: false, discovering: false },
    powered_on_at: now() - 30,
    power_cycled_at: null,
  }),
};

// The aircon page as it reads with a meter, while the E2E home-link has none.
async function airconWithMeter(page: Page) {
  await page.route("/api/aircon", async (route) => {
    const json = await (await route.fetch()).json();
    json.aircons[0].sensor = { id: 0, name: "寝室・温湿度計" };
    json.aircons[0].reading = { temperature: 26.7, humidity: 57 };
    json.aircons[0].reason = "室温が26.7℃で26℃を超えたため、冷房25℃にする";
    json.aircons.push({
      ...json.aircons[0],
      id: 999,
      name: "リビングのエアコン",
      on: false,
      sensor: { id: null, name: "リビング" },
      reading: { temperature: 24.1, humidity: 61 },
      reason: "室温が24.1℃で20〜26℃のため、変えない",
    });
    json.outdoor = {
      temperature: 30.2,
      at: "2026-10-24T16:15:00+09:00",
      fetched_at: "2026-10-24T16:20:03+09:00",
      fresh: true,
    };
    json.location = {
      name: "東京都新宿区西新宿二丁目８番",
      latitude: 35.689629,
      longitude: 139.691772,
      registered: true,
    };
    await route.fulfill({ json });
  });
}

async function meterAs(page: Page, kind: keyof typeof meters) {
  const kept = {
    name: null as string | null,
    labels: [] as number[],
    on: true,
  };
  await page.unroute("/api/meter");
  await page.route("/api/meter", (route) => {
    if (route.request().method() === "PUT")
      Object.assign(kept, route.request().postDataJSON());
    const labels = kept.labels.map((id) => ({ id, name: String(id) }));
    return route.fulfill({ json: { ...meters[kind](), ...kept, labels } });
  });
}

test.beforeAll(async ({ request }) => {
  const create = async (body: object) => {
    const response = await request.post("/api/labels", { data: body });
    expect(response.ok()).toBe(true);
    return (await response.json()).id as number;
  };
  labelId = await create({
    name: "作業",
    override_level: 254,
    override_kelvin: 5000,
    schedule: [
      { time: "07:00", on: true },
      { time: "23:30", on: false },
    ],
  });
  const ambient = await create({ name: "ambient", auto: true });
  const devices = await (await request.get("/api/devices")).json();
  deviceId = devices[0].id;
  for (const device of devices.slice(0, 2)) {
    await request.patch(`/api/devices/${device.id}`, {
      data: { label_ids: [labelId, ambient] },
    });
  }
});

for (const size of widths) {
  for (const { scheme, background } of schemes) {
    test(`screens at ${size.width}px in ${scheme}`, async ({ page }) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: scheme });
      await meterAs(page, "fresh");
      await airconWithMeter(page);
      const screens = [
        ["devices", "/"],
        ["device", `/devices/${deviceId}`],
        ["labels", "/labels"],
        ["label", `/labels/${labelId}`],
        ["meter", "/meter"],
        ["aircon", "/aircon"],
      ];
      for (const [name, path] of screens) {
        await page.goto(path);
        await expect(page.locator("section.content")).toHaveAttribute(
          "data-state",
          /success|empty/,
        );
        const layout = await page.evaluate(() => ({
          overflow:
            document.documentElement.scrollWidth -
            document.documentElement.clientWidth,
          background: getComputedStyle(document.body).backgroundColor,
          innerScroll: [...document.querySelectorAll("main, main *")].some(
            (el) =>
              el.scrollHeight > el.clientHeight + 1 &&
              /auto|scroll/.test(getComputedStyle(el).overflowY),
          ),
        }));
        expect(layout, `${name}`).toEqual({
          overflow: 0,
          background,
          innerScroll: false,
        });
        await page.screenshot({
          path: `test-results/screens/${name}-${size.width}-${scheme}.png`,
          fullPage: true,
        });
      }
      // The air conditioner's settings, opened.
      await page.goto("/aircon");
      await page.getByText("設定", { exact: true }).click();
      await expect(page.getByLabel("住所")).toBeVisible();
      expect(
        await page.evaluate(
          () =>
            document.documentElement.scrollWidth -
            document.documentElement.clientWidth,
        ),
      ).toBe(0);
      await page.screenshot({
        path: `test-results/screens/aircon-settings-${size.width}-${scheme}.png`,
        fullPage: true,
      });
    });
  }
}

test("at 390px the header holds only the name and the menu, and the menu stays on screen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/labels");
  await expect(page.locator("header").locator("a, button")).toHaveCount(2);
  await page.getByRole("button", { name: "メニュー", exact: true }).click();
  const menu = await page.locator("nav.menu").boundingBox();
  const button = await page
    .getByRole("button", { name: "メニュー", exact: true })
    .boundingBox();
  expect(
    Math.abs(menu!.x + menu!.width - (button!.x + button!.width)),
  ).toBeLessThanOrEqual(1);
  expect(
    await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    ),
  ).toBe(0);
});

test("the delete confirmation and an input error stay on a phone screen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto(`/labels/${labelId}`);
    await page.getByLabel("明るさ（%）").fill("101");
    await page.keyboard.press("Tab");
    await expect(page.getByRole("alert")).toHaveText("明るさは1〜100%");
    await page.screenshot({
      path: `test-results/screens/label-error-390-${scheme}.png`,
    });
    await page.getByRole("button", { name: "削除", exact: true }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    const box = (await dialog.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(390);
    await page.screenshot({
      path: `test-results/screens/confirm-390-${scheme}.png`,
    });
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  }
});

test("the meters show their values, a stopped reception, and having none", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await meterAs(page, "fresh");
    await page.goto("/meter");
    // The ledger's sensor first, linked to its device, then the SwitchBot meter.
    const sensor = page.locator("section.meter").first();
    await expect(sensor.locator("h2")).toHaveText("寝室・温湿度計");
    await expect(sensor.getByRole("link")).toHaveAttribute(
      "href",
      /^\/devices\/\d+$/,
    );
    await expect(sensor.locator(".reading dd")).toHaveText([
      "28.3℃",
      "52%",
      "90%",
    ]);
    const switchbot = page.locator('section[aria-labelledby="switchbot"]');
    const values = switchbot.locator(".reading dd");
    await expect(switchbot.locator("h2")).toHaveText("SwitchBot");
    await expect(values).toHaveText(["26.7℃", "57%", "717ppm", "74%"]);
    await expect(switchbot.locator(".received")).toHaveText(
      /^\d\d:\d\d:\d\d 受信$/,
    );
    await page.screenshot({
      path: `test-results/screens/meter-390-${scheme}.png`,
      fullPage: true,
    });

    // Named, and named back by a blank.
    await switchbot.getByRole("button", { name: "名前を変える" }).click();
    await page.getByLabel("名前").fill("リビング");
    await page.keyboard.press("Tab");
    await expect(page.locator("#switchbot")).toHaveText("リビング");
    await page.getByRole("button", { name: "名前を変える" }).click();
    await expect(page.getByLabel("名前")).toHaveValue("リビング");
    await page.getByLabel("名前").fill("");
    await page.keyboard.press("Tab");
    await expect(page.locator("#switchbot")).toHaveText("SwitchBot");

    // Put under a label and switched off, it says so by its name.
    await switchbot.getByText("自動運転", { exact: true }).click();
    const put = page.waitForRequest(
      (r) => r.url().endsWith("/api/meter") && r.method() === "PUT",
    );
    await switchbot.getByRole("checkbox", { name: "作業" }).check();
    expect((await put).postDataJSON()).toEqual({ labels: [labelId] });
    await expect(
      switchbot.getByRole("checkbox", { name: "作業" }),
    ).toBeChecked();
    const power = switchbot.getByRole("button", { name: "自動運転" });
    await expect(power).toHaveAttribute("aria-pressed", "true");
    await power.click();
    await expect(power).toHaveText("切");
    await expect(switchbot.locator(".badge")).toHaveText("自動運転 切");
    await page.screenshot({
      path: `test-results/screens/meter-switchbot-390-${scheme}.png`,
      fullPage: true,
    });

    await meterAs(page, "stale");
    await page.goto("/meter");
    await expect(switchbot.locator(".state")).toHaveText(
      "受信が途絶えています",
    );
    await expect(values).toHaveCount(0);
    await expect(switchbot.locator(".note")).toHaveText(
      /^最後の受信 \d+\/\d+ \d\d:\d\d$/,
    );
    await page.screenshot({
      path: `test-results/screens/meter-stale-390-${scheme}.png`,
    });

    await meterAs(page, "off");
    await page.goto("/meter");
    await expect(switchbot.locator(".state")).toHaveText(
      "受信が途絶えています",
    );
    await expect(switchbot.locator(".note")).toHaveText([
      /^最後の受信 \d+\/\d+ \d\d:\d\d$/,
      "アダプタの電源が入っていません",
      /^電源の入れ直し \d+\/\d+ \d\d:\d\d$/,
    ]);
    await page.screenshot({
      path: `test-results/screens/meter-adapter-off-390-${scheme}.png`,
    });

    await meterAs(page, "waiting");
    await page.goto("/meter");
    await expect(switchbot.locator(".state")).toHaveText(
      "まだ受信していません",
    );

    // The E2E home-link itself has no SwitchBot meter configured: the sensor alone.
    await page.unroute("/api/meter");
    await page.goto("/");
    await page.getByRole("button", { name: "メニュー", exact: true }).click();
    await page.getByRole("link", { name: "温度計" }).click();
    await expect(sensor.locator("h2")).toHaveText("寝室・温湿度計");
    await expect(switchbot).toHaveCount(0);
    await page.screenshot({
      path: `test-results/screens/meter-unset-390-${scheme}.png`,
    });
  }
});
