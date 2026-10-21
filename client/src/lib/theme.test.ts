import { afterEach, describe, expect, it } from "vitest";

import { THEME_STORAGE_KEY, applyTheme, loadTheme, saveTheme } from "./theme";

describe("theme", () => {
  afterEach(() => {
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
  });

  it("defaults to system when nothing is stored", () => {
    expect(loadTheme()).toBe("system");
  });

  it("saving light sets the attribute and persists the choice", () => {
    saveTheme("light");

    expect(document.documentElement.dataset.theme).toBe("light");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
    expect(loadTheme()).toBe("light");
  });

  it("saving dark sets the attribute and persists the choice", () => {
    saveTheme("dark");

    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(loadTheme()).toBe("dark");
  });

  it("saving system removes both the attribute and the stored key", () => {
    saveTheme("light");
    saveTheme("system");

    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
    expect(loadTheme()).toBe("system");
  });

  it("ignores unknown stored values", () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "sepia");

    expect(loadTheme()).toBe("system");
  });

  it("applyTheme alone does not persist", () => {
    applyTheme("light");

    expect(document.documentElement.dataset.theme).toBe("light");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });
});
