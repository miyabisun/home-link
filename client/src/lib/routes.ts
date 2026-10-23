export interface RouteMatch {
  index: number;
  params: Record<string, string>;
}

interface RouteDef {
  pattern: RegExp;
  params: string[];
}

export const routes: RouteDef[] = [
  { pattern: /^\/$/, params: [] },
  { pattern: /^\/devices\/([^/]+)$/, params: ["id"] },
  { pattern: /^\/labels$/, params: [] },
  { pattern: /^\/labels\/([^/]+)$/, params: ["id"] },
  { pattern: /^\/meter$/, params: [] },
  { pattern: /^\/aircon$/, params: [] },
];

export function matchRoute(pathname: string): RouteMatch {
  for (const [index, route] of routes.entries()) {
    const found = pathname.match(route.pattern);
    if (found) {
      const params: Record<string, string> = {};
      route.params.forEach((name, position) => {
        params[name] = decodeURIComponent(found[position + 1]);
      });
      return { index, params };
    }
  }
  return { index: 0, params: {} };
}
