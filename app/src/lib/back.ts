/**
 * Makes the Back button (Android's system Back, or the browser's history) close the top layer of the
 * interface first (an open note, a dialog, a menu) instead of leaving the app. Each layer that opens
 * adds one history entry; Back removes it and closes the layer.
 */

export interface HistoryLike {
  pushState(state: unknown, title: string): void;
  back(): void;
}

export interface WindowLike {
  history: HistoryLike;
  addEventListener(type: "popstate", listener: () => void): void;
}

export function createBackStack(win: WindowLike) {
  const layers: { close: () => void }[] = [];
  // closing a layer without Back means taking its history entry away again, which fires popstate once more
  let ignore = 0;

  win.addEventListener("popstate", () => {
    if (ignore > 0) {
      ignore--;
      return;
    }
    layers.pop()?.close();
  });

  /**
   * A layer has opened. `close` runs if the person presses Back. Returns what to call when the layer closes
   * some other way (its own button), which removes its history entry.
   */
  return function openLayer(close: () => void): () => void {
    const layer = { close };
    layers.push(layer);
    win.history.pushState(null, "");
    return () => {
      const at = layers.indexOf(layer);
      if (at < 0) return; // Back already closed it
      layers.splice(at, 1);
      ignore++;
      win.history.back();
    };
  };
}
