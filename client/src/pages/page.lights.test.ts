// The light watch's page: a light down, a long name, one that left the fabric and
// matterjs-server out of reach. Its layout at each width and theme is tests/page.rs's.
import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  cleanup,
  fireEvent,
  home,
  open,
  path,
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

const now = Math.floor(Date.now() / 1000);
const minutes = (n: number) => now - n * 60;
/** The day of `unix` in Tokyo, as the admin screen writes it (`10/17`). */
const day = (unix: number) =>
  new Intl.DateTimeFormat("ja-JP", {
    timeZone: "Asia/Tokyo",
    month: "numeric",
    day: "numeric",
  }).format(unix * 1000);

const light = {
  node_id: 5,
  endpoint: 1,
  device_id: 3,
  name: "玄関",
  network: "wifi",
  connected: true,
  since: minutes(90),
  last_answer: minutes(2),
  outage: null,
};
// A light no ledger row holds, named by its product: what a deleted row used to leave behind.
const down = {
  ...light,
  node_id: 6,
  device_id: null,
  name: "廊下の天井の照明（階段の上の吹き抜けの側）",
  connected: false,
  since: minutes(25),
  last_answer: minutes(27),
  outage: {
    node_id: 6,
    endpoint: 1,
    name: "廊下の天井の照明（階段の上の吹き抜けの側）",
    network: "wifi",
    started_at: minutes(25),
    ended_at: null,
    found_by: "unavailable",
    reconnects: 3,
    reconnect_result: "no_answer",
    reachable: false,
  },
};
const thread = {
  ...light,
  node_id: 7,
  device_id: 7,
  name: "寝室",
  network: "thread",
  since: minutes(600),
};
// A light that left the fabric five days ago, as matterjs-server sent.
const left = {
  node_id: null,
  endpoint: null,
  device_id: 19,
  name: "物置き1",
  network: null,
  connected: false,
  state: "left_fabric",
  since: minutes(5 * 1440),
  last_answer: null,
  outage: {
    ...down.outage,
    node_id: 52,
    name: "物置き1",
    network: "thread",
    started_at: minutes(5 * 1440),
    found_by: "left_fabric",
    reconnects: 0,
    reconnect_result: null,
    reachable: null,
    device_id: 19,
    left_at: minutes(5 * 1440),
    left_by: "event",
    rejoin: {
      attempts: 4,
      attempted_at: minutes(3),
      reason: "no_advert",
      reason_since: minutes(2),
      advert_at: minutes(10),
      rejoined: false,
    },
    leaves: { count: 2, previous_at: minutes(9 * 1440) },
  },
};
const server = {
  ...down.outage,
  node_id: null,
  endpoint: null,
  name: "matterjs-server",
  network: null,
  started_at: minutes(8),
  found_by: "unreachable",
  reconnects: 0,
  reconnect_result: null,
  reachable: null,
};

function watching({ serverDown }: { serverDown: boolean }) {
  return home({
    "GET /api/lights/watch": {
      checked_at: minutes(0),
      server_reachable: !serverDown,
      server_since: serverDown ? server.started_at : minutes(600),
      server_outage: serverDown ? server : null,
      wifi_all_down: false,
      lights: [light, down, thread, left],
    },
    "GET /api/lights/outages": {
      outages: [
        {
          ...down.outage,
          node_id: 5,
          name: "玄関",
          started_at: minutes(300),
          ended_at: minutes(293),
          reconnects: 1,
          reconnect_result: "accepted",
        },
        left.outage,
        down.outage,
      ],
    },
    // What the server lists as asking for the user: matterjs-server has its own banner here.
    "GET /api/attention": {
      items: [
        { kind: "server_down", since: server.started_at },
        {
          kind: "power_cycle",
          name: down.name,
          device_id: null,
          since: down.outage.started_at,
        },
        {
          kind: "left_fabric",
          name: "物置き1",
          device_id: 19,
          left_at: left.outage.left_at,
          left_by: "event",
          reason: "no_advert",
        },
        { kind: "meter_battery", battery: 19 },
      ],
    },
    "GET /api/devices": ledger(),
    "GET /api/devices/*": ledger()[0],
    "GET /api/labels": [],
  });
}

const cards = () => [...document.querySelectorAll<HTMLElement>(".card")];

describe("the light watch", () => {
  it("the light watch says what is down, what asks for the user, and its history", async () => {
    watching({ serverDown: true });
    open("/lights");
    await waitFor(() => expect(cards()).toHaveLength(4));
    const attention = within(
      screen.getByRole("list", { name: "要対応" }),
    ).getAllByRole("link");
    expect(attention).toHaveLength(2);
    expect(attention[0].textContent).toContain(
      "からつながっていません。壁のスイッチで電源を入れ直してください。",
    );
    expect(attention[1].textContent).toContain("物置き1の登録が");

    const text = texts(".card");
    expect(text[0]).toContain("玄関");
    expect(text[0]).toContain("から・応答");
    expect(cards()[0].querySelector(".badge")).toBeNull();
    expect(texts(".badge", cards()[1])).toEqual(["切断"]);
    expect(text[1]).toContain("つなぎ直し3回・網で届かない");
    expect(texts(".badge", cards()[3])).toEqual(["登録が外れた"]);
    expect(cards()[3].textContent?.replace(/\s+/g, "")).toContain(
      `から・自動で登録し直し中・登録を待っていない・4回試した・14日で2回目（前回 ${day(minutes(9 * 1440))} `.replace(
        /\s+/g,
        "",
      ),
    );
    expect(screen.getByRole("alert").textContent).toContain(
      "matterjs-serverに届きません",
    );

    const history = document.querySelector("details.history")!;
    expect(texts("summary", history)).toEqual(["14日の履歴（3件）"]);
    await fireEvent.click(history.querySelector("summary")!);
    // Newest first; the one still going says so.
    const items = texts("li", history);
    expect(items[0]).toContain("続いている");
    expect(items[1]).toContain("登録が外れた");
    expect(items[2]).toContain("（7分）");
  });

  it("a light in the ledger opens its device, and one outside it can be removed", async () => {
    const desk = watching({ serverDown: false });
    desk.answer("DELETE /api/lights/nodes/*", null);
    open("/lights");
    await waitFor(() => expect(cards()).toHaveLength(4));
    expect(cards()[0].getAttribute("href")).toBe("/devices/3");
    expect(cards()[2].getAttribute("href")).toBe("/devices/7");
    expect(cards()[3].getAttribute("href")).toBe("/devices/19");
    // Only the light outside the ledger offers to remove its registration.
    const remove = screen.getAllByRole("button", { name: /の登録を消す$/ });
    expect(remove).toHaveLength(1);
    expect(cards()[1].contains(remove[0])).toBe(true);

    await fireEvent.click(remove[0]);
    const dialog = screen.getByRole("dialog");
    expect(dialog.textContent).toContain(`「${down.name}」を削除`);
    await fireEvent.click(within(dialog).getByRole("button", { name: "削除" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(
      desk.asked.filter((a) => a.method === "DELETE").map((a) => a.path),
    ).toEqual(["/api/lights/nodes/6"]);

    await fireEvent.click(cards()[0]);
    expect(path()).toBe("/devices/3");
  });

  it("the menu leads to the light watch", async () => {
    watching({ serverDown: false });
    open("/");
    await viaMenu("照明の接続");
    expect(title()).toBe("照明の接続");
  });
});
