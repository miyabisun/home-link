import {
  type AirconDevice,
  type Attention,
  errorMessage,
  type LabelTraits,
  type MeterView,
  type AdapterView,
  type Outage,
  type Outdoor,
  type Place,
  type Switch,
  type SwitchEntry,
  type WatchLight,
} from "./admin";

export interface Watch {
  checked_at: number | null;
  server_reachable: boolean | null;
  server_since: number | null;
  server_outage: Outage | null;
  wifi_all_down: boolean;
  lights: WatchLight[];
}

export interface LabelRef {
  id: number;
  name: string;
}

export interface Device {
  id: number;
  name: string;
  vendor: string | null;
  product_name: string | null;
  state: "unconnected" | "connected" | "no_response" | null;
  kind: "light" | "plug" | "aircon" | "other" | null;
  on: boolean | null;
  wifi_ssid: string | null;
  network: "wifi" | "thread" | "ethernet" | null;
  labels: LabelRef[];
  ignored: boolean;
  /** Once connected and served no more: it left the fabric. `null` when unknown. */
  left_fabric?: boolean | null;
  left_at?: number | null;
}

export interface Label extends LabelTraits {
  id: number;
  name: string;
  on: boolean | null;
  schedule: Switch[];
  device_count: number;
}

export interface Meter extends MeterView, AdapterView {
  powered_on_at: number | null;
  power_cycled_at: number | null;
  reading: {
    temperature: number;
    humidity: number;
    co2: number;
    battery: number;
  } | null;
}

export interface Aircon {
  on: boolean;
  target: number;
  /** The night in minutes after midnight. */
  night_from: number;
  night_until: number;
  device: (AirconDevice & { id: number; name: string }) | null;
  reading: Meter["reading"];
  outdoor: Outdoor | null;
  location: Place;
  /** Why the latest run did what it did. */
  reason: string | null;
}

export interface Location extends Place {
  address: string | null;
}

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

async function request<T>(
  method: string,
  url: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(url, {
    method,
    signal,
    headers:
      body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  const json: unknown = text ? JSON.parse(text) : null;
  if (!response.ok) {
    throw new ApiError(errorMessage(response.status, json), response.status);
  }
  return json as T;
}

export const api = {
  devices: (signal?: AbortSignal) =>
    request<Device[]>("GET", "/api/devices", undefined, signal),
  device: (id: string, signal?: AbortSignal) =>
    request<Device>(
      "GET",
      `/api/devices/${encodeURIComponent(id)}`,
      undefined,
      signal,
    ),
  updateDevice: (
    id: number,
    patch: Partial<{ name: string; label_ids: number[]; ignored: boolean }>,
  ) => request<Device>("PATCH", `/api/devices/${id}`, patch),
  deleteDevice: (id: number) => request<null>("DELETE", `/api/devices/${id}`),
  switchDevice: (id: number, on: boolean) =>
    request<{ state: string }>(
      "POST",
      `/api/devices/${id}/${on ? "on" : "off"}`,
    ),
  labels: (signal?: AbortSignal) =>
    request<Label[]>("GET", "/api/labels", undefined, signal),
  createLabel: (name: string) =>
    request<Label>("POST", "/api/labels", { name }),
  updateLabel: (
    id: number,
    patch: Partial<LabelTraits & { name: string; schedule: Switch[] }>,
  ) => request<Label>("PATCH", `/api/labels/${id}`, patch),
  deleteLabel: (id: number) => request<null>("DELETE", `/api/labels/${id}`),
  meter: (signal?: AbortSignal) =>
    request<Meter>("GET", "/api/meter", undefined, signal),
  aircon: (signal?: AbortSignal) =>
    request<Aircon>("GET", "/api/aircon", undefined, signal),
  updateAircon: (
    patch: Partial<{
      on: boolean;
      target: number;
      night_from: number;
      night_until: number;
    }>,
  ) => request<Aircon>("PUT", "/api/aircon", patch),
  location: (signal?: AbortSignal) =>
    request<Location>("GET", "/api/location", undefined, signal),
  updateLocation: (address: string) =>
    request<Location>("PUT", "/api/location", { address }),
  toggleLabel: (id: number) =>
    request<{
      on: boolean | null;
      no_response: number;
      failed: number;
      missing: number;
      devices: SwitchEntry[];
      missing_devices: { id: number; name: string }[];
    }>("POST", `/api/labels/${id}/toggle`),
  watch: (signal?: AbortSignal) =>
    request<Watch>("GET", "/api/lights/watch", undefined, signal),
  attention: (signal?: AbortSignal) =>
    request<{ items: Attention[] }>("GET", "/api/attention", undefined, signal),
  outages: (signal?: AbortSignal) =>
    request<{ outages: Outage[] }>(
      "GET",
      "/api/lights/outages",
      undefined,
      signal,
    ),
};

export const aborted = (error: unknown) =>
  error instanceof DOMException && error.name === "AbortError";

export const missing = (error: unknown) =>
  error instanceof ApiError && error.status === 404;
