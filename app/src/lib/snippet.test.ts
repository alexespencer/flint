import { expect, it } from "vitest";
import { pieces } from "./snippet";

it("splits highlighted matches out of a snippet", () => {
  expect(pieces("a \u0001b\u0002 c")).toEqual([
    { text: "a ", match: false },
    { text: "b", match: true },
    { text: " c", match: false },
  ]);
  expect(pieces("")).toEqual([]);
  expect(pieces("plain")).toEqual([{ text: "plain", match: false }]);
});
