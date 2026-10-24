// What the editor shows for each kind of markdown, so a change that breaks one is noticed.
import { expect, test } from "@playwright/test";
import { openRenderingNote } from "./helpers";

test.beforeEach(async ({ page }) => {
  await openRenderingNote(page);
});

test("headings lose their hashes unless the cursor is on them", async ({ page }) => {
  const text = await page.locator(".cm-content").innerText();
  expect(text).toContain("Heading 1");
  expect(text).not.toMatch(/#+ Heading/);
  await page.getByText("Heading 2").click();
  await expect(page.locator(".cm-content")).toContainText("## Heading 2");
});

test("inline code loses its backticks", async ({ page }) => {
  const text = await page.locator(".cm-content").innerText();
  expect(text).toContain("inline code");
  expect(text).not.toContain("`inline code`");
});

test("fenced code is a block with its fences hidden and its language highlighted", async ({ page }) => {
  await expect(page.locator(".cm-codeblock")).not.toHaveCount(0);
  const text = await page.locator(".cm-content").innerText();
  expect(text).not.toContain("```");
  expect(text).toContain("import tqdm");
  await expect(page.locator(".cm-codeblock:not(.cm-fence-hidden) span")).not.toHaveCount(0);
});

test("task lists show checkboxes", async ({ page }) => {
  await expect(page.locator(".cm-task-checkbox")).toHaveCount(2);
  await expect(page.locator(".cm-task-done")).toHaveCount(1);
});

test("a wikilink shows as a link without its brackets", async ({ page }) => {
  await expect(page.locator(".cm-wikilink-rendered")).toHaveText("Welcome");
});
