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

/** A device once connected that matterjs-server serves no more (`leftFabric`) left the fabric. */
export function stateText(
  state: string | null,
  leftFabric: boolean | null = false,
): string | null {
  if (leftFabric) return "登録が外れた";
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

export const THREAD_DOWN =
  "Threadの機器が応答していません（Threadの基地を確かめてください）";

/** Only Thread devices fail to answer: the Thread base, not the devices, is the likely fault. */
export function threadDown(
  devices: { state: string | null; network: string | null }[],
): boolean {
  const down = devices.filter((device) => device.state === "no_response");
  return down.length > 0 && down.every((device) => device.network === "thread");
}

export interface SwitchEntry {
  name: string;
  result: string;
  network: string | null;
}

export function switchProblem(result: {
  no_response: number;
  failed: number;
  missing: number;
  devices?: SwitchEntry[];
  missing_devices?: { name: string }[];
}): string | null {
  const names = (outcome: string) => {
    const named = (result.devices ?? [])
      .filter((device) => device.result === outcome)
      .map((device) => device.name);
    return named.length ? `（${named.join("・")}）` : "";
  };
  const parts = [
    ["応答なし", result.no_response, names("no_response")],
    ["失敗", result.failed, names("failed")],
    [
      "登録が外れた",
      result.missing,
      result.missing_devices?.length
        ? `（${result.missing_devices.map((device) => device.name).join("・")}）`
        : "",
    ],
  ]
    .filter(([, count]) => Number(count) > 0)
    .map(([name, count, which]) => `${name} ${count}${which}`);
  if (!parts.length) return null;
  const thread = threadDown(
    (result.devices ?? []).map((device) => ({
      state: device.result,
      network: device.network,
    })),
  );
  return parts.join("・") + (thread ? `。${THREAD_DOWN}` : "");
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

export interface Outage {
  node_id: number | null;
  endpoint: number | null;
  name: string;
  network: "wifi" | "thread" | "ethernet" | null;
  started_at: number;
  ended_at: number | null;
  found_by: string;
  reconnects: number;
  reconnect_result: string | null;
  reachable: boolean | null;
  /** When the light left the fabric, and whether that is matterjs-server's time, a check's, or when it was found. */
  left_at?: number | null;
  left_by?: "event" | "check" | "unknown" | null;
  /** For a light that left: home-link registering it again, and how often it left in two weeks. */
  rejoin?: {
    attempts: number;
    reason: RejoinReason | null;
    rejoined: boolean;
    /** Until when the next try waits, after tries in a row that lost the adapter. */
    slowed_until?: number | null;
  };
  leaves?: { count: number; previous_at: number | null };
}

export type RejoinReason =
  | "no_advert"
  | "connect_failed"
  | "wrong_code"
  | "no_code"
  | "no_bluetooth"
  | "adapter_lost";

const REJOIN_REASONS: Record<RejoinReason, string> = {
  no_advert: "登録を待っていない",
  connect_failed: "つながらない",
  wrong_code: "コードが合わない",
  no_code: "コードがない",
  no_bluetooth: "Bluetoothが使えない",
  adapter_lost: "Bluetoothが途切れた",
};

export interface WatchLight {
  /** `null` for a light that left the fabric: matterjs-server serves no node for it. */
  node_id: number | null;
  endpoint: number | null;
  device_id?: number | null;
  name: string;
  network: "wifi" | "thread" | "ethernet" | null;
  connected: boolean;
  state?: "connected" | "down" | "left_fabric";
  since: number | null;
  last_answer: number | null;
  outage: Outage | null;
}

const HOUR_MINUTE = new Intl.DateTimeFormat("ja-JP", {
  timeZone: "Asia/Tokyo",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});
const DAY = new Intl.DateTimeFormat("ja-JP", {
  timeZone: "Asia/Tokyo",
  month: "numeric",
  day: "numeric",
});

/** The time alone on `now`'s day in Tokyo, else with the day. */
function moment(unix: number, now: number): string {
  const time = HOUR_MINUTE.format(unix * 1000);
  return DAY.format(unix * 1000) === DAY.format(now * 1000)
    ? time
    : `${DAY.format(unix * 1000)} ${time}`;
}

function span(seconds: number): string {
  const minutes = Math.max(0, Math.floor(seconds / 60));
  if (minutes < 60) return `${minutes}分`;
  const hours = `${Math.floor(minutes / 60) % 24}時間${minutes % 60}分`;
  return minutes < 1440 ? hours : `${Math.floor(minutes / 1440)}日${hours}`;
}

/** The badge of a light that needs attention: left the fabric, or down. */
export function watchBadge(light: WatchLight): string | null {
  if (light.state === "left_fabric") return "登録が外れた";
  return light.connected ? null : "切断";
}

/** Since when a light has been connected and when it last answered, or since when it is down and what was done. */
export function watchNote(light: WatchLight, now: number): string {
  const outage = light.outage;
  if (light.state === "left_fabric" && outage?.left_at != null) {
    const at = moment(outage.left_at, now);
    const parts = [
      outage.left_by === "unknown" ? `${at}に発見` : `${at}から`,
      "自動で登録し直し中",
    ];
    const { rejoin, leaves } = outage;
    if (rejoin?.reason) parts.push(REJOIN_REASONS[rejoin.reason]);
    if (rejoin?.attempts) parts.push(`${rejoin.attempts}回試した`);
    if (rejoin?.slowed_until != null)
      parts.push(`次は${moment(rejoin.slowed_until, now)}に試す`);
    if (leaves && leaves.count > 1) {
      const previous =
        leaves.previous_at != null
          ? `（前回 ${moment(leaves.previous_at, now)}）`
          : "";
      parts.push(`14日で${leaves.count}回目${previous}`);
    }
    return parts.join("・");
  }
  const parts = light.since === null ? [] : [`${moment(light.since, now)}から`];
  if (light.connected || outage === null) {
    if (light.last_answer !== null)
      parts.push(`応答 ${moment(light.last_answer, now)}`);
  } else {
    if (outage.reconnects > 0) parts.push(`つなぎ直し${outage.reconnects}回`);
    if (outage.reachable !== null)
      parts.push(outage.reachable ? "網では届く" : "網で届かない");
  }
  return parts.join("・");
}

/** When a stretch out of reach began and ended, and how long it lasted. */
export function outageText(outage: Outage, now: number): string {
  const start = moment(outage.started_at, now);
  const left = outage.left_at != null ? "・登録が外れた" : "";
  if (outage.ended_at === null)
    return `${start}〜（${span(now - outage.started_at)}、続いている）${left}`;
  return `${start}〜${moment(outage.ended_at, now)}（${span(outage.ended_at - outage.started_at)}）${left}`;
}
