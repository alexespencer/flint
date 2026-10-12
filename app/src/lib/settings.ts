/** Settings kept on this device only (in the web view's local storage). */

export const AUTO_LOCK_CHOICES = [
  { minutes: 1, label: "1 minute" },
  { minutes: 5, label: "5 minutes" },
  { minutes: 15, label: "15 minutes" },
  { minutes: 30, label: "30 minutes" },
  { minutes: 60, label: "1 hour" },
  { minutes: 0, label: "Never" },
];

export const DEFAULT_AUTO_LOCK_MINUTES = 15;
const KEY = "flint.autoLockMinutes";

/** A stored value as minutes: 0 means never; anything that is not a whole number of minutes is the default. */
export function parseAutoLockMinutes(raw: string | null): number {
  if (raw === null || raw.trim() === "") return DEFAULT_AUTO_LOCK_MINUTES;
  const minutes = Number(raw);
  return Number.isInteger(minutes) && minutes >= 0 ? minutes : DEFAULT_AUTO_LOCK_MINUTES;
}

export function autoLockMinutes(): number {
  try {
    return parseAutoLockMinutes(localStorage.getItem(KEY));
  } catch {
    return DEFAULT_AUTO_LOCK_MINUTES; // storage can be unavailable; the default still applies
  }
}

export function setAutoLockMinutes(minutes: number) {
  try {
    localStorage.setItem(KEY, String(minutes));
  } catch {
    // not saved: the setting lasts until the app is closed
  }
}

export type Theme = "auto" | "light" | "dark";
export const THEME_CHOICES: { theme: Theme; label: string }[] = [
  { theme: "auto", label: "Match the system" },
  { theme: "light", label: "Light" },
  { theme: "dark", label: "Dark" },
];
const THEME_KEY = "flint.theme";

/** A stored value as a theme: anything unrecognised is auto. */
export function parseTheme(raw: string | null): Theme {
  return raw === "light" || raw === "dark" ? raw : "auto";
}

export function theme(): Theme {
  try {
    return parseTheme(localStorage.getItem(THEME_KEY));
  } catch {
    return "auto";
  }
}

/** Shows the theme now (auto leaves it to the system) and remembers it. */
export function applyTheme(choice: Theme, save = false) {
  if (choice === "auto") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.dataset.theme = choice;
  if (!save) return;
  try {
    localStorage.setItem(THEME_KEY, choice);
  } catch {
    // not saved: the choice lasts until the app is closed
  }
}
