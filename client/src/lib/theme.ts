export type ThemeChoice = "dark" | "light" | "system";

// Derived projects rename this key to `<project>:theme` (DESIGN.md).
export const THEME_STORAGE_KEY = "home-link:theme";

export function loadTheme(): ThemeChoice {
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (stored === "dark" || stored === "light") {
      return stored;
    }
  } catch {
    // Storage can be unavailable (private mode); treat as system.
  }
  return "system";
}

export function applyTheme(choice: ThemeChoice): void {
  if (choice === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = choice;
  }
}

export function saveTheme(choice: ThemeChoice): void {
  try {
    if (choice === "system") {
      window.localStorage.removeItem(THEME_STORAGE_KEY);
    } else {
      window.localStorage.setItem(THEME_STORAGE_KEY, choice);
    }
  } catch {
    // Persisting is best-effort; still apply for this session.
  }
  applyTheme(choice);
}
