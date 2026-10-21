import { matchRoute } from "./routes";

const current = $state<{ index: number; params: Record<string, string> }>({
  index: 0,
  params: {},
});

export const router = {
  get index(): number {
    return current.index;
  },
  get params(): Record<string, string> {
    return current.params;
  },
};

export function syncRoute(): void {
  const match = matchRoute(window.location.pathname);
  current.index = match.index;
  current.params = match.params;
}

export function navigate(path: string): void {
  window.history.pushState(null, "", path);
  syncRoute();
  window.scrollTo(0, 0);
}

function onDocumentClick(event: MouseEvent): void {
  if (event.defaultPrevented || event.button !== 0) {
    return;
  }
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
    return;
  }
  const target = event.target as Element | null;
  const anchor = target?.closest<HTMLAnchorElement>("a[href]");
  if (!anchor || anchor.target === "_blank") {
    return;
  }
  const href = anchor.getAttribute("href");
  if (!href || !href.startsWith("/")) {
    return;
  }
  event.preventDefault();
  navigate(href);
}

export function initRouter(): () => void {
  syncRoute();
  window.addEventListener("popstate", syncRoute);
  document.addEventListener("click", onDocumentClick);
  return () => {
    window.removeEventListener("popstate", syncRoute);
    document.removeEventListener("click", onDocumentClick);
  };
}
