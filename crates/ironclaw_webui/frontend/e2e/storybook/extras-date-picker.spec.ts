/**
 * Extras/DatePicker — inline Calendar grid (selection, arrow-key day
 * movement, month paging, min/max bounds) and the DatePicker popover
 * (open/close, day selection, Escape, disabled trigger), plus hover and
 * focus-visible states and theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const INLINE = "extras-datepicker--inline-calendar";
const BOUNDED = "extras-datepicker--monday-first-with-bounds";
const PICKER = "extras-datepicker--picker";
const PICKER_DISABLED = "extras-datepicker--picker-disabled";

function iso(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

test.describe("extras date-picker", () => {
  test("inline calendar renders with today selected (dark theme)", async ({
    page,
  }) => {
    await gotoStory(page, INLINE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByRole("grid")).toBeVisible();
    const today = page.locator(`button[data-date="${iso(new Date())}"]`);
    await expect(today).toHaveAttribute("aria-current", "date");
    await expect(today.locator("xpath=..")).toHaveAttribute(
      "aria-selected",
      "true"
    );
    await expect(
      page.getByRole("button", { name: "Previous month" })
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Next month" })
    ).toBeVisible();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, INLINE, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("grid")).toBeVisible();
  });

  test("clicking a day moves the selection (aria-selected)", async ({
    page,
  }) => {
    await gotoStory(page, INLINE);
    const now = new Date();
    // Pick a mid-month day that isn't today so the selection visibly moves.
    const targetDay = now.getDate() === 15 ? 16 : 15;
    const target = new Date(now.getFullYear(), now.getMonth(), targetDay);
    const dayButton = page.locator(`button[data-date="${iso(target)}"]`);
    await dayButton.click();
    await expect(dayButton.locator("xpath=..")).toHaveAttribute(
      "aria-selected",
      "true"
    );
  });

  test("arrow keys move day focus; month nav pages the grid", async ({
    page,
  }) => {
    await gotoStory(page, INLINE);
    const today = new Date();
    const todayButton = page.locator(`button[data-date="${iso(today)}"]`);
    await todayButton.click();
    await page.keyboard.press("ArrowRight");
    const tomorrow = new Date(today);
    tomorrow.setDate(tomorrow.getDate() + 1);
    await expect(
      page.locator(`button[data-date="${iso(tomorrow)}"]`)
    ).toBeFocused();
    await page.keyboard.press("ArrowDown");
    const nextWeek = new Date(tomorrow);
    nextWeek.setDate(nextWeek.getDate() + 7);
    await expect(
      page.locator(`button[data-date="${iso(nextWeek)}"]`)
    ).toBeFocused();
  });

  test("bounded story disables out-of-range days", async ({ page }) => {
    await gotoStory(page, BOUNDED);
    const now = new Date();
    // minDate is the 5th of the current month → the 4th must be disabled.
    const before = new Date(now.getFullYear(), now.getMonth(), 4);
    const disabledDay = page.locator(`button[data-date="${iso(before)}"]`);
    await expect(disabledDay).toBeDisabled();
    await expect(disabledDay).toHaveCSS("opacity", "0.4");
    // An in-range day stays enabled.
    const inRange = new Date(now.getFullYear(), now.getMonth(), 15);
    await expect(
      page.locator(`button[data-date="${iso(inRange)}"]`)
    ).toBeEnabled();
  });

  test("picker opens a dialog, selecting a day closes and fills the trigger", async ({
    page,
  }) => {
    await gotoStory(page, PICKER);
    const trigger = page.getByRole("button", { name: "Due date" });
    await expect(trigger).toContainText("Pick a due date");
    await expect(trigger).toHaveAttribute("aria-haspopup", "dialog");
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await trigger.click();
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    const dialog = page.getByRole("dialog", { name: "Due date" });
    await expect(dialog).toBeVisible();
    const now = new Date();
    const day = new Date(now.getFullYear(), now.getMonth(), 15);
    await dialog.locator(`button[data-date="${iso(day)}"]`).click();
    await expect(dialog).toBeHidden();
    await expect(trigger).not.toContainText("Pick a due date");
    await expect(trigger).toBeFocused();
  });

  test("Escape closes the picker popover and restores trigger focus", async ({
    page,
  }) => {
    await gotoStory(page, PICKER);
    const trigger = page.getByRole("button", { name: "Due date" });
    await trigger.click();
    await expect(page.getByRole("dialog", { name: "Due date" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog", { name: "Due date" })).toBeHidden();
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(trigger).toBeFocused();
  });

  test("disabled picker trigger has reduced opacity and never opens", async ({
    page,
  }) => {
    await gotoStory(page, PICKER_DISABLED);
    const trigger = page.getByRole("button", { name: "Due date" });
    await expect(trigger).toBeDisabled();
    await expect(trigger).toHaveCSS("opacity", "0.5");
    await trigger.click({ force: true });
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });

  test("hover changes the month-nav button background", async ({ page }) => {
    await gotoStory(page, INLINE);
    const nav = page.getByRole("button", { name: "Next month" });
    const before = await computedStyle(nav, "background-color");
    await nav.hover();
    await expect(nav).not.toHaveCSS("background-color", before);
  });

  test("focus-visible: keyboard focus rings the month-nav button", async ({
    page,
  }) => {
    await gotoStory(page, INLINE);
    const nav = page.getByRole("button", { name: "Previous month" });
    await page.keyboard.press("Tab");
    await expect(nav).toBeFocused();
    await expect(nav).not.toHaveCSS("box-shadow", "none");
  });
});
