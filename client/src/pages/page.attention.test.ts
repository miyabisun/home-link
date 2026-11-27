// What asks for the user, at the top of the home and of the meter's page. The height of its
// links and its layout at each width and theme are tests/page.rs's.
import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  cleanup,
  fireEvent,
  home,
  open,
  screen,
  title,
  within,
} from "../testing/home";
import { ledger } from "../testing/ledger";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

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

const asking = (listed: object[]) =>
  home({
    "GET /api/attention": { items: listed },
    "GET /api/devices": ledger(),
    "GET /api/labels": [],
    "GET /api/lights/watch": {
      checked_at: now,
      server_reachable: true,
      server_since: minutes(600),
      server_outage: null,
      wifi_all_down: false,
      lights: [],
    },
    "GET /api/lights/outages": { outages: [] },
    "GET /api/meter": {
      address: "AA:BB:CC:DD:EE:01",
      name: null,
      labels: [],
      on: true,
      reading: null,
      received_at: minutes(20),
      fresh_for: 300,
      adapter: { present: false, powered: false, discovering: false },
      powered_on_at: null,
      power_cycled_at: null,
    },
  });

const links = () =>
  within(screen.getByRole("list", { name: "要対応" })).getAllByRole("link");

describe("what asks for the user", () => {
  it("what asks for the user is listed at the top of the home", async () => {
    asking(items);
    open("/");
    await waitFor(() => expect(links()).toHaveLength(4));
    expect(links()[0].textContent).toContain(
      "からつながりません。照明を操作できません。",
    );
    expect(links()[1].textContent).toContain(
      "壁のスイッチで電源を入れ直してください。",
    );
    expect(links()[2].textContent).toContain("アプリで登録し直してください。");
    expect(links()[3].textContent).toContain(
      "UB500が外れています。挿し直してください。",
    );
  });

  it("nothing asking leaves the home as it was", async () => {
    const desk = asking([]);
    open("/");
    await waitFor(() =>
      expect(
        document.querySelector("section.content")?.getAttribute("data-state"),
      ).not.toBe("loading"),
    );
    await waitFor(() =>
      expect(desk.sent("GET /api/attention")).toHaveLength(1),
    );
    expect(screen.queryByRole("list", { name: "要対応" })).toBeNull();
  });

  it("each item leads to the page that shows more of it", async () => {
    asking(items);
    open("/");
    await waitFor(() => expect(links()).toHaveLength(4));
    await fireEvent.click(links()[1]);
    expect(title()).toBe("照明の接続");
    open("/");
    await waitFor(() => expect(links()).toHaveLength(4));
    await fireEvent.click(links()[3]);
    expect(title()).toBe("温度計");
  });

  it("the meter's page shows what of the meter asks for the user", async () => {
    asking(items);
    open("/meter");
    await waitFor(() => expect(links()).toHaveLength(1));
    expect(links()[0].textContent).toContain("温度計を");
  });
});
