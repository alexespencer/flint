import { describe, expect, it } from "vitest";
import { DEFAULT_AUTO_LOCK_MINUTES, parseAutoLockMinutes, parseTheme } from "./settings";

describe("parseAutoLockMinutes", () => {
  it("reads a whole number of minutes, with 0 meaning never", () => {
    expect(parseAutoLockMinutes("5")).toBe(5);
    expect(parseAutoLockMinutes("0")).toBe(0);
    expect(parseAutoLockMinutes("60")).toBe(60);
  });

  it("falls back to the default for anything else", () => {
    for (const bad of [null, "", "  ", "abc", "-3", "2.5", "NaN", "Infinity"]) {
      expect(parseAutoLockMinutes(bad)).toBe(DEFAULT_AUTO_LOCK_MINUTES);
    }
  });
});

describe("parseTheme", () => {
  it("reads light and dark, and treats anything else as auto", () => {
    expect(parseTheme("light")).toBe("light");
    expect(parseTheme("dark")).toBe("dark");
    for (const other of [null, "", "auto", "Dark", "blue"]) expect(parseTheme(other)).toBe("auto");
  });
});
