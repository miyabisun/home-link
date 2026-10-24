// Pure rules of the admin screen: what a row shows and what an input sends.
// The server checks the same limits; these keep a bad value from leaving the page.

const MAX_NAME = 100;
const FIXED_DEFAULT = { override_level: 203, override_kelvin: 5000 };

export type Dimming = "none" | "auto" | "fixed";

export interface LabelTraits {
  auto: boolean;
  override_level: number | null;
  override_kelvin: number | null;
}

export interface Switch {
  time: string;
  on: boolean;
}

const length = (text: string) => [...text].length;

export function deviceName(device: {
  name: string;
  product_name: string | null;
  vendor: string | null;
}): string {
  return device.name || device.product_name || device.vendor || "名前なし";
}

export function deviceNameError(name: string): string | null {
  return length(name.trim()) > MAX_NAME ? `${MAX_NAME}文字以内` : null;
}

export function labelNameError(name: string): string | null {
  const n = length(name.trim());
  return n < 1 || n > MAX_NAME ? `1〜${MAX_NAME}文字` : null;
}

export function stateText(state: string | null): string | null {
  if (state === "unconnected") return "未接続";
  if (state === "no_response") return "応答なし";
  return null;
}

/** Matter's brightness runs 1–254; 254 is 100%. */
export function percentOf(level: number): number {
  return Math.round((level * 100) / 254);
}

export function levelOf(percent: number): number {
  return Math.max(1, Math.min(254, Math.round((percent * 254) / 100)));
}

export function dimming(label: LabelTraits): Dimming {
  if (label.override_level !== null) return "fixed";
  return label.auto ? "auto" : "none";
}

/** The server holds auto and an override apart, so a change sends all three. */
export function dimmingChange(mode: Dimming, label: LabelTraits): LabelTraits {
  if (mode === "fixed") {
    return label.override_level !== null && label.override_kelvin !== null
      ? {
          auto: false,
          override_level: label.override_level,
          override_kelvin: label.override_kelvin,
        }
      : { auto: false, ...FIXED_DEFAULT };
  }
  return { auto: mode === "auto", override_level: null, override_kelvin: null };
}

const integerIn = (text: string, min: number, max: number) => {
  const value = Number(text);
  return text.trim() !== "" &&
    Number.isInteger(value) &&
    value >= min &&
    value <= max
    ? value
    : null;
};

export function overrideInput(
  percent: string,
  kelvin: string,
):
  | { patch: { override_level: number; override_kelvin: number } }
  | { field: "percent" | "kelvin"; error: string } {
  const p = integerIn(percent, 1, 100);
  if (p === null) return { field: "percent", error: "明るさは1〜100%" };
  const k = integerIn(kelvin, 1000, 10000);
  if (k === null) return { field: "kelvin", error: "色温度は1000〜10000K" };
  return { patch: { override_level: levelOf(p), override_kelvin: k } };
}

export function labelSummary(label: LabelTraits): string {
  if (label.override_level !== null) {
    return `固定 ${percentOf(label.override_level)}%・${label.override_kelvin}K`;
  }
  return label.auto ? "自動調光" : "";
}

export function scheduleInput(
  rows: Switch[],
): { schedule: Switch[] } | { error: string } {
  if (rows.some((row) => !/^\d\d:\d\d$/.test(row.time)))
    return { error: "時刻を入力" };
  if (new Set(rows.map((row) => row.time)).size !== rows.length) {
    return { error: "同じ時刻があります" };
  }
  const schedule = rows
    .map(({ time, on }) => ({ time, on }))
    .sort((a, b) => a.time.localeCompare(b.time));
  return { schedule };
}

export function withLabel(ids: number[], id: number, on: boolean): number[] {
  if (on) return ids.includes(id) ? ids : [...ids, id];
  return ids.filter((other) => other !== id);
}

export function errorMessage(status: number, body: unknown): string {
  const message = (body as { message?: unknown } | null)?.message;
  return typeof message === "string" && message
    ? message
    : `失敗しました（HTTP ${status}）`;
}

export function switchProblem(result: {
  no_response: number;
  failed: number;
  missing: number;
}): string | null {
  const parts = [
    ["応答なし", result.no_response],
    ["失敗", result.failed],
    ["見つからない", result.missing],
  ]
    .filter(([, count]) => Number(count) > 0)
    .map(([name, count]) => `${name} ${count}`);
  return parts.length ? parts.join("・") : null;
}

export interface MeterView {
  address: string | null;
  reading: object | null;
  received_at: number | null;
}

/** The server withholds a reading once it is too old, but keeps when it came. */
export function meterState(
  meter: MeterView,
): "unset" | "waiting" | "stale" | "fresh" {
  if (meter.address === null) return "unset";
  if (meter.reading !== null) return "fresh";
  return meter.received_at === null ? "waiting" : "stale";
}

const TIME = new Intl.DateTimeFormat("ja-JP", {
  timeZone: "Asia/Tokyo",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hourCycle: "h23",
});
const DATE_TIME = new Intl.DateTimeFormat("ja-JP", {
  timeZone: "Asia/Tokyo",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

/** A fresh reading came minutes ago, so its time is enough; a stale one needs the day. */
export function receivedTime(unix: number, stale: boolean): string {
  return (stale ? DATE_TIME : TIME).format(unix * 1000);
}

export interface AirconDevice {
  state: "connected" | "no_response";
  on: boolean | null;
  mode: string | null;
  setpoint: number | null;
}

const MODES: Record<string, string> = {
  cool: "冷房",
  heat: "暖房",
  dry: "除湿",
  fan_only: "送風",
  auto: "自動",
};

/** The air conditioner sends whole degrees, so the setpoint shows rounded. */
export function airconText(device: AirconDevice | null): string {
  if (device === null) return "未登録";
  if (device.state === "no_response") return "応答なし";
  const mode = device.mode && MODES[device.mode];
  if (!device.on || !mode) return "切";
  if (device.setpoint === null || device.mode === "fan_only") return mode;
  return `${mode} ${Math.round(device.setpoint)}℃`;
}

export const TARGET_MIN = 18;
export const TARGET_MAX = 30;

export function targetError(value: string): string | null {
  const target = Number(value);
  const ok =
    value.trim() !== "" &&
    Number.isInteger(target) &&
    target >= TARGET_MIN &&
    target <= TARGET_MAX;
  return ok ? null : `${TARGET_MIN}〜${TARGET_MAX}℃`;
}

export interface Outdoor {
  temperature: number;
  /** The quarter hour the temperature is for, in Asia/Tokyo. */
  at: string;
  fetched_at: string;
  fresh: boolean;
}

export interface Place {
  name: string | null;
  latitude: number;
  longitude: number;
  registered: boolean;
}

/** A fresh outside temperature to a tenth and its time; nothing once it is two hours old. */
export function outdoorText(
  outdoor: Outdoor | null,
): { celsius: string; time: string } | null {
  if (!outdoor?.fresh) return null;
  return {
    celsius: outdoor.temperature.toFixed(1),
    time: outdoor.at.slice(11, 16),
  };
}

export function placeText(place: Place): string {
  return place.registered && place.name ? place.name : "未登録";
}

/** Minutes after midnight as `HH:MM`, the value of a time input. */
export function nightTime(minute: number): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(Math.floor(minute / 60))}:${pad(minute % 60)}`;
}

export function nightMinute(time: string): number | null {
  const match = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(time);
  return match ? Number(match[1]) * 60 + Number(match[2]) : null;
}
