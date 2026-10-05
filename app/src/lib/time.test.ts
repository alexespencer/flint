import { expect, it } from "vitest";
import { ago } from "./time";

it("describes how long ago in a few words", () => {
  const now = 1_000_000_000;
  const at = (seconds: number) => ago(now - seconds * 1000, now);
  expect(at(0)).toBe("just now");
  expect(at(44)).toBe("just now");
  expect(at(60)).toBe("1 min ago");
  expect(at(5 * 60)).toBe("5 min ago");
  expect(at(59 * 60)).toBe("59 min ago");
  expect(at(2 * 3600)).toBe("2 h ago");
  expect(at(3 * 86400)).toBe("3 d ago");
  expect(ago(now + 5000, now)).toBe("just now"); // a clock a little ahead is not "in the future"
});
