import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import Icon, { ICON_NAMES } from "./Icon.svelte";

interface Shape {
  tag: string;
  attrs: Record<string, string>;
}

const STAR_POINTS =
  "12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2";

// Fixed expectations for the entries adopted from the family projects.
// Written out by hand (not derived from the component) so a silent shape
// change cannot re-green this file. `mail` is the canonical Lucide
// geometry, deliberately replacing agent-talkd's smaller custom envelope
// to keep every icon at the same optical size.
const ADOPTED_SHAPES: Record<string, Shape[]> = {
  trash: [
    { tag: "polyline", attrs: { points: "3 6 5 6 21 6" } },
    {
      tag: "path",
      attrs: {
        d: "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2",
      },
    },
    { tag: "line", attrs: { x1: "10", y1: "11", x2: "10", y2: "17" } },
    { tag: "line", attrs: { x1: "14", y1: "11", x2: "14", y2: "17" } },
  ],
  megaphone: [
    { tag: "path", attrs: { d: "m3 11 18-5v12L3 14v-3z" } },
    { tag: "path", attrs: { d: "M11.6 16.8a3 3 0 1 1-5.8-1.6" } },
  ],
  "megaphone-off": [
    { tag: "path", attrs: { d: "M9.26 9.26 3 11v3l14 4v-2.34" } },
    {
      tag: "path",
      attrs: { d: "M21 15V6l-6.5 1.86M11.6 16.8a3 3 0 1 1-5.8-1.6" },
    },
    { tag: "line", attrs: { x1: "2", y1: "2", x2: "22", y2: "22" } },
  ],
  pencil: [
    {
      tag: "path",
      attrs: {
        d: "M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z",
      },
    },
    { tag: "path", attrs: { d: "m15 5 4 4" } },
  ],
  "refresh-cw": [
    {
      tag: "path",
      attrs: { d: "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" },
    },
    { tag: "path", attrs: { d: "M21 3v5h-5" } },
    {
      tag: "path",
      attrs: { d: "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" },
    },
    { tag: "path", attrs: { d: "M8 16H3v5" } },
  ],
  "check-check": [
    { tag: "path", attrs: { d: "M18 6 7 17l-5-5" } },
    { tag: "path", attrs: { d: "m22 10-7.5 7.5L13 16" } },
  ],
  mail: [
    {
      tag: "rect",
      attrs: { x: "2", y: "4", width: "20", height: "16", rx: "2" },
    },
    { tag: "path", attrs: { d: "m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7" } },
  ],
  book: [
    { tag: "path", attrs: { d: "M4 19.5A2.5 2.5 0 0 1 6.5 17H20" } },
    {
      tag: "path",
      attrs: {
        d: "M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z",
      },
    },
  ],
  search: [
    { tag: "circle", attrs: { cx: "11", cy: "11", r: "8" } },
    { tag: "line", attrs: { x1: "21", y1: "21", x2: "16.65", y2: "16.65" } },
  ],
  // star and star-filled share one geometry; only the primitive fill
  // differs. The root svg stays fill="none" for every entry.
  star: [
    {
      tag: "polygon",
      attrs: { points: STAR_POINTS, fill: "none" },
    },
  ],
  "star-filled": [
    {
      tag: "polygon",
      attrs: { points: STAR_POINTS, fill: "currentColor" },
    },
  ],
};

function renderIcon(name: string): SVGSVGElement {
  const { container } = render(Icon, { props: { name } });
  const svg = container.querySelector("svg");
  if (!svg) {
    throw new Error(`icon "${name}" did not render an svg`);
  }
  return svg;
}

describe("Icon", () => {
  afterEach(cleanup);

  it("keeps every dictionary entry on the shared Lucide grammar", () => {
    for (const name of ICON_NAMES) {
      const svg = renderIcon(name);
      expect(svg.getAttribute("viewBox"), name).toBe("0 0 24 24");
      expect(svg.getAttribute("fill"), name).toBe("none");
      expect(svg.getAttribute("stroke"), name).toBe("currentColor");
      expect(svg.getAttribute("stroke-width"), name).toBe("2");
      expect(svg.childElementCount, name).toBeGreaterThan(0);
      cleanup();
    }
  });

  it.each(Object.keys(ADOPTED_SHAPES))(
    "renders the adopted shape for %s verbatim",
    (name) => {
      const svg = renderIcon(name);
      const shapes = ADOPTED_SHAPES[name];
      const children = [...svg.children];
      expect(children).toHaveLength(shapes.length);
      for (const [position, shape] of shapes.entries()) {
        const child = children[position];
        expect(child.tagName.toLowerCase(), `${name}[${position}]`).toBe(
          shape.tag,
        );
        for (const [attr, value] of Object.entries(shape.attrs)) {
          expect(child.getAttribute(attr), `${name}[${position}].${attr}`).toBe(
            value,
          );
        }
      }
    },
  );
});
