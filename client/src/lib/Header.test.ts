import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import Header from "./Header.svelte";
import { THEME_STORAGE_KEY } from "./theme";

describe("Header menu and theme flow", () => {
  afterEach(() => {
    cleanup();
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
  });

  it("hamburger opens an anchored dropdown, not a dialog", async () => {
    render(Header);
    const hamburger = screen.getByRole("button", { name: "メニュー" });

    await fireEvent.click(hamburger);

    expect(screen.queryByRole("dialog", { name: "メニュー" })).toBeNull();
    expect(hamburger.getAttribute("aria-expanded")).toBe("true");
    const menu = screen.getByRole("navigation");
    expect(screen.getByRole("banner").contains(menu)).toBe(true);
    const first = menu.querySelector(":scope > :first-child");
    expect(first?.textContent?.trim()).toBe("テーマ設定");
    expect(screen.queryByText("トップ")).toBeNull();
    const labels = screen.getByRole("link", { name: "ラベル" });
    expect(labels.getAttribute("href")).toBe("/labels");
  });

  it("a screen link closes the dropdown", async () => {
    render(Header);
    await fireEvent.click(screen.getByRole("button", { name: "メニュー" }));
    const labels = screen.getByRole("link", { name: "ラベル" });
    labels.addEventListener("click", (event) => event.preventDefault());

    await fireEvent.click(labels);

    expect(screen.queryByRole("navigation")).toBeNull();
  });

  it("the overlay stays out of the tab order", async () => {
    render(Header);
    await fireEvent.click(screen.getByRole("button", { name: "メニュー" }));

    const overlay = screen.getByRole("button", { name: "メニューを閉じる" });
    expect(overlay.getAttribute("tabindex")).toBe("-1");
  });

  it("Escape and outside click close the dropdown and refocus", async () => {
    render(Header);
    const hamburger = screen.getByRole("button", { name: "メニュー" });

    await fireEvent.click(hamburger);
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("navigation")).toBeNull();
    expect(hamburger.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(hamburger);

    await fireEvent.click(hamburger);
    await fireEvent.click(
      screen.getByRole("button", { name: "メニューを閉じる" }),
    );
    expect(screen.queryByRole("navigation")).toBeNull();
  });

  it("テーマ設定 opens the modal and choices apply without closing", async () => {
    render(Header);
    await fireEvent.click(screen.getByRole("button", { name: "メニュー" }));
    await fireEvent.click(screen.getByRole("button", { name: "テーマ設定" }));

    expect(screen.getByRole("dialog", { name: "テーマ設定" })).toBeTruthy();
    expect(screen.getAllByRole("radio")).toHaveLength(3);

    await fireEvent.click(screen.getByRole("radio", { name: "ライト" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
    expect(
      screen
        .getByRole("radio", { name: "ライト" })
        .getAttribute("aria-checked"),
    ).toBe("true");
    expect(screen.getByRole("radiogroup")).toBeTruthy();

    await fireEvent.click(screen.getByRole("radio", { name: "ダーク" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(screen.getByRole("radiogroup")).toBeTruthy();

    await fireEvent.click(screen.getByRole("radio", { name: "自動" }));
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
    expect(screen.getByRole("radiogroup")).toBeTruthy();
  });

  it("Escape closes the theme modal and focus returns", async () => {
    render(Header);
    const hamburger = screen.getByRole("button", { name: "メニュー" });
    await fireEvent.click(hamburger);
    await fireEvent.click(screen.getByRole("button", { name: "テーマ設定" }));

    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("radiogroup")).toBeNull();
    expect(document.activeElement).toBe(hamburger);
  });
});
