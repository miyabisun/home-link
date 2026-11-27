// The routines' screen: written in YAML, run from their cards, with the phones and the home's
// pin a `near` step needs. How home-link reads the YAML and runs the steps is tests/api.rs's;
// the layout at each width and theme is tests/page.rs's.
import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { SceneRun } from "../lib/admin";
import {
  cleanup,
  fill,
  fireEvent,
  home,
  open,
  refused,
  screen,
  texts,
  title,
  viaMenu,
  within,
} from "../testing/home";
import { ledger } from "../testing/ledger";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const LABEL = "e2e-routine";
const PHONES = [
  ["3f2b6c1e-9a4d-4c55-8d1f-0e6a7b9c2d10", "Pixel 9"],
  [
    "7a1c0d9e-2b3f-4a5b-9c8d-1e2f3a4b5c6d",
    "家族が共用で使っている古いタブレット（リビングの棚に置いてあるもの）",
  ],
];
const HTTP =
  "http: POST http://127.0.0.1:1/a/very/long/path/that/must/wrap/on/a/phone/screen/without/running/off/to/the/side";
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
  - ${HTTP}
  - near: 100
    wait: 30
  - door: unlock
`;
const SCENES = [
  { name: "おやすみ", steps: [`off: ${LABEL}`, "door: lock"] },
  { name: "おはよう", steps: [`on: ${LABEL}`] },
  { name: "外出", steps: [`off: ${LABEL}`, "door: lock", "on: 無いlabel"] },
  { name: "帰宅", steps: [`on: ${LABEL}`, HTTP, "near: 100", "door: unlock"] },
];
// What running each answers: done, failed steps, and a wait at `near` for the phone.
const RUNS: Record<string, SceneRun> = {
  おはよう: {
    scene: "おはよう",
    status: "done",
    failed: 0,
    steps: [{ step: `on: ${LABEL}`, result: "ok" }],
  },
  外出: {
    scene: "外出",
    status: "done",
    failed: 2,
    steps: [
      { step: `off: ${LABEL}`, result: "ok" },
      {
        step: "door: lock",
        result: "failed",
        error: "door_not_configured",
        message: "SESAMEが設定されていません",
      },
      {
        step: "on: 無いlabel",
        result: "failed",
        error: "label_not_found",
        message: "labelの「無いlabel」がありません",
      },
    ],
  },
  帰宅: {
    scene: "帰宅",
    status: "waiting",
    failed: 1,
    steps: [
      { step: `on: ${LABEL}`, result: "ok" },
      {
        step: HTTP,
        result: "failed",
        error: "http_failed",
        message: "接続できませんでした",
      },
    ],
    next: 2,
    wait: 30,
    reason: "no_phone",
  },
};

/** The home as the screens show it: four routines, two phones, the pin set. */
function furnished(state = { yaml: YAML, scenes: SCENES }) {
  const phones = PHONES.map(([id, name]) => ({
    id,
    name,
    registered_at: 1_790_000_000,
  }));
  let pin: { latitude: number; longitude: number } | null = {
    latitude: 34.25,
    longitude: 132.5,
  };
  const place = () => ({
    name: null,
    latitude: 35.68,
    longitude: 139.76,
    registered: false,
    address: null,
    pin,
  });
  const desk = home({
    "GET /api/scenes": () => state,
    "GET /api/phones": () => phones,
    "GET /api/location": place,
    "GET /api/devices": ledger(),
    "GET /api/attention": { items: [] },
    "POST /api/scenes/*/run": (_, { path }) =>
      RUNS[decodeURIComponent(path.split("/")[3])],
    "DELETE /api/phones/*": (_, { path }) => {
      phones.splice(
        phones.findIndex((p) => path.endsWith(p.id)),
        1,
      );
      return null;
    },
    "PUT /api/location": (body) => {
      pin = body.pin;
      return place();
    },
  });
  return { desk, state };
}

const card = (name: string) =>
  screen
    .getByRole("button", { name: `${name}を実行` })
    .closest("li.card") as HTMLElement;

async function opened() {
  open("/scenes");
  await waitFor(() =>
    expect(
      document.querySelector("section.content")?.getAttribute("data-state"),
    ).toBe("success"),
  );
}

describe("the routines", () => {
  it("the routines, phones and pin are listed, and each run is told on its own card", async () => {
    furnished();
    await opened();
    expect(texts("li.card .name")).toEqual([
      "おやすみ",
      "おはよう",
      "外出",
      "帰宅",
      PHONES[0][1],
      PHONES[1][1],
      "登録済み",
    ]);
    expect(texts(".steps", card("外出"))).toEqual([
      `off: ${LABEL} → door: lock → on: 無いlabel`,
    ]);

    // Done, failed steps, and a wait: each told on its own card.
    await fireEvent.click(
      screen.getByRole("button", { name: "おはようを実行" }),
    );
    await waitFor(() =>
      expect(texts(".told", card("おはよう"))).toEqual(["完了"]),
    );
    await fireEvent.click(screen.getByRole("button", { name: "外出を実行" }));
    await waitFor(() =>
      expect(texts(".told", card("外出"))).toEqual(["2件失敗"]),
    );
    const done = texts(".steps-done li", card("外出"));
    expect(done[0]).toMatch(new RegExp(`^off: ${LABEL}\\s*済み$`));
    expect(done[1]).toMatch(/^door: lock\s*失敗\s*\S+/);
    expect(done[2]).toMatch(
      /^on: 無いlabel\s*失敗\s*labelの「無いlabel」がありません$/,
    );
    await fireEvent.click(screen.getByRole("button", { name: "帰宅を実行" }));
    await waitFor(() =>
      expect(texts(".told", card("帰宅"))).toEqual([
        "「near: 100」で停止。続きは電話のウィジェットから実行します",
      ]),
    );
    expect(texts(".steps-done .why", card("帰宅"))).toEqual([
      "接続できませんでした",
    ]);
    await fireEvent.click(screen.getByText("定義"));
    expect(
      (screen.getByLabelText("操作の定義（YAML）") as HTMLTextAreaElement)
        .value,
    ).toBe(YAML);
  });

  it("with nothing set up the screen says so at each place", async () => {
    const { desk } = furnished({ yaml: "", scenes: [] });
    desk.answer("GET /api/phones", []);
    desk.answer("GET /api/location", {
      name: null,
      latitude: 35.68,
      longitude: 139.76,
      registered: false,
      address: null,
      pin: null,
    });
    await opened();
    expect(screen.getByText("操作がありません")).toBeTruthy();
    expect(screen.getAllByText("未登録（アプリで登録）")).toHaveLength(2);
    expect(document.querySelectorAll("li.card")).toHaveLength(0);
  });

  it("the routines are rewritten in YAML, and one that does not read is refused with why", async () => {
    const { desk, state } = furnished();
    const next = `# 朝\nおはよう:\n  - on: ${LABEL}\n`;
    desk.answer("PUT /api/scenes", (body) => {
      if (body.yaml !== next)
        return refused(400, "「おはよう」の1番目: door は lock か unlock です");
      state.yaml = next;
      state.scenes = [SCENES[1]];
      return state;
    });
    open("/");
    await viaMenu("操作");
    expect(title()).toBe("操作");
    await fireEvent.click(await screen.findByText("定義"));
    const yaml = screen.getByLabelText(
      "操作の定義（YAML）",
    ) as HTMLTextAreaElement;
    const save = screen.getByRole("button", {
      name: "保存",
    }) as HTMLButtonElement;
    await waitFor(() => expect(yaml.value).toBe(YAML));
    expect(save.disabled).toBe(true);

    await fill(yaml, "おはよう:\n  - door: open\n");
    await fireEvent.click(save);
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent?.trim()).toBe(
        "「おはよう」の1番目: door は lock か unlock です",
      ),
    );
    expect(yaml.getAttribute("aria-invalid")).toBe("true");
    // The routines stay as they were saved, and so does what was typed.
    expect(screen.getAllByRole("button", { name: /を実行$/ })).toHaveLength(4);
    expect(yaml.value).toBe("おはよう:\n  - door: open\n");

    await fill(yaml, next);
    await fireEvent.click(save);
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: /を実行$/ })).toHaveLength(
        1,
      ),
    );
    expect(screen.queryByRole("alert")).toBeNull();
    expect(save.disabled).toBe(true);
    expect(desk.sent("PUT /api/scenes")).toEqual([
      { yaml: "おはよう:\n  - door: open\n" },
      { yaml: next },
    ]);

    await fireEvent.click(
      screen.getByRole("button", { name: "おはようを実行" }),
    );
    await waitFor(() =>
      expect(texts(".told", card("おはよう"))).toEqual(["完了"]),
    );
    await fireEvent.click(card("おはよう").querySelector("summary")!);
    expect(texts(".steps-done li", card("おはよう"))[0]).toMatch(
      new RegExp(`^on: ${LABEL}\\s*済み$`),
    );
  });

  it("a phone and the home's pin are removed after a confirmation", async () => {
    const { desk } = furnished();
    await opened();
    await fireEvent.click(
      screen.getByRole("button", { name: "Pixel 9を削除" }),
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog.textContent).toContain("「Pixel 9」を削除");
    await fireEvent.click(
      within(dialog).getByRole("button", { name: "キャンセル" }),
    );
    expect(screen.getByText("Pixel 9")).toBeTruthy();
    await fireEvent.click(
      screen.getByRole("button", { name: "Pixel 9を削除" }),
    );
    await fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "削除" }),
    );
    await waitFor(() => expect(screen.queryByText("Pixel 9")).toBeNull());
    expect(
      desk.asked.filter((a) => a.method === "DELETE").map((a) => a.path),
    ).toEqual([`/api/phones/${PHONES[0][0]}`]);

    await fireEvent.click(
      screen.getByRole("button", { name: "自宅の位置を削除" }),
    );
    await fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "削除" }),
    );
    await waitFor(() =>
      expect(screen.getAllByText("未登録（アプリで登録）")).toHaveLength(1),
    );
    expect(desk.sent("PUT /api/location")).toEqual([{ pin: null }]);
  });
});
