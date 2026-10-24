import { expect, test, type Page } from "@playwright/test";

// Devices that do not answer, and the Thread base pointed to when only Thread devices do not,
// at a desk and a phone width in both themes. Images go to test-results/screens.
const widths = [
  { width: 1280, height: 800 },
  { width: 390, height: 844 },
];
const schemes = ["light", "dark"] as const;
const THREAD_DOWN =
  "Threadの機器が応答していません（Threadの基地を確かめてください）";

let labelId = 0;

// The ledger as home-link lists it while its first devices, on the networks `down` gives
// in order, do not answer; the others answer on Wi-Fi.
async function devicesDown(page: Page, down: string[]) {
  await page.unroute("/api/devices");
  await page.route("/api/devices", async (route) => {
    const devices = await (await route.fetch()).json();
    devices.forEach((device: Record<string, unknown>, i: number) => {
      device.network = down[i] ?? "wifi";
      if (i < down.length) device.state = "no_response";
    });
    await route.fulfill({ json: devices });
  });
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

test.beforeAll(async ({ request }) => {
  const labels = await (await request.get("/api/labels")).json();
  const found = labels.find((l: { name: string }) => l.name === "居間");
  if (found) labelId = found.id;
  else {
    const response = await request.post("/api/labels", {
      data: { name: "居間" },
    });
    expect(response.ok()).toBe(true);
    labelId = (await response.json()).id;
  }
});

for (const size of widths) {
  for (const scheme of schemes) {
    test(`devices not answering at ${size.width}px in ${scheme}`, async ({
      page,
    }) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: scheme });
      const shot = (name: string) =>
        page.screenshot({
          path: `test-results/screens/${name}-${size.width}-${scheme}.png`,
          fullPage: true,
        });

      // Only Thread devices are down: the Thread base is pointed to, each device marked.
      await devicesDown(page, ["thread", "thread"]);
      await page.goto("/");
      const cards = page.locator(".card");
      await expect(page.getByRole("alert")).toHaveText(THREAD_DOWN);
      await expect(cards.nth(0)).toContainText("応答なし");
      await expect(cards.nth(1)).toContainText("応答なし");
      await noSideScroll(page);
      await shot("thread-down");

      // A Wi-Fi device is down too: no single cause, only the devices are marked.
      await devicesDown(page, ["thread", "wifi"]);
      await page.goto("/");
      await expect(cards.nth(1)).toContainText("応答なし");
      await expect(page.getByRole("alert")).toHaveCount(0);

      // Switching a label names the devices that did not answer.
      await page.route(`/api/labels/${labelId}/toggle`, (route) =>
        route.fulfill({
          json: {
            on: true,
            switched: 1,
            no_response: 2,
            failed: 0,
            missing: 0,
            devices: [
              { name: "書斎", result: "switched", network: "wifi" },
              { name: "居間の照明", result: "no_response", network: "thread" },
              {
                name: "寝室の天井の照明（窓側）",
                result: "no_response",
                network: "thread",
              },
            ],
          },
        }),
      );
      await page.goto(`/labels/${labelId}`);
      await page.getByRole("button", { name: "入り切り" }).click();
      await expect(page.getByRole("alert")).toHaveText(
        `応答なし 2（居間の照明・寝室の天井の照明（窓側））。${THREAD_DOWN}`,
      );
      await noSideScroll(page);
      await shot("label-thread-down");
    });
  }
}
