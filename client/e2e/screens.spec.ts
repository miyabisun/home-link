import { expect, test } from "@playwright/test";

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
      const screens = [
        ["devices", "/"],
        ["device", `/devices/${deviceId}`],
        ["labels", "/labels"],
        ["label", `/labels/${labelId}`],
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
