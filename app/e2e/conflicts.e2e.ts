import { expect, test } from "@playwright/test";
import { mockTauri } from "./mockTauri";

test("the ! beside a conflicted note opens its conflict", async ({ page }) => {
  await mockTauri(page, {
    storeName: "Notes",
    notes: { "todo.md": "# Todo\n\nmine\n", "todo.conflict-1.md": "# Todo\n\ntheirs\n" },
    conflicts: [{ path: "todo.md", copy: "todo.conflict-1.md" }],
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Resolve the conflict in todo.md" }).click();
  const dialog = page.getByRole("dialog", { name: "Resolve conflict" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("Changed on two devices");
  await expect(dialog.getByRole("button", { name: "Mark as resolved" })).toBeVisible();
});
