// Devices that do not answer, and the Thread base pointed to when only Thread devices do not.
// Their layout at each width and theme is tests/page.rs's.
import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Device } from "../lib/api";
import { cleanup, fireEvent, home, open, screen, texts } from "../testing/home";
import { label, ledger } from "../testing/ledger";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const THREAD_DOWN =
  "Threadの機器が応答していません（Threadの基地を確かめてください）";

/** The ledger while its first devices, on the networks `down` gives in order, do not answer. */
function devicesDown(down: string[]): Device[] {
  return ledger().map((device, i) => ({
    ...device,
    network: (down[i] ?? "wifi") as Device["network"],
    state: i < down.length ? "no_response" : device.state,
  }));
}

describe("devices not answering", () => {
  it("devices not answering point to the Thread base when only Thread devices do", async () => {
    const devices = { now: devicesDown(["thread", "thread"]) };
    home({
      "GET /api/devices": () => devices.now,
      "GET /api/attention": { items: [] },
      "GET /api/labels": [label({ id: 1, name: "居間" })],
      // Switching a label names the devices that did not answer.
      "POST /api/labels/1/toggle": {
        on: true,
        switched: 1,
        no_response: 2,
        failed: 0,
        missing: 0,
        devices: [
          { name: "書斎", result: "switched", network: "wifi" },
          { name: "居間の照明", result: "no_response", network: "thread" },
          {
            name: "寝室の天井の照明（窓側）",
            result: "no_response",
            network: "thread",
          },
        ],
      },
    });

    // Only Thread devices are down: the Thread base is pointed to, each device marked.
    open("/");
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent?.trim()).toBe(THREAD_DOWN),
    );
    expect(texts(".card")[0]).toContain("応答なし");
    expect(texts(".card")[1]).toContain("応答なし");

    // A Wi-Fi device is down too: no single cause, only the devices are marked.
    devices.now = devicesDown(["thread", "wifi"]);
    open("/");
    await waitFor(() =>
      expect(document.querySelectorAll(".card")).toHaveLength(6),
    );
    expect(texts(".card")[1]).toContain("応答なし");
    expect(screen.queryByRole("alert")).toBeNull();

    open("/labels/1");
    await fireEvent.click(
      await screen.findByRole("button", { name: "入り切り" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent?.trim()).toBe(
        `応答なし 2（居間の照明・寝室の天井の照明（窓側））。${THREAD_DOWN}`,
      ),
    );
  });
});
