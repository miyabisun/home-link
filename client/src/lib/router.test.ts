import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import { initRouter, navigate, router } from "./router.svelte";
import { matchRoute } from "./routes";

describe("matchRoute", () => {
  it("maps / to the home route", () => {
    expect(matchRoute("/")).toEqual({ index: 0, params: {} });
  });

  it("maps the device and label screens with their ids", () => {
    expect(matchRoute("/devices/12")).toEqual({
      index: 1,
      params: { id: "12" },
    });
    expect(matchRoute("/labels")).toEqual({ index: 2, params: {} });
    expect(matchRoute("/labels/3")).toEqual({
      index: 3,
      params: { id: "3" },
    });
  });

  it("normalizes unknown paths to home", () => {
    expect(matchRoute("/no/such/page")).toEqual({ index: 0, params: {} });
  });
});

describe("router", () => {
  afterEach(() => {
    window.history.replaceState(null, "", "/");
  });

  it("navigate() pushes history and updates the route state", () => {
    const teardown = initRouter();

    navigate("/devices/7");

    expect(window.location.pathname).toBe("/devices/7");
    expect(router.index).toBe(1);
    expect(router.params.id).toBe("7");
    teardown();
  });

  it("the browser back button returns to the previous route", async () => {
    const teardown = initRouter();
    window.history.replaceState(null, "", "/");
    navigate("/devices/7");

    window.history.back();

    await waitFor(() => expect(router.index).toBe(0));
    expect(window.location.pathname).toBe("/");
    teardown();
  });

  it("intercepts clicks on internal links", () => {
    const teardown = initRouter();
    const anchor = document.createElement("a");
    anchor.href = "/devices/5";
    anchor.textContent = "card";
    document.body.appendChild(anchor);

    anchor.click();

    expect(window.location.pathname).toBe("/devices/5");
    expect(router.index).toBe(1);
    anchor.remove();
    teardown();
  });
});
