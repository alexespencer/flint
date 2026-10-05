import { createBackStack } from "./back";

/** The app's one back stack, on the real window. */
export const openLayer = createBackStack(window);

/**
 * For a component that is a layer (a dialog, a menu): call it from an effect, `$effect(() => backLayer(close))`.
 * While the component exists, Back closes it; when it goes away some other way, its history entry is removed.
 */
export function backLayer(close: () => void): () => void {
  return openLayer(close);
}
