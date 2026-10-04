import { describe, expect, it } from "vitest";
import { baseName, buildTree, isConflictCopy, isNote, isWithin, moveTargets, parentOf, title } from "./tree";

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

describe("folders", () => {
  it("shows empty folders, and merges them with the folders of files", () => {
    const tree = buildTree(["a/x.md"], ["a", "a/empty", "b/c", "b"]);
    expect(tree.map((n) => n.name)).toEqual(["a", "b"]);
    expect(tree[0].children!.map((n) => [n.name, !!n.children])).toEqual([
      ["empty", true],
      ["x.md", false],
    ]);
    expect(tree[1].children![0].path).toBe("b/c");
    expect(buildTree([], ["only"])).toEqual([{ name: "only", path: "only", children: [] }]);
  });

  it("tells what is inside what", () => {
    expect(isWithin("a/b.md", "a")).toBe(true);
    expect(isWithin("a", "a")).toBe(true);
    expect(isWithin("ab/c.md", "a")).toBe(false);
    expect(baseName("a/b/c.md")).toBe("c.md");
    expect(baseName("c.md")).toBe("c.md");
  });
});

describe("moving", () => {
  const folders = [
    { path: "old", createdMs: 100 },
    { path: "new", createdMs: 300 },
    { path: "new/inner", createdMs: 400 },
    { path: "mid", createdMs: 200 },
  ];

  it("finds the folder a path is in", () => {
    expect(parentOf("a/b/c.md")).toBe("a/b");
    expect(parentOf("c.md")).toBe("");
  });

  it("offers the newest folders first and the top level last, but not where the note already is", () => {
    expect(moveTargets("c.md", folders)).toEqual(["new/inner", "new", "mid", "old"]);
    expect(moveTargets("old/c.md", folders)).toEqual(["new/inner", "new", "mid", ""]);
  });

  it("never offers a folder itself or anything inside it", () => {
    expect(moveTargets("new", folders)).toEqual(["mid", "old"]);
    expect(moveTargets("a/new", folders)).toEqual(["new/inner", "new", "mid", "old", ""]);
  });

  it("breaks ties by name, so the order is stable", () => {
    const same = [
      { path: "b", createdMs: 1 },
      { path: "a", createdMs: 1 },
    ];
    expect(moveTargets("x.md", same)).toEqual(["a", "b"]);
  });
});

describe("conflict copies", () => {
  it("are recognised by their name", () => {
    expect(isConflictCopy("Test.conflict-Mac-20261004-1730.md")).toBe(true);
    expect(isConflictCopy("folder/Test.conflict-Phone-20261004-1730-2.md")).toBe(true);
    expect(isConflictCopy("conflict-notes.md")).toBe(false);
    expect(isConflictCopy("a.conflict/b.md")).toBe(false);
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
