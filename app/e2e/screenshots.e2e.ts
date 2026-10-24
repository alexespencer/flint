// Regenerates the README's pictures: `bun run screenshots` (they land in ../docs/img).
import { test } from "@playwright/test";
import { chooseTheme, openRenderingNote } from "./helpers";

const out = (name: string) => `../docs/img/${name}.png`;

test("a note, light", async ({ page }) => {
  await openRenderingNote(page);
  await chooseTheme(page, "Light");
  await page.screenshot({ path: out("note-light") });
});

test("a note, dark", async ({ page }) => {
  await openRenderingNote(page);
  await chooseTheme(page, "Dark");
  await page.screenshot({ path: out("note-dark") });
});

test("settings", async ({ page }) => {
  await openRenderingNote(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.locator(".dialog").screenshot({ path: out("settings") });
});
