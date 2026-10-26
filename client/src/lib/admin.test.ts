import { describe, expect, it } from "vitest";

import {
  airconText,
  nightMinute,
  nightTime,
  outdoorText,
  placeText,
  deviceName,
  deviceNameError,
  dimming,
  dimmingChange,
  errorMessage,
  labelNameError,
  labelSummary,
  levelOf,
  meterState,
  adapterText,
  targetError,
  overrideInput,
  percentOf,
  receivedTime,
  scheduleInput,
  stateText,
  outageText,
  watchBadge,
  watchNote,
  switchProblem,
  withLabel,
  THREAD_DOWN,
  threadDown,
} from "./admin";

const label = {
  auto: false,
  override_level: null,
  override_kelvin: null,
};

describe("device names", () => {
  it("falls back to what the device reports, then to a placeholder", () => {
    expect(
      deviceName({ name: "寝室", product_name: "Bulb", vendor: "Tapo" }),
    ).toBe("寝室");
    expect(deviceName({ name: "", product_name: "Bulb", vendor: "Tapo" })).toBe(
      "Bulb",
    );
    expect(deviceName({ name: "", product_name: null, vendor: "Tapo" })).toBe(
      "Tapo",
    );
    expect(deviceName({ name: "", product_name: null, vendor: null })).toBe(
      "名前なし",
    );
  });

  it("allows an empty name and at most 100 characters", () => {
    expect(deviceNameError("")).toBeNull();
    expect(deviceNameError(" 机 ")).toBeNull();
    expect(deviceNameError("あ".repeat(100))).toBeNull();
    expect(deviceNameError("あ".repeat(101))).toBe("100文字以内");
  });
});

describe("label names", () => {
  it("need 1 to 100 characters after trimming", () => {
    expect(labelNameError("work")).toBeNull();
    expect(labelNameError("  ")).toBe("1〜100文字");
    expect(labelNameError("𠮷".repeat(100))).toBeNull();
    expect(labelNameError("a".repeat(101))).toBe("1〜100文字");
  });
});

describe("device states", () => {
  it("names only the states that need attention", () => {
    expect(stateText("connected")).toBeNull();
    expect(stateText(null)).toBeNull();
    expect(stateText("unconnected")).toBe("未接続");
    expect(stateText("no_response")).toBe("応答なし");
  });

  it("tells a device that left the fabric from one never connected", () => {
    expect(stateText("unconnected", true)).toBe("登録が外れた");
    expect(stateText("unconnected", false)).toBe("未接続");
    expect(stateText(null, null)).toBeNull();
  });
});

describe("brightness", () => {
  it("maps 254 to 100% both ways", () => {
    expect(percentOf(254)).toBe(100);
    expect(percentOf(203)).toBe(80);
    expect(percentOf(1)).toBe(0);
    expect(levelOf(100)).toBe(254);
    expect(levelOf(80)).toBe(203);
    expect(levelOf(1)).toBe(3);
  });
});

describe("dimming", () => {
  it("is one of none, auto and fixed", () => {
    expect(dimming(label)).toBe("none");
    expect(dimming({ ...label, auto: true })).toBe("auto");
    expect(
      dimming({ ...label, override_level: 254, override_kelvin: 5000 }),
    ).toBe("fixed");
  });

  it("changes as one patch the server accepts", () => {
    const fixed = { ...label, override_level: 254, override_kelvin: 4000 };
    expect(dimmingChange("auto", fixed)).toEqual({
      auto: true,
      override_level: null,
      override_kelvin: null,
    });
    expect(dimmingChange("none", { ...label, auto: true })).toEqual({
      auto: false,
      override_level: null,
      override_kelvin: null,
    });
    expect(dimmingChange("fixed", label)).toEqual({
      auto: false,
      override_level: 203,
      override_kelvin: 5000,
    });
    expect(dimmingChange("fixed", fixed)).toEqual({
      auto: false,
      override_level: 254,
      override_kelvin: 4000,
    });
  });

  it("checks the fixed values before sending them", () => {
    expect(overrideInput("100", "5000")).toEqual({
      patch: { override_level: 254, override_kelvin: 5000 },
    });
    const percent = { field: "percent", error: "明るさは1〜100%" };
    const kelvin = { field: "kelvin", error: "色温度は1000〜10000K" };
    expect(overrideInput("0", "5000")).toEqual(percent);
    expect(overrideInput("50.5", "5000")).toEqual(percent);
    expect(overrideInput("", "5000")).toEqual(percent);
    expect(overrideInput("80", "999")).toEqual(kelvin);
    expect(overrideInput("80", "10001")).toEqual(kelvin);
  });

  it("summarises a label in its list row", () => {
    expect(labelSummary(label)).toBe("");
    expect(labelSummary({ ...label, auto: true })).toBe("自動調光");
    expect(
      labelSummary({ ...label, override_level: 203, override_kelvin: 5000 }),
    ).toBe("固定 80%・5000K");
  });
});

describe("schedules", () => {
  it("sends the rows sorted by time", () => {
    expect(
      scheduleInput([
        { time: "23:30", on: false },
        { time: "07:00", on: true },
      ]),
    ).toEqual({
      schedule: [
        { time: "07:00", on: true },
        { time: "23:30", on: false },
      ],
    });
    expect(scheduleInput([])).toEqual({ schedule: [] });
  });

  it("rejects an empty or repeated time", () => {
    expect(scheduleInput([{ time: "", on: true }])).toEqual({
      error: "時刻を入力",
    });
    expect(
      scheduleInput([
        { time: "07:00", on: true },
        { time: "07:00", on: false },
      ]),
    ).toEqual({ error: "同じ時刻があります" });
  });
});

describe("label assignment", () => {
  it("adds or removes one label and keeps the others", () => {
    expect(withLabel([1, 3], 2, true)).toEqual([1, 3, 2]);
    expect(withLabel([1, 3], 3, true)).toEqual([1, 3]);
    expect(withLabel([1, 3], 1, false)).toEqual([3]);
  });
});

describe("errors", () => {
  it("shows the server's message, or the status when there is none", () => {
    expect(
      errorMessage(409, {
        error: "duplicate_label_name",
        message: "同じ名前のlabelがあります",
      }),
    ).toBe("同じ名前のlabelがあります");
    expect(errorMessage(502, null)).toBe("失敗しました（HTTP 502）");
  });
});

describe("switch results", () => {
  it("names the devices that did not switch, or nothing when all did", () => {
    expect(switchProblem({ no_response: 0, failed: 0, missing: 0 })).toBeNull();
    expect(switchProblem({ no_response: 2, failed: 0, missing: 1 })).toBe(
      "応答なし 2・登録が外れた 1",
    );
    expect(switchProblem({ no_response: 0, failed: 1, missing: 0 })).toBe(
      "失敗 1",
    );
  });

  const entry = (name: string, result: string, network: string | null) => ({
    name,
    result,
    network,
  });

  it("names each device that did not answer", () => {
    expect(
      switchProblem({
        no_response: 2,
        failed: 1,
        missing: 0,
        devices: [
          entry("読書灯", "switched", "wifi"),
          entry("台所", "no_response", "wifi"),
          entry("寝室", "no_response", "thread"),
          entry("玄関", "failed", "wifi"),
        ],
      }),
    ).toBe("応答なし 2（台所・寝室）・失敗 1（玄関）");
  });

  it("names each device that left the fabric", () => {
    expect(
      switchProblem({
        no_response: 0,
        failed: 0,
        missing: 1,
        missing_devices: [{ name: "物置き1" }],
      }),
    ).toBe("登録が外れた 1（物置き1）");
  });

  it("points to the Thread base when only Thread devices did not answer", () => {
    expect(
      switchProblem({
        no_response: 2,
        failed: 0,
        missing: 0,
        devices: [
          entry("読書灯", "switched", "wifi"),
          entry("台所", "no_response", "thread"),
          entry("寝室", "no_response", "thread"),
        ],
      }),
    ).toBe(`応答なし 2（台所・寝室）。${THREAD_DOWN}`);
  });
});

describe("the Thread base", () => {
  const device = (state: string, network: string | null) => ({
    state,
    network,
  });

  it("is down when every device not answering is on Thread", () => {
    expect(
      threadDown([
        device("connected", "wifi"),
        device("no_response", "thread"),
        device("no_response", "thread"),
      ]),
    ).toBe(true);
  });

  it("is not blamed when nothing is down or a device elsewhere is down too", () => {
    expect(
      threadDown([device("connected", "wifi"), device("connected", "thread")]),
    ).toBe(false);
    expect(
      threadDown([
        device("no_response", "wifi"),
        device("no_response", "thread"),
      ]),
    ).toBe(false);
    expect(threadDown([device("no_response", null)])).toBe(false);
  });
});

describe("the meter", () => {
  const reading = { temperature: 26.7, humidity: 57, co2: 717, battery: 74 };
  const address = "AA:BB:CC:DD:EE:01";

  it("tells unset, waiting, stale and fresh apart", () => {
    expect(
      meterState({ address: null, reading: null, received_at: null }),
    ).toBe("unset");
    expect(meterState({ address, reading: null, received_at: null })).toBe(
      "waiting",
    );
    expect(meterState({ address, reading: null, received_at: 1 })).toBe(
      "stale",
    );
    expect(meterState({ address, reading, received_at: 1 })).toBe("fresh");
  });

  it("shows the time of a fresh reading and the date of a stale one, in Tokyo", () => {
    // 2026-10-17 18:55:07 in Tokyo.
    const unix = Date.UTC(2026, 9, 17, 9, 55, 7) / 1000;
    expect(receivedTime(unix, false)).toBe("18:55:07");
    expect(receivedTime(unix, true)).toBe("10/17 18:55");
  });

  it("names what is wrong with the adapter, and nothing when it discovers", () => {
    const adapter = (
      present: boolean,
      powered: boolean,
      discovering: boolean,
    ) => ({
      adapter: { present, powered, discovering },
    });
    expect(adapterText({ adapter: null })).toBe("Bluetoothに届きません");
    expect(adapterText(adapter(false, false, false))).toBe(
      "アダプタがありません",
    );
    expect(adapterText(adapter(true, false, false))).toBe(
      "アダプタの電源が入っていません",
    );
    expect(adapterText(adapter(true, true, false))).toBe(
      "アダプタが探索していません",
    );
    expect(adapterText(adapter(true, true, true))).toBeNull();
  });
});

describe("the air conditioner", () => {
  const device = {
    state: "connected" as const,
    on: true,
    mode: "cool",
    setpoint: 25,
  };

  it("names its mode and setpoint while on", () => {
    expect(airconText(device)).toBe("冷房 25℃");
    expect(airconText({ ...device, mode: "heat", setpoint: 22 })).toBe(
      "暖房 22℃",
    );
    expect(airconText({ ...device, mode: "dry", setpoint: 24.5 })).toBe(
      "除湿 25℃",
    );
    expect(airconText({ ...device, mode: "fan_only" })).toBe("送風");
    expect(airconText({ ...device, mode: "auto", setpoint: null })).toBe(
      "自動",
    );
  });

  it("is off, unanswered or absent", () => {
    expect(airconText({ ...device, on: false })).toBe("切");
    expect(airconText({ ...device, mode: "off" })).toBe("切");
    expect(airconText({ ...device, state: "no_response" })).toBe("応答なし");
    expect(airconText(null)).toBe("未登録");
  });

  it("takes a whole target from 18 to 30", () => {
    for (const ok of ["18", "25", "30"]) expect(targetError(ok)).toBeNull();
    for (const bad of ["17", "31", "24.5", "", "a"])
      expect(targetError(bad)).toBe("18〜30℃");
  });
});

describe("the outside and the night", () => {
  it("shows a fresh outside temperature with its time, and nothing stale", () => {
    const outdoor = {
      temperature: 30.24,
      at: "2026-10-24T16:15:00+09:00",
      fetched_at: "2026-10-24T16:20:03+09:00",
      fresh: true,
    };
    expect(outdoorText(outdoor)).toEqual({ celsius: "30.2", time: "16:15" });
    expect(outdoorText({ ...outdoor, fresh: false })).toBeNull();
    expect(outdoorText(null)).toBeNull();
  });

  it("names the registered place, or says it is not registered", () => {
    const place = { latitude: 35.6895, longitude: 139.6917 };
    expect(
      placeText({
        ...place,
        name: "東京都新宿区西新宿二丁目８番",
        registered: true,
      }),
    ).toBe("東京都新宿区西新宿二丁目８番");
    expect(placeText({ ...place, name: null, registered: false })).toBe(
      "未登録",
    );
  });

  it("turns the night's minutes into times and back", () => {
    expect(nightTime(1320)).toBe("22:00");
    expect(nightTime(420)).toBe("07:00");
    expect(nightTime(5)).toBe("00:05");
    expect(nightMinute("22:00")).toBe(1320);
    expect(nightMinute("00:05")).toBe(5);
    expect(nightMinute("23:59")).toBe(1439);
    for (const bad of ["", "24:00", "7:00", "aa:bb"])
      expect(nightMinute(bad)).toBeNull();
  });
});

describe("the light watch", () => {
  // 2026-10-26 00:00 in Tokyo.
  const T0 = 1_792_940_400;
  const light = {
    node_id: 5 as number | null,
    endpoint: 1 as number | null,
    device_id: 1,
    name: "玄関",
    network: "wifi" as "wifi" | null,
    connected: true,
    state: "connected" as const,
    since: T0 + 8 * 60,
    last_answer: T0 + 20 * 60,
    outage: null,
  };
  const outage = {
    node_id: 5,
    endpoint: 1,
    name: "玄関",
    network: "wifi" as const,
    started_at: T0 + 14 * 60,
    ended_at: T0 + 21 * 60,
    found_by: "unavailable",
    reconnects: 1,
    reconnect_result: "accepted",
    reachable: false,
    left_at: null as number | null,
    left_by: null as "event" | "check" | "unknown" | null,
  };

  it("tells a connected light since when and its last answer", () => {
    const now = T0 + 21 * 60;
    expect(watchNote(light, now)).toBe("00:08から・応答 00:20");
    expect(watchNote({ ...light, last_answer: null }, now)).toBe("00:08から");
    expect(watchNote({ ...light, since: T0 - 86_400 }, now)).toBe(
      "10/25 00:00から・応答 00:20",
    );
  });

  it("tells a light that is down since when, how often reconnected and its reach", () => {
    const down = {
      ...light,
      connected: false,
      since: T0 + 14 * 60,
      outage: { ...outage, ended_at: null },
    };
    const now = T0 + 30 * 60;
    expect(watchNote(down, now)).toBe("00:14から・つなぎ直し1回・網で届かない");
    expect(
      watchNote(
        { ...down, outage: { ...down.outage, reconnects: 0, reachable: null } },
        now,
      ),
    ).toBe("00:14から");
    expect(
      watchNote({ ...down, outage: { ...down.outage, reachable: true } }, now),
    ).toBe("00:14から・つなぎ直し1回・網では届く");
  });

  it("tells a stretch out of reach by when and how long", () => {
    const now = T0 + 30 * 60;
    expect(outageText(outage, now)).toBe("00:14〜00:21（7分）");
    expect(outageText({ ...outage, ended_at: null }, now)).toBe(
      "00:14〜（16分、続いている）",
    );
    expect(
      outageText(
        { ...outage, started_at: T0 - 3 * 3600, ended_at: T0 + 5 * 60 },
        now,
      ),
    ).toBe("10/25 21:00〜00:05（3時間5分）");
  });

  it("tells a light that left the fabric since when and to register it again", () => {
    const now = T0 + 30 * 60;
    const left = {
      ...light,
      node_id: null,
      endpoint: null,
      network: null,
      connected: false,
      state: "left_fabric" as const,
      since: T0 - 5 * 86_400 + (9 * 60 + 17) * 60,
      outage: {
        ...outage,
        found_by: "left_fabric",
        started_at: T0 - 5 * 86_400 + (9 * 60 + 17) * 60,
        ended_at: null,
        reconnects: 0,
        reachable: null,
        left_at: T0 - 5 * 86_400 + (9 * 60 + 17) * 60,
        left_by: "event" as const,
      },
    };
    expect(watchNote(left, now)).toBe("10/21 09:17から・自動で登録し直し中");
    expect(
      watchNote(
        {
          ...left,
          outage: { ...left.outage, left_at: T0 + 14 * 60, left_by: "unknown" },
        },
        now,
      ),
    ).toBe("00:14に発見・自動で登録し直し中");
    expect(
      watchNote(
        {
          ...left,
          outage: {
            ...left.outage,
            rejoin: {
              attempts: 3,
              reason: "wrong_code" as const,
              rejoined: false,
            },
            leaves: { count: 2, previous_at: T0 - 86_400 },
          },
        },
        now,
      ),
    ).toBe(
      "10/21 09:17から・自動で登録し直し中・コードが合わない・3回試した・14日で2回目（前回 10/25 00:00）",
    );
    expect(
      watchNote(
        {
          ...left,
          outage: {
            ...left.outage,
            rejoin: {
              attempts: 0,
              reason: "no_advert" as const,
              rejoined: false,
            },
          },
        },
        now,
      ),
    ).toBe("10/21 09:17から・自動で登録し直し中・登録を待っていない");
    expect(
      watchNote(
        {
          ...left,
          outage: {
            ...left.outage,
            rejoin: {
              attempts: 2,
              reason: "adapter_lost" as const,
              rejoined: false,
              slowed_until: now + 25 * 60,
            },
          },
        },
        now,
      ),
    ).toBe(
      "10/21 09:17から・自動で登録し直し中・Bluetoothが途切れた・2回試した・次は00:55に試す",
    );
    expect(watchBadge(left)).toBe("登録が外れた");
    expect(watchBadge({ ...light, connected: false, state: "down" })).toBe(
      "切断",
    );
    expect(watchBadge(light)).toBeNull();
    expect(outageText(left.outage, now)).toBe(
      "10/21 09:17〜（4日15時間13分、続いている）・登録が外れた",
    );
  });
});
