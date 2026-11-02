import { expect, test, type Page } from "@playwright/test";
import WebSocket from "ws";

// PCs connected to home-link as windows-link does: each says hello and answers the
// commands. Images go to test-results/screens.
const widths = [
  { width: 1280, height: 800 },
  { width: 390, height: 844 },
];
const schemes = ["light", "dark"] as const;
const PORT = "5198";

interface Pc {
  socket: WebSocket;
  commands: string[];
  close: () => Promise<void>;
}

/** A PC named `name`; `answer` decides each result. */
async function connect(
  name: string,
  sleeping: boolean | null,
  answer: (action: string) => object = (action) => ({
    ok: true,
    sleeping: action === "sleep",
  }),
): Promise<Pc> {
  const socket = new WebSocket(`ws://127.0.0.1:${PORT}/api/pcs/connect`);
  await new Promise((resolve, reject) => {
    socket.once("open", resolve);
    socket.once("error", reject);
  });
  const commands: string[] = [];
  socket.on("message", (data) => {
    const message = JSON.parse(String(data));
    if (message.type !== "command") return;
    commands.push(message.action);
    socket.send(
      JSON.stringify({
        type: "result",
        id: message.id,
        ...answer(message.action),
      }),
    );
  });
  socket.send(
    JSON.stringify(
      sleeping === null
        ? { type: "hello", name }
        : { type: "hello", name, sleeping },
    ),
  );
  return {
    socket,
    commands,
    close: () =>
      new Promise((resolve) => {
        socket.once("close", () => resolve());
        socket.close();
      }),
  };
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
    test(`PCs at ${size.width}px in ${scheme}`, async ({ page }) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: scheme });
      const pcs = [
        await connect("書斎", false),
        await connect(
          "ゲーム用のデスクトップPC（リビングのテレビの横の棚の中）",
          true,
        ),
        await connect("ノート", null),
      ];
      try {
        await page.goto("/pcs");
        const cards = page.locator(".card");
        await expect(cards).toHaveCount(3);
        // By name, as the server sorts them.
        await expect(cards.nth(0)).toContainText("ゲーム用");
        await expect(cards.nth(0).getByRole("button")).toHaveText("スリープ中");
        await expect(cards.nth(1)).toContainText("ノート");
        await expect(cards.nth(1).getByRole("button")).toHaveText([
          "スリープ",
          "復帰",
        ]);
        await expect(cards.nth(2).getByRole("button")).toHaveText("動作中");
        await noSideScroll(page);
        await page.screenshot({
          path: `test-results/screens/pcs-${size.width}-${scheme}.png`,
          fullPage: true,
        });
      } finally {
        await Promise.all(pcs.map((pc) => pc.close()));
      }
    });
  }
}

test("a PC is put to sleep and woken from its card", async ({ page }) => {
  const study = await connect("書斎", false);
  try {
    await page.goto("/");
    await page.getByRole("button", { name: "メニュー" }).click();
    await page.getByRole("link", { name: "PC" }).click();
    await expect(page.locator(".sub-header h1")).toHaveText("PC");

    const button = page.getByRole("button", { name: "スリープ" });
    await expect(button).toHaveAttribute("aria-pressed", "false");
    await button.click();
    await expect(button).toHaveAttribute("aria-pressed", "true");
    await expect(button).toHaveText("スリープ中");
    await button.press("Enter");
    await expect(button).toHaveAttribute("aria-pressed", "false");
    await expect(button).toHaveText("動作中");
    expect(study.commands).toEqual(["sleep", "wake"]);
  } finally {
    await study.close();
  }
});

test("a PC that refuses, or one that is gone, says so above the list", async ({
  page,
}) => {
  const study = await connect("書斎", false, () => ({
    ok: false,
    message: "モニターがありません",
  }));
  try {
    await page.goto("/pcs");
    await page.getByRole("button", { name: "スリープ" }).click();
    await expect(page.getByRole("alert")).toHaveText(
      "書斎がスリープを受け付けませんでした: モニターがありません",
    );
    await expect(
      page.getByRole("button", { name: "スリープ" }),
    ).toHaveAttribute("aria-pressed", "false");
  } finally {
    await study.close();
  }
  // Gone after the page listed it: the error names it and the list drops it.
  await page.getByRole("button", { name: "スリープ" }).click();
  await expect(page.getByRole("alert")).toHaveText(
    "書斎はhome-linkにつながっていません",
  );
  await expect(page.getByText("つながっているPCはありません")).toBeVisible();
});

test("no PC connected shows that there is none", async ({ page }) => {
  await page.goto("/pcs");
  await expect(page.getByText("つながっているPCはありません")).toBeVisible();
  await noSideScroll(page);
});
