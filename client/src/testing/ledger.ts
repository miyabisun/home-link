// The ledger the page tests start from, as GET /api/devices lists it with a light, a plug, an
// air conditioner and a climate sensor connected, one device never connected and one with a
// long name.
import type { Device, Label } from "../lib/api";

const device = (fields: Partial<Device> & Pick<Device, "id" | "name">) =>
  ({
    vendor: null,
    product_name: null,
    state: "connected",
    kind: null,
    on: null,
    climate: null,
    modes: null,
    wifi_ssid: null,
    network: "wifi",
    labels: [],
    ignored: false,
    left_fabric: false,
    left_at: null,
    ...fields,
  }) as Device;

export const LONG_NAME =
  "寝室の天井の照明（窓側）と書斎の机の上のスタンドライトをまとめて呼ぶための長い名前";

export function ledger(): Device[] {
  return [
    device({
      id: 1,
      name: "居間の照明",
      vendor: "Tapo",
      product_name: "Smart Bulb",
      kind: "light",
    }),
    device({
      id: 2,
      name: "",
      vendor: "Tapo",
      product_name: "P110M",
      kind: "plug",
      on: false,
    }),
    device({ id: 3, name: "押入れ1", state: "unconnected", network: null }),
    device({ id: 4, name: LONG_NAME, vendor: "Tapo", state: null }),
    device({
      id: 5,
      name: "エアコン",
      vendor: "Airco",
      product_name: "Room Air Conditioner",
      kind: "aircon",
      on: true,
      modes: {
        current: 0,
        options: [
          { mode: 0, label: "HITACHI_AC424" },
          { mode: 1, label: "HITACHI_AC344" },
        ],
      },
    }),
    device({
      id: 6,
      name: "寝室・温湿度計",
      vendor: "Aqara",
      product_name: "Aqara Climate Sensor W100",
      kind: "sensor",
      on: true,
      climate: { temperature: 28.3, humidity: 52, battery: 90 },
    }),
  ];
}

export function label(fields: Partial<Label> & Pick<Label, "id" | "name">) {
  return {
    auto: false,
    override_level: null,
    override_kelvin: null,
    on: null,
    schedule: [],
    device_count: 0,
    ...fields,
  } as Label;
}
