import {
  expect,
  test,
  type APIRequestContext,
  type Page,
} from "@playwright/test";

// The routines' screen against the built home-link: written in YAML, run from their cards, with
// the phones and the home's pin a `near` step needs. Images go to test-results/screens.
const widths = [
  { width: 1280, height: 800 },
  { width: 390, height: 844 },
];
const schemes = [
  { scheme: "light", background: "rgb(250, 246, 239)" },
  { scheme: "dark", background: "rgb(25, 25, 25)" },
] as const;

const LABEL = "e2e-routine";
const PHONES = [
  ["3f2b6c1e-9a4d-4c55-8d1f-0e6a7b9c2d10", "Pixel 9"],
  [
    "7a1c0d9e-2b3f-4a5b-9c8d-1e2f3a4b5c6d",
    "家族が共用で使っている古いタブレット（リビングの棚に置いてあるもの）",
  ],
];
const YAML = `# 操作ごとに、上から順に実行するステップを書く。
おやすみ:
  - off: ${LABEL}
  - door: lock
おはよう:
  - on: ${LABEL}
外出:
  - off: ${LABEL}
  - door: lock
  - on: 無いlabel
帰宅:
  - on: ${LABEL}
  - http: POST http://127.0.0.1:1/a/very/long/path/that/must/wrap/on/a/phone/screen/without/running/off/to/the/side
  - near: 100
    wait: 30
  - door: unlock
`;

async function write(request: APIRequestContext, yaml: string) {
  expect((await request.put("/api/scenes", { data: { yaml } })).status()).toBe(
    200,
  );
}

/** The home as the screens show it: four routines, two phones, the pin set. */
async function furnish(request: APIRequestContext) {
  await write(request, YAML);
  for (const [id, name] of PHONES) {
    await request.put(`/api/phones/${id}`, { data: { name } });
  }
  await request.put("/api/location", {
    data: { pin: { latitude: 34.25, longitude: 132.5 } },
  });
}

/** A home that has set nothing up. */
async function strip(request: APIRequestContext) {
  await write(request, "");
  for (const [id] of PHONES) await request.delete(`/api/phones/${id}`);
  await request.put("/api/location", { data: { pin: null } });
}

const card = (page: Page, name: string) =>
  page.locator("li.card").filter({
    has: page.getByRole("button", { name: `${name}を実行` }),
  });

async function open(page: Page) {
  await page.goto("/scenes");
  await expect(page.locator("section.content")).toHaveAttribute(
    "data-state",
    "success",
  );
}

/** The whole page from its top, where the sticky headers sit as the user first sees them. */
async function shoot(page: Page, name: string) {
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: `test-results/screens/${name}.png`,
    fullPage: true,
  });
}

async function layout(page: Page) {
  return page.evaluate(() => ({
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
}

test.beforeAll(async ({ request }) => {
  const labels = await (await request.get("/api/labels")).json();
  let label = labels.find((l: { name: string }) => l.name === LABEL)?.id;
  if (label === undefined) {
    const created = await request.post("/api/labels", {
      data: { name: LABEL },
    });
    label = (await created.json()).id;
  }
  const devices = await (await request.get("/api/devices")).json();
  for (const device of devices.slice(0, 2)) {
    const ids = device.labels.map((l: { id: number }) => l.id);
    await request.patch(`/api/devices/${device.id}`, {
      data: { label_ids: [...new Set([...ids, label])] },
    });
  }
});

for (const size of widths) {
  for (const { scheme, background } of schemes) {
    test(`scenes at ${size.width}px in ${scheme}`, async ({
      page,
      request,
    }) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: scheme });
      await furnish(request);
      await open(page);
      await expect(page.locator("li.card .name")).toHaveText([
        "おやすみ",
        "おはよう",
        "外出",
        "帰宅",
        PHONES[0][1],
        PHONES[1][1],
        "登録済み",
      ]);
      await expect(card(page, "外出").locator(".steps")).toHaveText(
        `off: ${LABEL} → door: lock → on: 無いlabel`,
      );
      expect(await layout(page), "as opened").toEqual({
        overflow: 0,
        background,
        innerScroll: false,
      });
      await shoot(page, `scenes-${size.width}-${scheme}`);

      // Done, failed steps, and a wait: each told on its own card.
      await page.getByRole("button", { name: "おはようを実行" }).click();
      await expect(card(page, "おはよう").locator(".told")).toHaveText("完了");
      await page.getByRole("button", { name: "外出を実行" }).click();
      await expect(card(page, "外出").locator(".told")).toHaveText("2件失敗");
      await expect(card(page, "外出").locator(".steps-done li")).toHaveText([
        new RegExp(`^off: ${LABEL}\\s*済み\\s*$`),
        /^door: lock\s*失敗\s*\S+/,
        /^on: 無いlabel\s*失敗\s*labelの「無いlabel」がありません\s*$/,
      ]);
      await page.getByRole("button", { name: "帰宅を実行" }).click();
      await expect(card(page, "帰宅").locator(".told")).toHaveText(
        "「near: 100」で停止。続きは電話のウィジェットから実行します",
      );
      await expect(card(page, "帰宅").locator(".steps-done .why")).toHaveText(
        "接続できませんでした",
      );
      await page.getByText("定義", { exact: true }).click();
      await expect(page.getByLabel("操作の定義（YAML）")).toHaveValue(YAML);
      expect(await layout(page), "run and opened").toEqual({
        overflow: 0,
        background,
        innerScroll: false,
      });
      await shoot(page, `scenes-run-${size.width}-${scheme}`);
    });
  }
}

test("with nothing set up the screen says so at each place", async ({
  page,
  request,
}) => {
  await strip(request);
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page);
  await expect(page.getByText("操作がありません")).toBeVisible();
  await expect(page.getByText("未登録（アプリで登録）")).toHaveCount(2);
  await expect(page.locator("li.card")).toHaveCount(0);
  expect((await layout(page)).overflow).toBe(0);
  await shoot(page, "scenes-empty-390-light");
});

test("the routines are rewritten in YAML, and one that does not read is refused with why", async ({
  page,
  request,
}) => {
  await furnish(request);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await page.getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("link", { name: "操作" }).click();
  await expect(page.locator(".sub-header h1")).toHaveText("操作");
  await page.getByText("定義", { exact: true }).click();
  const yaml = page.getByLabel("操作の定義（YAML）");
  const save = page.getByRole("button", { name: "保存" });
  await expect(yaml).toHaveValue(YAML);
  await expect(save).toBeDisabled();

  await yaml.fill("おはよう:\n  - door: open\n");
  await save.click();
  await expect(page.getByRole("alert")).toHaveText(
    "「おはよう」の1番目: door は lock か unlock です",
  );
  await expect(yaml).toHaveAttribute("aria-invalid", "true");
  // The routines stay as they were saved, and so does what was typed.
  await expect(page.getByRole("button", { name: /を実行$/ })).toHaveCount(4);
  await expect(yaml).toHaveValue("おはよう:\n  - door: open\n");
  await shoot(page, "scenes-invalid-1280-dark");

  const next = `# 朝\nおはよう:\n  - on: ${LABEL}\n`;
  await yaml.fill(next);
  await save.press("Enter");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("button", { name: /を実行$/ })).toHaveCount(1);
  await expect(save).toBeDisabled();
  expect((await (await request.get("/api/scenes")).json()).yaml).toBe(next);

  // Run from the keyboard.
  const run = page.getByRole("button", { name: "おはようを実行" });
  await run.focus();
  await page.keyboard.press("Enter");
  await expect(card(page, "おはよう").locator(".told")).toHaveText("完了");
  await card(page, "おはよう").locator("summary").click();
  await expect(card(page, "おはよう").locator(".steps-done li")).toHaveText(
    new RegExp(`^on: ${LABEL}\\s*済み\\s*$`),
  );
});

test("a phone and the home's pin are removed after a confirmation", async ({
  page,
  request,
}) => {
  await furnish(request);
  await open(page);
  await page.getByRole("button", { name: "Pixel 9を削除" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("「Pixel 9」を削除");
  await dialog.getByRole("button", { name: "キャンセル" }).click();
  await expect(page.getByText("Pixel 9", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Pixel 9を削除" }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "削除", exact: true })
    .click();
  await expect(page.getByText("Pixel 9", { exact: true })).toHaveCount(0);
  const phones = await (await request.get("/api/phones")).json();
  expect(phones.map((phone: { name: string }) => phone.name)).toEqual([
    PHONES[1][1],
  ]);

  await page.getByRole("button", { name: "自宅の位置を削除" }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "削除", exact: true })
    .click();
  await expect(page.getByText("未登録（アプリで登録）")).toHaveCount(1);
  expect((await (await request.get("/api/location")).json()).pin).toBeNull();
});
