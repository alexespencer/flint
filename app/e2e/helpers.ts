import { readFileSync } from "node:fs";
import { expect, type Page } from "@playwright/test";
import { mockTauri } from "./mockTauri";

export const note = (name: string) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8");

/** Opens the app on a small fixture store with the rendering note open, its cursor at the end. */
export async function openRenderingNote(page: Page) {
  await mockTauri(page, { storeName: "Notes", notes: { "Rendering.md": note("rendering.md"), "Welcome.md": note("welcome.md") } });
  await page.goto("/");
  await page.getByText("Rendering", { exact: true }).first().click();
  const editor = page.locator(".cm-content");
  await expect(editor).toContainText("Heading 1");
  await editor.click();
  await page.keyboard.press("ControlOrMeta+End");
  // the code languages load on demand: a coloured token (a span inside a code line) means they have
  await expect(page.locator(".cm-codeblock:not(.cm-fence-hidden) span").first()).toBeVisible();
}

export async function chooseTheme(page: Page, theme: "Light" | "Dark") {
  await page.getByRole("button", { name: theme, exact: true }).click();
}
