// The page tests: the whole admin screen in jsdom against a stand-in for home-link's API.
// The stand-in keeps no rules of the server's: each test says what each request answers,
// and reads back what the page asked. That the server does what the page asks is
// tests/api.rs's to show.
import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/svelte";
import { vi } from "vitest";

import App from "../App.svelte";

export interface Asked {
  method: string;
  path: string;
  body: unknown;
}

/** What a request answers: a value sent as JSON, a Response, or a function of the request. */
export type Answer =
  | object
  | string
  | number
  | boolean
  | null
  | ((body: any, asked: Asked) => unknown);

/** JSON, as home-link sends it; `null` is an empty body. */
export function json(value: unknown, status = 200): Response {
  if (value === null || status === 204)
    return new Response(null, { status: status === 200 ? 204 : status });
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** A refusal as home-link words one. */
export const refused = (status: number, message: string) =>
  json({ error: "refused", message }, status);

/**
 * Answers the page's requests from `answers`, keyed by `METHOD /path`; a `*` in the path
 * stands for one segment. A request no key names answers 404 and is kept in `strays`.
 */
export function home(answers: Record<string, Answer>) {
  const table: Record<string, Answer> = { ...answers };
  const asked: Asked[] = [];
  const strays: string[] = [];
  // jsdom has no scrolling; the router scrolls to the top on each page.
  vi.stubGlobal("scrollTo", () => {});
  const find = (key: string): Answer | undefined => {
    if (key in table) return table[key];
    const pattern = Object.keys(table).find((k) =>
      new RegExp(
        `^${k.replace(/[.?+]/g, "\\$&").replace(/\*/g, "[^/]+")}$`,
      ).test(key),
    );
    return pattern === undefined ? undefined : table[pattern];
  };
  vi.stubGlobal(
    "fetch",
    async (input: RequestInfo | URL, init: RequestInit = {}) => {
      const url = new URL(String(input), window.location.origin);
      const method = init.method ?? "GET";
      const body =
        typeof init.body === "string" ? JSON.parse(init.body) : undefined;
      const one = { method, path: url.pathname, body };
      asked.push(one);
      const key = `${method} ${url.pathname}`;
      const answer = find(key);
      if (answer === undefined) {
        strays.push(key);
        return json({ error: "not_found", message: "Not Found" }, 404);
      }
      const reply =
        typeof answer === "function" ? await answer(body, one) : answer;
      if (init.signal?.aborted)
        throw new DOMException("The operation was aborted.", "AbortError");
      return reply instanceof Response ? reply : json(reply);
    },
  );
  return {
    asked,
    strays,
    /** Has `key` answer `answer` from now on. */
    answer(key: string, answer: Answer) {
      table[key] = answer;
    },
    /** The bodies the page sent with `METHOD /path`, in order. */
    sent(key: string): unknown[] {
      return asked
        .filter((a) => `${a.method} ${a.path}` === key)
        .map((a) => a.body);
    },
  };
}

/** Opens the admin screen at `path`. */
export function open(path: string) {
  cleanup();
  window.history.pushState(null, "", path);
  return render(App);
}

/** Reads the page again where it is, as a reload does. */
export function reload() {
  cleanup();
  return render(App);
}

/** Types `value` into a field and leaves it, as a person does with a Tab. */
export async function fill(field: HTMLElement, value: string) {
  (field as HTMLInputElement).value = value;
  await fireEvent.input(field);
  await fireEvent.change(field);
}

/** Picks the option whose text is `text`. */
export async function choose(select: HTMLElement, text: string) {
  const option = within(select).getByRole("option", { name: text });
  (select as HTMLSelectElement).value = (option as HTMLOptionElement).value;
  await fireEvent.change(select);
}

/** The page's own path. */
export const path = () => window.location.pathname;

/** The heading under the header. */
export const title = () =>
  document.querySelector(".sub-header h1")?.textContent?.trim() ??
  document.querySelector(".sub-header")?.textContent?.trim();

/** The text of each element `selector` finds in `root`, trimmed and with its spaces joined. */
export const texts = (selector: string, root: ParentNode = document) =>
  [...root.querySelectorAll(selector)].map((el) =>
    (el.textContent ?? "").replace(/\s+/g, " ").trim(),
  );

/** Opens the menu and follows its link named `name`. */
export async function viaMenu(name: string) {
  await fireEvent.click(
    screen.getByRole("button", { name: "メニュー", exact: true } as never),
  );
  await fireEvent.click(screen.getByRole("link", { name }));
}

export { cleanup, fireEvent, screen, within };
