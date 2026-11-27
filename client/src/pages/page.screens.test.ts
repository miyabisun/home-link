// What the screens say beside their layout: an input error and the delete confirmation, and
// the meters. Their layout at each width and theme is tests/page.rs's.
import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  cleanup,
  fill,
  fireEvent,
  home,
  open,
  screen,
  texts,
  viaMenu,
  within,
} from "../testing/home";
import { label, ledger } from "../testing/ledger";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const now = () => Math.floor(Date.now() / 1000);
const address = "AA:BB:CC:DD:EE:01";
const adapter = { present: true, powered: true, discovering: true };
const mended = { powered_on_at: null, power_cycled_at: null };
// What GET /api/meter answers with a SwitchBot meter, in each of its states.
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
  // No SwitchBot meter set up.
  unset: () => ({
    address: null,
    reading: null,
    received_at: null,
    fresh_for: 300,
    adapter: null,
    ...mended,
  }),
};

const work = label({
  id: 1,
  name: "作業",
  override_level: 254,
  override_kelvin: 5000,
});

function homeWith(kind: { current: keyof typeof meters }) {
  const kept = {
    name: null as string | null,
    labels: [] as number[],
    on: true,
  };
  const meter = () => ({
    ...meters[kind.current](),
    ...kept,
    labels: kept.labels.map((id) => ({
      id,
      name: id === 1 ? "作業" : String(id),
    })),
  });
  return home({
    "GET /api/devices": ledger(),
    "GET /api/labels": [work],
    "GET /api/attention": { items: [] },
    "GET /api/meter": meter,
    "PUT /api/meter": (body) => {
      Object.assign(kept, body);
      return meter();
    },
  });
}

describe("the screens", () => {
  it("the delete confirmation and an input error stay on a phone screen", async () => {
    home({
      "GET /api/labels": [work],
      "GET /api/devices": ledger(),
    });
    open("/labels/1");
    await fill(await screen.findByLabelText("明るさ（%）"), "101");
    expect(screen.getByRole("alert").textContent?.trim()).toBe(
      "明るさは1〜100%",
    );
    await fireEvent.click(screen.getByRole("button", { name: "削除" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog.textContent).toContain("「作業」を削除");
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("the meters show their values, a stopped reception, and having none", async () => {
    const kind = { current: "fresh" as keyof typeof meters };
    const desk = homeWith(kind);
    open("/meter");
    // The ledger's sensor first, linked to its device, then the SwitchBot meter.
    await waitFor(() =>
      expect(document.querySelector("section.meter")).not.toBeNull(),
    );
    const sensor = document.querySelector("section.meter")!;
    expect(texts("h2", sensor)).toEqual(["寝室・温湿度計"]);
    expect(
      within(sensor as HTMLElement)
        .getByRole("link")
        .getAttribute("href"),
    ).toBe("/devices/6");
    expect(texts(".reading dd", sensor)).toEqual(["28.3℃", "52%", "90%"]);
    const switchbot = () =>
      document.querySelector(
        'section[aria-labelledby="switchbot"]',
      ) as HTMLElement;
    expect(texts("h2", switchbot())).toEqual(["SwitchBot"]);
    expect(texts(".reading dd", switchbot())).toEqual([
      "26.7℃",
      "57%",
      "717ppm",
      "74%",
    ]);
    expect(texts(".received", switchbot())[0]).toMatch(/^\d\d:\d\d:\d\d 受信$/);

    // Named, and named back by a blank.
    await fireEvent.click(
      within(switchbot()).getByRole("button", { name: "名前を変える" }),
    );
    await fill(screen.getByLabelText("名前"), "リビング");
    await waitFor(() => expect(texts("#switchbot")).toEqual(["リビング"]));
    await fireEvent.click(screen.getByRole("button", { name: "名前を変える" }));
    expect((screen.getByLabelText("名前") as HTMLInputElement).value).toBe(
      "リビング",
    );
    await fill(screen.getByLabelText("名前"), "");
    await waitFor(() => expect(texts("#switchbot")).toEqual(["SwitchBot"]));
    expect(desk.sent("PUT /api/meter")).toEqual([
      { name: "リビング" },
      { name: null },
    ]);

    // Put under a label and switched off, it says so by its name.
    await fireEvent.click(within(switchbot()).getByText("自動運転"));
    await fireEvent.click(
      within(switchbot()).getByRole("checkbox", { name: "作業" }),
    );
    await waitFor(() =>
      expect(desk.sent("PUT /api/meter").at(-1)).toEqual({ labels: [1] }),
    );
    await waitFor(() =>
      expect(
        (
          within(switchbot()).getByRole("checkbox", {
            name: "作業",
          }) as HTMLInputElement
        ).checked,
      ).toBe(true),
    );
    const power = within(switchbot()).getByRole("button", { name: "自動運転" });
    expect(power.getAttribute("aria-pressed")).toBe("true");
    await fireEvent.click(power);
    await waitFor(() => expect(power.textContent?.trim()).toBe("切"));
    expect(texts(".badge", switchbot())).toEqual(["自動運転 切"]);

    kind.current = "stale";
    open("/meter");
    await waitFor(() => expect(switchbot()).not.toBeNull());
    expect(texts(".state", switchbot())).toEqual(["受信が途絶えています"]);
    expect(switchbot().querySelectorAll(".reading dd")).toHaveLength(0);
    expect(texts(".note", switchbot())[0]).toMatch(
      /^最後の受信 \d+\/\d+ \d\d:\d\d$/,
    );

    kind.current = "off";
    open("/meter");
    await waitFor(() => expect(switchbot()).not.toBeNull());
    expect(texts(".state", switchbot())).toEqual(["受信が途絶えています"]);
    const notes = texts(".note", switchbot());
    expect(notes).toHaveLength(3);
    expect(notes[0]).toMatch(/^最後の受信 \d+\/\d+ \d\d:\d\d$/);
    expect(notes[1]).toBe("アダプタの電源が入っていません");
    expect(notes[2]).toMatch(/^電源の入れ直し \d+\/\d+ \d\d:\d\d$/);

    kind.current = "waiting";
    open("/meter");
    await waitFor(() => expect(switchbot()).not.toBeNull());
    expect(texts(".state", switchbot())).toEqual(["まだ受信していません"]);

    // With no SwitchBot meter set up: the sensor alone.
    kind.current = "unset";
    open("/");
    await viaMenu("温度計");
    await waitFor(() =>
      expect(texts("section.meter h2")).toEqual(["寝室・温湿度計"]),
    );
    expect(switchbot()).toBeNull();
    expect(desk.strays).toEqual([]);
  });
});
