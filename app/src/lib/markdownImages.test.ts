/** What the editor relies on from the markdown parser to draw `![alt](picture.png)`: an Image node with link marks and a URL. */
import { syntaxTree } from "@codemirror/language";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { EditorState } from "@codemirror/state";
import { expect, it } from "vitest";

it("finds an image's description and address as parts of the Image node", () => {
  const doc = "text ![a map](Images/map%20one.png) more";
  const state = EditorState.create({ doc, extensions: [markdown({ base: markdownLanguage })] });
  const found: { alt: string; url: string }[] = [];
  syntaxTree(state).iterate({
    enter(node) {
      if (node.name !== "Image") return;
      const marks = node.node.getChildren("LinkMark");
      const url = node.node.getChild("URL")!;
      found.push({ alt: state.sliceDoc(marks[0].to, marks[1].from), url: state.sliceDoc(url.from, url.to) });
    },
  });
  expect(found).toEqual([{ alt: "a map", url: "Images/map%20one.png" }]);
});

it("gives a wikilink embed no address, so the editor's own handling of `![[...]]` is the only one that draws it", () => {
  const state = EditorState.create({ doc: "![[Pasted image 1.png]]", extensions: [markdown({ base: markdownLanguage })] });
  const addresses: (string | null)[] = [];
  syntaxTree(state).iterate({
    enter(node) {
      if (node.name === "Image") {
        const url = node.node.getChild("URL");
        addresses.push(url ? state.sliceDoc(url.from, url.to) : null);
      }
    },
  });
  expect(addresses.every((address) => address === null)).toBe(true);
});
