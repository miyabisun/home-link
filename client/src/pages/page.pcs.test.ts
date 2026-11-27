// The PCs connected to home-link, put to sleep and woken from their cards. That home-link
// passes the command to the PC and its answer back is tests/pc.rs's; the layout at each width
// and theme is tests/page.rs's.
import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Pc } from "../lib/api";
import {
  cleanup,
  fireEvent,
  home,
  open,
  refused,
  screen,
  texts,
  title,
  viaMenu,
  within,
} from "../testing/home";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** home-link with `pcs` connected; `answer` decides what a PC says to a command. */
function connected(
  pcs: Pc[],
  answer: (pc: Pc, action: string) => unknown = (pc, action) => {
    pc.sleeping = action === "sleep";
    return { status: "accepted", sleeping: pc.sleeping };
  },
) {
  return home({
    "GET /api/pcs": () => pcs,
    "GET /api/devices": [],
    "GET /api/attention": { items: [] },
    "POST /api/pcs/*/*": (_, { path }) => {
      const [, , , name, action] = path.split("/");
      const pc = pcs.find((p) => p.name === decodeURIComponent(name));
      if (!pc)
        return refused(
          404,
          `${decodeURIComponent(name)}はhome-linkにつながっていません`,
        );
      return answer(pc, action);
    },
  });
}

const cards = () => [...document.querySelectorAll<HTMLElement>(".card")];

describe("the PCs", () => {
  it("each PC is listed with what it can be told", async () => {
    // By name, as the server sorts them.
    connected([
      {
        name: "ゲーム用のデスクトップPC（リビングのテレビの横の棚の中）",
        sleeping: true,
      },
      { name: "ノート", sleeping: null },
      { name: "書斎", sleeping: false },
    ]);
    open("/pcs");
    await waitFor(() => expect(cards()).toHaveLength(3));
    expect(cards()[0].textContent).toContain("ゲーム用");
    expect(texts("button", cards()[0])).toEqual(["スリープ中"]);
    expect(cards()[1].textContent).toContain("ノート");
    expect(texts("button", cards()[1])).toEqual(["スリープ", "復帰"]);
    expect(texts("button", cards()[2])).toEqual(["動作中"]);
  });

  it("a PC is put to sleep and woken from its card", async () => {
    const desk = connected([{ name: "書斎", sleeping: false }]);
    open("/");
    await viaMenu("PC");
    expect(title()).toBe("PC");

    const button = await screen.findByRole("button", { name: "スリープ" });
    expect(button.getAttribute("aria-pressed")).toBe("false");
    await fireEvent.click(button);
    await waitFor(() =>
      expect(button.getAttribute("aria-pressed")).toBe("true"),
    );
    expect(button.textContent?.trim()).toBe("スリープ中");
    await fireEvent.click(button);
    await waitFor(() =>
      expect(button.getAttribute("aria-pressed")).toBe("false"),
    );
    expect(button.textContent?.trim()).toBe("動作中");
    expect(
      desk.asked.filter((a) => a.method === "POST").map((a) => a.path),
    ).toEqual([
      `/api/pcs/${encodeURIComponent("書斎")}/sleep`,
      `/api/pcs/${encodeURIComponent("書斎")}/wake`,
    ]);
  });

  it("a PC that refuses, or one that is gone, says so above the list", async () => {
    const pcs: Pc[] = [{ name: "書斎", sleeping: false }];
    connected(pcs, () =>
      refused(
        502,
        "書斎がスリープを受け付けませんでした: モニターがありません",
      ),
    );
    open("/pcs");
    await fireEvent.click(
      await screen.findByRole("button", { name: "スリープ" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent?.trim()).toBe(
        "書斎がスリープを受け付けませんでした: モニターがありません",
      ),
    );
    expect(
      screen
        .getByRole("button", { name: "スリープ" })
        .getAttribute("aria-pressed"),
    ).toBe("false");
    // Gone after the page listed it: the error names it and the list drops it.
    pcs.splice(0, 1);
    await fireEvent.click(screen.getByRole("button", { name: "スリープ" }));
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent?.trim()).toBe(
        "書斎はhome-linkにつながっていません",
      ),
    );
    await waitFor(() =>
      expect(screen.getByText("つながっているPCはありません")).toBeTruthy(),
    );
  });

  it("no PC connected shows that there is none", async () => {
    connected([]);
    open("/pcs");
    expect(
      await screen.findByText("つながっているPCはありません"),
    ).toBeTruthy();
    expect(
      within(document.body).queryAllByRole("button", { name: "スリープ" }),
    ).toEqual([]);
  });
});
