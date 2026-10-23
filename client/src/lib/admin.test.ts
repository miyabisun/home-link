import { describe, expect, it } from "vitest";

import {
  deviceName,
  deviceNameError,
  dimming,
  dimmingChange,
  errorMessage,
  labelNameError,
  labelSummary,
  levelOf,
  meterState,
  overrideInput,
  percentOf,
  receivedTime,
  scheduleInput,
  stateText,
  switchProblem,
  withLabel,
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
      "応答なし 2・見つからない 1",
    );
    expect(switchProblem({ no_response: 0, failed: 1, missing: 0 })).toBe(
      "失敗 1",
    );
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
});
