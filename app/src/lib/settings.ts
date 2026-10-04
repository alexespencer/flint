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
