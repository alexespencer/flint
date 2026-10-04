import { expect, it } from "vitest";
import { wikilinkAt } from "./wikilink";

it("finds the link under the cursor and strips headings and aliases", () => {
  const line = "see [[Plan]] and [[Ideas#Later|my ideas]] ok";
  expect(wikilinkAt(line, 6)).toBe("Plan");
  expect(wikilinkAt(line, 20)).toBe("Ideas");
  expect(wikilinkAt(line, 1)).toBeNull();
  expect(wikilinkAt(line, line.length - 1)).toBeNull();
  expect(wikilinkAt("![[pic.png]]", 4)).toBe("pic.png");
  expect(wikilinkAt("[[#Heading]]", 4)).toBeNull();
});
