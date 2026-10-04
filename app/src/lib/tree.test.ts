import { describe, expect, it } from "vitest";
import { buildTree, isNote, title } from "./tree";

describe("buildTree", () => {
  it("nests folders, puts folders first, and sorts ignoring case", () => {
    const tree = buildTree(["b.md", "A/z.md", "a.md", "A/B/deep.md", "C.md"]);
    expect(tree.map((n) => n.name)).toEqual(["A", "a.md", "b.md", "C.md"]);
    const folder = tree[0];
    expect(folder.children!.map((n) => n.name)).toEqual(["B", "z.md"]);
    expect(folder.children![0].children![0].path).toBe("A/B/deep.md");
    expect(folder.path).toBe("A");
  });

  it("copes with no files", () => {
    expect(buildTree([])).toEqual([]);
  });

  it("keeps a file and a folder of the same name apart", () => {
    const tree = buildTree(["x", "x/y.md"]);
    expect(tree.map((n) => [n.name, !!n.children])).toEqual([
      ["x", true],
      ["x", false],
    ]);
  });
});

describe("titles", () => {
  it("drops the folder and the .md", () => {
    expect(title("Locations/Vallaki.md")).toBe("Vallaki");
    expect(title("pic.png")).toBe("pic.png");
    expect(isNote("A.MD")).toBe(true);
    expect(isNote("a.png")).toBe(false);
  });
});
