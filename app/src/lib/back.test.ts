import { describe, expect, it } from "vitest";
import { createBackStack, type WindowLike } from "./back";

/** A browser's history, reduced to what matters: a stack of entries, and popstate when it goes back. */
function fakeWindow() {
  let entries = 1; // the page itself
  let onPop: () => void = () => {};
  const win: WindowLike = {
    history: {
      pushState: () => void entries++,
      back: () => {
        if (entries > 1) {
          entries--;
          onPop();
        }
      },
    },
    addEventListener: (_type, listener) => void (onPop = listener),
  };
  return { win, entries: () => entries, back: () => win.history.back() };
}

describe("back stack", () => {
  it("closes the top layer when Back is pressed, one per press, and leaves the page's own entry", () => {
    const { win, entries, back } = fakeWindow();
    const openLayer = createBackStack(win);
    const closed: string[] = [];
    openLayer(() => closed.push("note"));
    openLayer(() => closed.push("dialog"));
    expect(entries()).toBe(3);
    back();
    expect(closed).toEqual(["dialog"]);
    back();
    expect(closed).toEqual(["dialog", "note"]);
    expect(entries()).toBe(1);
  });

  it("removes the history entry of a layer that closed itself, without closing anything else", () => {
    const { win, entries, back } = fakeWindow();
    const openLayer = createBackStack(win);
    const closed: string[] = [];
    const noteDone = openLayer(() => closed.push("note"));
    openLayer(() => closed.push("dialog"));
    noteDone(); // the note closed by its own button
    expect(entries()).toBe(2);
    expect(closed).toEqual([]);
    back(); // Back now closes the dialog, the only layer left
    expect(closed).toEqual(["dialog"]);
  });

  it("does nothing more when a layer closed by Back reports that it has closed", () => {
    const { win, entries, back } = fakeWindow();
    const openLayer = createBackStack(win);
    const done = openLayer(() => {});
    back();
    done(); // its component unmounts after Back closed it
    expect(entries()).toBe(1);
    back();
    expect(entries()).toBe(1);
  });
});
