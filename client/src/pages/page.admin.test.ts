// The devices, the labels and the air conditioners worked through the screen. What the page
// asks is read back from the stand-in; that home-link keeps it in the ledger is tests/api.rs's.
import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Aircon, Device, Label } from "../lib/api";
import {
  choose,
  cleanup,
  fill,
  fireEvent,
  home,
  open,
  path,
  refused,
  reload,
  screen,
  texts,
  title,
  within,
} from "../testing/home";
import { label, ledger } from "../testing/ledger";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** home-link's labels and devices: what is sent merges into what is listed. */
function ledgerHome(labels: Label[] = [], devices: Device[] = ledger()) {
  const named = (id: number) =>
    labels.find((l) => l.id === id) ?? { id, name: String(id) };
  const desk = home({
    "GET /api/devices": () => devices,
    "GET /api/devices/*": (_, { path }) =>
      devices.find((d) => `/api/devices/${d.id}` === path),
    "GET /api/labels": () => labels,
    "GET /api/attention": { items: [] },
    "PATCH /api/devices/*": (body, { path }) => {
      const device = devices.find((d) => `/api/devices/${d.id}` === path)!;
      const { label_ids, ...rest } = body;
      Object.assign(device, rest);
      if (label_ids)
        device.labels = label_ids.map((id: number) => {
          const { name } = named(id);
          return { id, name };
        });
      return device;
    },
    "POST /api/labels": (body) => {
      const made = label({ id: labels.length + 1, name: body.name });
      labels.push(made);
      return made;
    },
    "PATCH /api/labels/*": (body, { path }) => {
      const found = labels.find((l) => `/api/labels/${l.id}` === path)!;
      return Object.assign(found, body);
    },
    "DELETE /api/labels/*": (_, { path }) => {
      labels.splice(
        labels.findIndex((l) => `/api/labels/${l.id}` === path),
        1,
      );
      return null;
    },
  });
  return desk;
}

const card = (name: RegExp | string) =>
  screen
    .getAllByRole("link")
    .find(
      (link) =>
        link.classList.contains("card") &&
        (typeof name === "string"
          ? link.textContent?.includes(name)
          : name.test(link.textContent ?? "")),
    )!;

describe("the devices and the labels", () => {
  it("the device list shows names and the states that need attention", async () => {
    ledgerHome();
    open("/");
    await waitFor(() =>
      expect(document.querySelectorAll(".card")).toHaveLength(6),
    );
    const cards = texts(".card");
    expect(cards[0]).toContain("居間の照明");
    expect(cards[1]).toContain("P110M");
    expect(cards[2]).toContain("押入れ1");
    expect(cards[2]).toContain("未接続");
    expect(
      document.querySelectorAll(".card")[0].querySelector(".badge"),
    ).toBeNull();
  });

  it("labels are created, renamed, dimmed, fixed and scheduled", async () => {
    const desk = ledgerHome();
    open("/labels");
    const name = await screen.findByRole("textbox", { name: "ラベル名" });

    await fireEvent.click(screen.getByRole("button", { name: "追加" }));
    expect(screen.getByRole("alert").textContent?.trim()).toBe("1〜100文字");
    expect(desk.sent("POST /api/labels")).toEqual([]);

    await fill(name, "ambient");
    await fireEvent.click(screen.getByRole("button", { name: "追加" }));
    await waitFor(() => expect(path()).toBe("/labels/1"));
    await waitFor(() => expect(title()).toBe("ambient"));
    expect(desk.sent("POST /api/labels")).toEqual([{ name: "ambient" }]);

    await fill(screen.getByLabelText("名前"), "常夜灯");
    await waitFor(() => expect(title()).toBe("常夜灯"));
    expect(desk.sent("PATCH /api/labels/1")).toEqual([{ name: "常夜灯" }]);

    await choose(screen.getByLabelText("調光"), "自動調光");
    await waitFor(() =>
      expect(desk.sent("PATCH /api/labels/1").at(-1)).toEqual({
        auto: true,
        override_level: null,
        override_kelvin: null,
      }),
    );

    await choose(screen.getByLabelText("調光"), "固定");
    await waitFor(() =>
      expect(desk.sent("PATCH /api/labels/1").at(-1)).toEqual({
        auto: false,
        override_level: 203,
        override_kelvin: 5000,
      }),
    );
    const percent = await screen.findByLabelText("明るさ（%）");
    await waitFor(() => expect((percent as HTMLInputElement).value).toBe("80"));

    const patches = desk.sent("PATCH /api/labels/1").length;
    await fill(percent, "0");
    expect(screen.getByRole("alert").textContent?.trim()).toBe(
      "明るさは1〜100%",
    );
    expect(percent.getAttribute("aria-invalid")).toBe("true");
    expect(
      screen.getByLabelText("色温度（K）").hasAttribute("aria-invalid"),
    ).toBe(false);
    expect(desk.sent("PATCH /api/labels/1")).toHaveLength(patches);
    await fill(percent, "100");
    await fill(screen.getByLabelText("色温度（K）"), "4000");
    await waitFor(() =>
      expect(desk.sent("PATCH /api/labels/1").at(-1)).toEqual({
        override_level: 254,
        override_kelvin: 4000,
      }),
    );

    const schedule = () => desk.sent("PATCH /api/labels/1").at(-1);
    await fireEvent.click(screen.getByRole("button", { name: "時刻を追加" }));
    await fill(screen.getByLabelText("時刻1"), "07:00");
    await waitFor(() =>
      expect(schedule()).toEqual({ schedule: [{ time: "07:00", on: true }] }),
    );
    await fireEvent.click(screen.getByRole("button", { name: "時刻を追加" }));
    await fill(screen.getByLabelText("時刻2"), "23:30");
    await choose(screen.getByLabelText("操作2"), "切");
    await waitFor(() =>
      expect(schedule()).toEqual({
        schedule: [
          { time: "07:00", on: true },
          { time: "23:30", on: false },
        ],
      }),
    );
    await fireEvent.click(screen.getByRole("button", { name: "時刻1を削除" }));
    await waitFor(() =>
      expect(schedule()).toEqual({ schedule: [{ time: "23:30", on: false }] }),
    );

    reload();
    const dimming = await screen.findByLabelText("調光");
    expect((dimming as HTMLSelectElement).value).toBe("fixed");
    expect((screen.getByLabelText("時刻1") as HTMLInputElement).value).toBe(
      "23:30",
    );
    expect(desk.strays).toEqual([]);
  });

  it("a label assigns devices and switches them", async () => {
    const night = label({ id: 1, name: "常夜灯" });
    const desk = ledgerHome([night]);
    desk.answer("POST /api/labels/1/toggle", (body) => {
      // Whatever it is now, the label goes the other way from what the page read.
      night.on = !(body?.from ?? night.on);
      return {
        on: night.on,
        no_response: 0,
        failed: 0,
        missing: 0,
        devices: [],
        missing_devices: [],
      };
    });
    open("/labels/1");
    await fireEvent.click(
      await screen.findByRole("checkbox", { name: "居間の照明" }),
    );
    await waitFor(() =>
      expect(desk.sent("PATCH /api/devices/1")).toEqual([{ label_ids: [1] }]),
    );

    const toggle = screen.getByRole("button", { name: "入り切り" });
    await fireEvent.click(toggle);
    await waitFor(() =>
      expect(toggle.getAttribute("aria-pressed")).toBe("true"),
    );
    expect(desk.sent("POST /api/labels/1/toggle")).toEqual([undefined]);

    // Switched off elsewhere, the button still reads on: pressed, it goes the way it reads.
    night.on = false;
    expect(toggle.textContent?.trim()).toBe("入");
    await fireEvent.click(toggle);
    await waitFor(() => expect(toggle.textContent?.trim()).toBe("切"));
    expect(desk.sent("POST /api/labels/1/toggle").at(-1)).toEqual({
      from: true,
    });

    await fireEvent.click(screen.getByRole("checkbox", { name: "居間の照明" }));
    await waitFor(() =>
      expect(desk.sent("PATCH /api/devices/1").at(-1)).toEqual({
        label_ids: [],
      }),
    );
  });

  it("a device is renamed, labelled, ignored, switched and deleted", async () => {
    const devices = ledger();
    const desk = ledgerHome([label({ id: 1, name: "常夜灯" })], devices);
    desk.answer("POST /api/devices/2/on", () => {
      devices[1].on = true;
      return { state: "on" };
    });
    desk.answer("DELETE /api/devices/1", () => {
      devices.splice(0, 1);
      return null;
    });
    open("/devices/1");
    await fill(await screen.findByLabelText("名前"), "リビング");
    await waitFor(() => expect(title()).toBe("リビング"));
    expect(desk.sent("PATCH /api/devices/1")).toEqual([{ name: "リビング" }]);

    await fireEvent.click(screen.getByRole("checkbox", { name: "常夜灯" }));
    await waitFor(() =>
      expect(desk.sent("PATCH /api/devices/1").at(-1)).toEqual({
        label_ids: [1],
      }),
    );
    await fireEvent.click(screen.getByRole("checkbox", { name: "無視" }));
    await waitFor(() =>
      expect(desk.sent("PATCH /api/devices/1").at(-1)).toEqual({
        ignored: true,
      }),
    );

    open("/");
    await waitFor(() => expect(card(/P110M/)).toBeDefined());
    await fireEvent.click(card(/P110M/));
    const plug = await screen.findByRole("button", { name: "入り切り" });
    expect(plug.textContent?.trim()).toBe("切");
    await fireEvent.click(plug);
    await waitFor(() => expect(plug.getAttribute("aria-pressed")).toBe("true"));
    expect(plug.textContent?.trim()).toBe("入");
    open("/");
    await waitFor(() =>
      expect(card(/P110M/)?.querySelector(".badge")?.textContent).toBe("入"),
    );

    open("/devices/1");
    await fireEvent.click(await screen.findByRole("button", { name: "削除" }));
    await fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "キャンセル",
      }),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(desk.sent("DELETE /api/devices/1")).toEqual([]);
    await fireEvent.click(screen.getByRole("button", { name: "削除" }));
    await fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "削除" }),
    );
    await waitFor(() => expect(path()).toBe("/"));
    await waitFor(() =>
      expect(document.querySelectorAll(".card")).toHaveLength(5),
    );
    expect(desk.sent("DELETE /api/devices/1")).toHaveLength(1);
    expect(desk.strays).toEqual([]);
  });

  it("labels keep their names unique and are deleted", async () => {
    const desk = ledgerHome([label({ id: 1, name: "常夜灯" })]);
    desk.answer("POST /api/labels", refused(409, "同じ名前のlabelがあります"));
    open("/labels");
    await fill(
      await screen.findByRole("textbox", { name: "ラベル名" }),
      "常夜灯",
    );
    await fireEvent.click(screen.getByRole("button", { name: "追加" }));
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent?.trim()).toBe(
        "同じ名前のlabelがあります",
      ),
    );

    await fireEvent.click(card("常夜灯"));
    await fireEvent.click(await screen.findByRole("button", { name: "削除" }));
    await fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "削除" }),
    );
    await waitFor(() => expect(path()).toBe("/labels"));
    expect(desk.asked.filter((a) => a.method === "DELETE")).toEqual([
      { method: "DELETE", path: "/api/labels/1", body: undefined },
    ]);
  });
});

describe("the air conditioners", () => {
  const unit = {
    id: 5,
    name: "エアコン",
    state: "connected",
    on: true,
    mode: "cool",
    setpoint: 25,
    sensor: null,
    reading: null,
    reason: null,
  } as Aircon["aircons"][number];
  const nowhere = {
    name: null,
    latitude: 35.6812,
    longitude: 139.7671,
    registered: false,
  };
  const shinjuku = {
    name: "東京都新宿区西新宿二丁目８番",
    latitude: 35.689629,
    longitude: 139.691772,
    registered: true,
  };
  const values = () => texts(".reading dd");

  it("the air conditioner follows the sensor under its label and is switched on its own page", async () => {
    const devices = ledger();
    const desk = ledgerHome([], devices);
    let aircon: Aircon = {
      aircons: [{ ...unit }],
      outdoor: null,
      location: nowhere,
    };
    desk.answer("GET /api/aircon", () => aircon);
    desk.answer("GET /api/location", { ...nowhere, address: null, pin: null });
    desk.answer("POST /api/devices/5/*", (_, { path }) => {
      devices[4].on = path.endsWith("/on");
      return { state: devices[4].on ? "on" : "off" };
    });
    open("/");
    await viaMenuTo("エアコン");
    await waitFor(() => expect(title()).toBe("エアコン"));
    // No sensor under a label of the air conditioner's: no room, the air conditioner as it is.
    await waitFor(() => expect(values()).toEqual(["—", "—", "冷房 25℃"]));

    aircon = {
      ...aircon,
      aircons: [
        {
          ...unit,
          sensor: { id: 6, name: "寝室・温湿度計" },
          reading: { temperature: 28.3, humidity: 52 },
        },
      ],
    };
    reload();
    await waitFor(() => expect(values().slice(0, 2)).toEqual(["28.3℃", "52%"]));
    expect(texts(".reading dt")[0]).toBe("室温寝室・温湿度計");

    // Its name leads to its own page, where it is switched as a plug is.
    await fireEvent.click(
      within(document.querySelector("section.aircon")!).getByRole("link", {
        name: "エアコン",
      }),
    );
    expect(path()).toBe("/devices/5");
    const power = await screen.findByRole("button", { name: "入り切り" });
    expect(power.getAttribute("aria-pressed")).toBe("true");
    await fireEvent.click(power);
    await waitFor(() => expect(power.textContent?.trim()).toBe("切"));
    expect(desk.asked.at(-1)).toMatchObject({ path: "/api/devices/5/off" });
    aircon = { ...aircon, aircons: [{ ...aircon.aircons[0], on: false }] };
    open("/aircon");
    await waitFor(() => expect(values()[2]).toBe("切"));

    open("/devices/5");
    const again = await screen.findByRole("button", { name: "入り切り" });
    expect(again.textContent?.trim()).toBe("切");
    await fireEvent.click(again);
    await waitFor(() => expect(again.textContent?.trim()).toBe("入"));
    expect(desk.asked.at(-1)).toMatchObject({ path: "/api/devices/5/on" });
  });

  it("a device is switched in each of its modes on its own page", async () => {
    const devices = ledger();
    const desk = ledgerHome([], devices);
    desk.answer("POST /api/devices/5/mode", (body) => {
      devices[4].modes!.current = body.mode;
      return { mode: body.mode };
    });
    desk.answer("POST /api/devices/5/*", (_, { path }) => {
      devices[4].on = path.endsWith("/on");
      return { state: devices[4].on ? "on" : "off" };
    });
    const asked = () =>
      desk.asked
        .filter((a) => a.method === "POST")
        .map(
          (a) =>
            `${a.path.replace("/api/devices/5/", "")}${a.body ? ` ${JSON.stringify(a.body)}` : ""}`,
        );
    const badge = (mode: string) =>
      [...document.querySelectorAll(".modes li")]
        .find((li) => li.textContent?.includes(mode))
        ?.querySelector(".badge")?.textContent ?? null;
    open("/devices/5");
    const power = await screen.findByRole("button", { name: "入り切り" });
    expect(power.textContent?.trim()).toBe("入");
    expect(badge("HITACHI_AC424")).toBe("使用中");

    await fireEvent.click(
      screen.getByRole("button", { name: "HITACHI_AC344で切" }),
    );
    await waitFor(() => expect(power.textContent?.trim()).toBe("切"));
    expect(badge("HITACHI_AC344")).toBe("使用中");
    expect(badge("HITACHI_AC424")).toBeNull();
    expect(asked()).toEqual(['mode {"mode":1}', "off"]);
    // Already off: it is switched on and off again, and stays off.
    await fireEvent.click(
      screen.getByRole("button", { name: "HITACHI_AC344で切" }),
    );
    await waitFor(() =>
      expect((power as HTMLButtonElement).disabled).toBe(false),
    );
    expect(asked().slice(2)).toEqual(["on", "off"]);
    reload();
    const reread = await screen.findByRole("button", { name: "入り切り" });
    expect(reread.textContent?.trim()).toBe("切");
    expect(badge("HITACHI_AC344")).toBe("使用中");

    await fireEvent.click(
      screen.getByRole("button", { name: "HITACHI_AC424で入" }),
    );
    await waitFor(() => expect(reread.textContent?.trim()).toBe("入"));
    expect(badge("HITACHI_AC424")).toBe("使用中");
    expect(asked().slice(4)).toEqual(['mode {"mode":0}', "on"]);
  });

  it("the home's address is registered and the outside shows with the reason", async () => {
    const desk = ledgerHome();
    let aircon: Aircon = {
      aircons: [{ ...unit }],
      outdoor: null,
      location: nowhere,
    };
    let address: string | null = null;
    desk.answer("GET /api/aircon", () => aircon);
    desk.answer("GET /api/location", () => ({
      ...aircon.location,
      address,
      pin: null,
    }));
    desk.answer("PUT /api/location", (body) => {
      if (body.address.includes("見つからない"))
        return refused(422, "住所が見つかりませんでした。前の位置のままです");
      address = body.address;
      aircon = { ...aircon, location: shinjuku };
      return { ...shinjuku, address, pin: null };
    });
    open("/aircon");
    // Unregistered, the settings are open.
    await waitFor(() =>
      expect(document.querySelector("#aircon-place")).not.toBeNull(),
    );
    const place = () =>
      document.querySelector("#aircon-place")?.textContent ?? "";
    expect(place()).toContain("未登録");
    expect(
      document.querySelector("details.settings")?.hasAttribute("open"),
    ).toBe(true);
    const field = screen.getByLabelText("住所");

    await fill(field, "見つからない町1-1");
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent?.trim()).toBe(
        "住所が見つかりませんでした。前の位置のままです",
      ),
    );
    expect(place()).toContain("未登録");

    await fill(field, "東京都新宿区西新宿2-8-1");
    await waitFor(() =>
      expect(place()).toContain("東京都新宿区西新宿二丁目８番"),
    );
    expect(place()).toContain("35.6896, 139.6918");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(desk.sent("PUT /api/location")).toEqual([
      { address: "見つからない町1-1" },
      { address: "東京都新宿区西新宿2-8-1" },
    ]);

    // A run says what it was outside and why.
    aircon = {
      ...aircon,
      outdoor: {
        temperature: 30.2,
        at: "2026-10-24T16:15:00+09:00",
        fetched_at: "2026-10-24T16:20:03+09:00",
        fresh: true,
      },
      aircons: [
        {
          ...unit,
          sensor: { id: 6, name: "寝室・温湿度計" },
          reading: { temperature: 28.3, humidity: 52 },
          reason: "室温が28.3℃で26℃を超えているが、冷房25℃で運転中",
        },
      ],
    };
    reload();
    await waitFor(() =>
      expect(texts(".outdoor")).toEqual(["外気温 30.2℃ 16:15"]),
    );
    expect(texts(".reason")).toEqual([
      "室温が28.3℃で26℃を超えているが、冷房25℃で運転中",
    ]);

    // Once registered, a reload shows the settings closed, with the address in them.
    const details = document.querySelector("details.settings")!;
    expect(details.hasAttribute("open")).toBe(false);
    await fireEvent.click(screen.getByText("設定"));
    expect(details.hasAttribute("open")).toBe(true);
    await waitFor(() =>
      expect((screen.getByLabelText("住所") as HTMLInputElement).value).toBe(
        "東京都新宿区西新宿2-8-1",
      ),
    );
  });
});

/** Follows the header menu's link named `name`. */
async function viaMenuTo(name: string) {
  await fireEvent.click(
    await screen.findByRole("button", { name: "メニュー" }),
  );
  await fireEvent.click(
    within(document.querySelector("nav.menu") as HTMLElement).getByRole(
      "link",
      {
        name,
      },
    ),
  );
}
