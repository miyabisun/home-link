import { DatabaseSync } from "node:sqlite";

import { expect, test, type Page } from "@playwright/test";

// Every change goes through the screen and the API; the ledger file is read to see it landed.
function sql(query: string, ...params: (string | number)[]): unknown[] {
  const db = new DatabaseSync(process.env.E2E_DB!, { readOnly: true });
  try {
    return db
      .prepare(query)
      .all(...params)
      .map((row) => ({ ...row }));
  } finally {
    db.close();
  }
}

const labelRow = (name: string) =>
  sql(
    `SELECT id, auto, override_level, override_kelvin, "on" FROM labels WHERE name = ?`,
    name,
  )[0] as Record<string, number | null> | undefined;

const deviceId = (name: string) =>
  (sql("SELECT id FROM devices WHERE name = ?", name)[0] as { id: number }).id;

const labelsOf = (device: number) =>
  sql(
    `SELECT labels.name FROM device_labels JOIN labels ON labels.id = label_id
     WHERE device_id = ? ORDER BY labels.name`,
    device,
  ).map((row) => (row as { name: string }).name);

async function commit(page: Page) {
  await page.keyboard.press("Tab");
}

test.describe.configure({ mode: "serial" });

test("the device list shows names and the states that need attention", async ({
  page,
}) => {
  await page.goto("/");
  const cards = page.locator(".card");
  await expect(cards).toHaveCount(5);
  await expect(cards.nth(0)).toContainText("居間の照明");
  await expect(cards.nth(1)).toContainText("P110M");
  await expect(cards.nth(2)).toContainText("押入れ1");
  await expect(cards.nth(2)).toContainText("未接続");
  await expect(cards.nth(0).locator(".badge")).toHaveCount(0);
});

test("labels are created, renamed, dimmed, fixed and scheduled", async ({
  page,
}) => {
  await page.goto("/labels");
  const name = page.getByRole("textbox", { name: "ラベル名" });

  await page.getByRole("button", { name: "追加" }).click();
  await expect(page.getByRole("alert")).toHaveText("1〜100文字");

  await name.fill("ambient");
  await page.getByRole("button", { name: "追加" }).click();
  await expect(page).toHaveURL(/\/labels\/\d+$/);
  await expect(page.locator(".sub-header")).toHaveText("ambient");
  expect(labelRow("ambient")).toMatchObject({ auto: 0, override_level: null });

  await page.getByLabel("名前", { exact: true }).fill("常夜灯");
  await commit(page);
  await expect(page.locator(".sub-header")).toHaveText("常夜灯");
  await expect.poll(() => labelRow("常夜灯")).toBeDefined();

  await page.getByLabel("調光").selectOption("自動調光");
  await expect.poll(() => labelRow("常夜灯")?.auto).toBe(1);

  await page.getByLabel("調光").selectOption("固定");
  await expect
    .poll(() => labelRow("常夜灯"))
    .toMatchObject({ auto: 0, override_level: 203, override_kelvin: 5000 });
  await expect(page.getByLabel("明るさ（%）")).toHaveValue("80");

  await page.getByLabel("明るさ（%）").fill("0");
  await commit(page);
  await expect(page.getByRole("alert")).toHaveText("明るさは1〜100%");
  await expect(page.getByLabel("明るさ（%）")).toHaveAttribute(
    "aria-invalid",
    "true",
  );
  await expect(page.getByLabel("色温度（K）")).not.toHaveAttribute(
    "aria-invalid",
  );
  await page.getByLabel("明るさ（%）").fill("100");
  await page.getByLabel("色温度（K）").fill("4000");
  await commit(page);
  await expect
    .poll(() => labelRow("常夜灯"))
    .toMatchObject({ override_level: 254, override_kelvin: 4000 });

  const schedule = () =>
    sql(
      `SELECT minute, "on" FROM label_schedules WHERE label_id = ? ORDER BY minute`,
      labelRow("常夜灯")!.id!,
    );
  await page.getByRole("button", { name: "時刻を追加" }).click();
  await page.getByLabel("時刻1", { exact: true }).fill("07:00");
  await expect.poll(schedule).toEqual([{ minute: 420, on: 1 }]);
  await page.getByRole("button", { name: "時刻を追加" }).click();
  await page.getByLabel("時刻2", { exact: true }).fill("23:30");
  await page.getByLabel("操作2").selectOption("切");
  await expect.poll(schedule).toEqual([
    { minute: 420, on: 1 },
    { minute: 1410, on: 0 },
  ]);
  await page.getByRole("button", { name: "時刻1を削除" }).click();
  await expect.poll(schedule).toEqual([{ minute: 1410, on: 0 }]);

  await page.reload();
  await expect(page.getByLabel("調光")).toHaveValue("fixed");
  await expect(page.getByLabel("時刻1", { exact: true })).toHaveValue("23:30");
});

test("a label assigns devices and switches them", async ({ page }) => {
  const label = labelRow("常夜灯")!;
  await page.goto(`/labels/${label.id}`);
  await page.getByRole("checkbox", { name: "居間の照明" }).check();
  await expect.poll(() => labelsOf(deviceId("居間の照明"))).toEqual(["常夜灯"]);

  const toggle = page.getByRole("button", { name: "入り切り" });
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() => labelRow("常夜灯")?.on).toBe(1);

  await page.getByRole("checkbox", { name: "居間の照明" }).uncheck();
  await expect.poll(() => labelsOf(deviceId("居間の照明"))).toEqual([]);
});

test("a device is renamed, labelled, ignored, switched and deleted", async ({
  page,
}) => {
  const id = deviceId("居間の照明");
  await page.goto(`/devices/${id}`);
  await page.getByLabel("名前", { exact: true }).fill("リビング");
  await commit(page);
  await expect(page.locator(".sub-header")).toHaveText("リビング");
  await expect
    .poll(() => sql("SELECT name FROM devices WHERE id = ?", id))
    .toEqual([{ name: "リビング" }]);

  await page.getByRole("checkbox", { name: "常夜灯" }).check();
  await expect.poll(() => labelsOf(id)).toEqual(["常夜灯"]);
  await page.getByRole("checkbox", { name: "無視" }).check();
  await expect
    .poll(() => sql("SELECT ignored FROM devices WHERE id = ?", id))
    .toEqual([{ ignored: 1 }]);

  await page.goto("/");
  await page.getByRole("link", { name: /P110M/ }).click();
  const plug = page.getByRole("button", { name: "入り切り" });
  await expect(plug).toHaveText("切");
  await plug.click();
  await expect(plug).toHaveAttribute("aria-pressed", "true");
  await expect(plug).toHaveText("入");
  await page.goto("/");
  await expect(
    page.getByRole("link", { name: /P110M/ }).locator(".badge"),
  ).toHaveText("入");

  await page.goto(`/devices/${id}`);
  await page.getByRole("button", { name: "削除", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "キャンセル" })
    .click();
  expect(sql("SELECT id FROM devices WHERE id = ?", id)).toHaveLength(1);
  await page.getByRole("button", { name: "削除", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "削除", exact: true })
    .click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator(".card")).toHaveCount(4);
  expect(sql("SELECT id FROM devices WHERE id = ?", id)).toEqual([]);
  expect(sql("SELECT * FROM device_labels WHERE device_id = ?", id)).toEqual(
    [],
  );
});

test("labels keep their names unique and are deleted", async ({ page }) => {
  await page.goto("/labels");
  await page.getByRole("textbox", { name: "ラベル名" }).fill("常夜灯");
  await page.getByRole("button", { name: "追加" }).click();
  await expect(page.getByRole("alert")).toHaveText("同じ名前のlabelがあります");

  await page.getByRole("link", { name: /常夜灯/ }).click();
  await page.getByRole("button", { name: "削除", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "削除", exact: true })
    .click();
  await expect(page).toHaveURL(/\/labels$/);
  expect(labelRow("常夜灯")).toBeUndefined();
});

const airconSettings = () =>
  sql("SELECT value FROM settings WHERE key = 'aircon'").map((row) =>
    JSON.parse((row as { value: string }).value),
  )[0];

test("the air conditioner is switched off and on and gets a target", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("button", { name: "メニュー", exact: true }).click();
  await page
    .locator("nav.menu")
    .getByRole("link", { name: "エアコン" })
    .click();
  await expect(page.locator("h1")).toHaveText("エアコン");
  const values = page.locator(".reading dd");
  // The E2E home-link has no meter: no reading, and the air conditioner as it is.
  await expect(values).toHaveText(["—", "—", "冷房 25℃"]);
  const power = page.getByRole("button", { name: "入り切り" });
  await expect(power).toHaveAttribute("aria-pressed", "true");

  await power.click();
  await expect(power).toHaveText("切");
  await expect(power).toHaveAttribute("aria-pressed", "false");
  await expect(values.nth(2)).toHaveText("切");
  expect(airconSettings()).toEqual({ on: false, target: 25 });

  const target = page.getByLabel("目標");
  await target.fill("31");
  await commit(page);
  await expect(page.getByRole("alert")).toHaveText("18〜30℃");
  expect(airconSettings().target).toBe(25);
  await target.fill("23");
  await commit(page);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect.poll(() => airconSettings().target).toBe(23);

  await power.click();
  await expect(power).toHaveText("入");
  expect(airconSettings()).toEqual({ on: true, target: 23 });
  // Still no reading: switching on waits for one before sending.
  await expect(values.nth(2)).toHaveText("切");
  await page.reload();
  await expect(target).toHaveValue("23");
  await expect(power).toHaveAttribute("aria-pressed", "true");
});
