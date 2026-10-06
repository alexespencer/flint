/**
 * Task lists are the markdown library's job (the GitHub-flavoured mode parses `- [ ] item`, and its Enter key continues a
 * checklist); the editor only draws a checkbox where the parser finds a task marker. These tests pin down what we rely on
 * from the library, so an upgrade that changes it is noticed.
 */
import { syntaxTree } from "@codemirror/language";
import { indentLess, indentMore } from "@codemirror/commands";
import { indentUnit } from "@codemirror/language";
import { insertNewlineContinueMarkup, markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";

const extensions = [markdown({ base: markdownLanguage })];

function markers(doc: string): [number, number][] {
  const found: [number, number][] = [];
  syntaxTree(EditorState.create({ doc, extensions })).iterate({
    enter: (node) => void (node.name === "TaskMarker" && found.push([node.from, node.to])),
  });
  return found;
}

function enter(doc: string, at = doc.length): string {
  let state = EditorState.create({ doc, selection: { anchor: at }, extensions });
  insertNewlineContinueMarkup({ state, dispatch: (tr) => (state = tr.state) });
  return state.doc.toString();
}

describe("task markers, as the editor's markdown mode parses them", () => {
  it("are found for the forms Obsidian shows as checkboxes, as soon as the space after the box is typed", () => {
    expect(markers("- [ ] a")).toEqual([[2, 5]]);
    expect(markers("- [x] b")).toEqual([[2, 5]]);
    expect(markers("* [X] c")).toEqual([[2, 5]]);
    expect(markers("+ [ ] d")).toEqual([[2, 5]]);
    expect(markers("  - [ ] nested")).toEqual([[4, 7]]);
    expect(markers("- [ ] ")).toEqual([[2, 5]]);
  });

  it("are not found for text that merely looks similar", () => {
    for (const doc of ["- [ ]", "[ ] bare", "- [] no", "-[ ] tight", "- [ ]x", "- [  ] two", "- plain"]) {
      expect(markers(doc), doc).toEqual([]);
    }
  });
});

describe("Enter in a checklist", () => {
  it("starts the next item with an empty box, whether or not the last was ticked", () => {
    expect(enter("- [ ] buy milk")).toBe("- [ ] buy milk\n- [ ] ");
    expect(enter("- [x] done")).toBe("- [x] done\n- [ ] ");
    expect(enter("  - [ ] nested")).toBe("  - [ ] nested\n  - [ ] ");
  });

  it("splits an item at the cursor, and an empty item ends the list", () => {
    expect(enter("- [ ] split here", 8)).toBe("- [ ] sp\n- [ ] lit here");
    expect(enter("- [ ] ")).toBe("");
  });
});

describe("Tab in a checklist (the editor binds Tab to indentMore and Shift+Tab to indentLess, four spaces)", () => {
  function run(command: typeof indentMore, doc: string): string {
    let state = EditorState.create({ doc, selection: { anchor: doc.length }, extensions: [...extensions, indentUnit.of("    ")] });
    command({ state, dispatch: (tr) => (state = tr.state) });
    return state.doc.toString();
  }

  it("nests an item under the one above, and un-nests it again", () => {
    expect(run(indentMore, "- [ ] one\n- [ ] two")).toBe("- [ ] one\n    - [ ] two");
    expect(run(indentLess, "- [ ] one\n    - [ ] two")).toBe("- [ ] one\n- [ ] two");
  });
});
