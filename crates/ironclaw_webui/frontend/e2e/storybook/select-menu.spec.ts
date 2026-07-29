import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

test.describe("SelectMenu", () => {
  test("renders closed with the selected value (dark)", async ({ page }) => {
    await gotoStory(page, "components-selectmenu--default");
    const trigger = page.getByRole("button", { name: "Status" });
    await expect(trigger).toBeVisible();
    await expect(trigger).toContainText("Running");
    await expect(trigger).toHaveAttribute("aria-haspopup", "listbox");
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(page.getByRole("listbox")).toBeHidden();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("renders in light theme", async ({ page }) => {
    await gotoStory(page, "components-selectmenu--default", { theme: "light" });
    await expect(page.getByRole("button", { name: "Status" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("opens on click and selects an option with the mouse", async ({ page }) => {
    await gotoStory(page, "components-selectmenu--default");
    const trigger = page.getByRole("button", { name: "Status" });
    await trigger.click();
    const listbox = page.getByRole("listbox");
    await expect(listbox).toBeVisible();
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await expect(listbox.getByRole("option")).toHaveCount(4);
    await expect(listbox.getByRole("option", { name: "Running" })).toHaveAttribute(
      "aria-selected",
      "true"
    );
    await expect(listbox.getByRole("option", { name: "Archived" })).toBeDisabled();
    await expect(listbox.getByRole("option", { name: "Archived" })).toHaveAttribute(
      "aria-disabled",
      "true"
    );

    await listbox.getByRole("option", { name: "Paused" }).click();
    await expect(listbox).toBeHidden();
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(trigger).toContainText("Paused");
  });

  test("full keyboard flow: open, arrows, Enter selects, Escape closes", async ({ page }) => {
    await gotoStory(page, "components-selectmenu--default");
    const trigger = page.getByRole("button", { name: "Status" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await expect(trigger).not.toHaveCSS("box-shadow", "none");

    await page.keyboard.press("Enter");
    const listbox = page.getByRole("listbox");
    await expect(listbox).toBeVisible();
    // Active option starts on the selected value (index 0).
    await expect(trigger).toHaveAttribute("aria-activedescendant", /-option-0$/);
    await page.keyboard.press("ArrowDown");
    await expect(trigger).toHaveAttribute("aria-activedescendant", /-option-1$/);
    await page.keyboard.press("ArrowDown");
    await expect(trigger).toHaveAttribute("aria-activedescendant", /-option-2$/);
    // Index 3 is disabled, so ArrowDown wraps back to the first option.
    await page.keyboard.press("ArrowDown");
    await expect(trigger).toHaveAttribute("aria-activedescendant", /-option-0$/);
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect(listbox).toBeHidden();
    await expect(trigger).toContainText("Paused");
    await expect(trigger).toBeFocused();

    await page.keyboard.press("Space");
    await expect(listbox).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(listbox).toBeHidden();
    await expect(trigger).toBeFocused();
  });

  test("closes on outside click", async ({ page }) => {
    await gotoStory(page, "components-selectmenu--default");
    await page.getByRole("button", { name: "Status" }).click();
    const listbox = page.getByRole("listbox");
    await expect(listbox).toBeVisible();
    await page.locator("body").click({ position: { x: 5, y: 5 } });
    await expect(listbox).toBeHidden();
  });

  test("trigger background changes on hover", async ({ page }) => {
    await gotoStory(page, "components-selectmenu--default");
    const trigger = page.getByRole("button", { name: "Status" });
    const before = await computedStyle(trigger, "background-color");
    await trigger.hover();
    await expect(trigger).not.toHaveCSS("background-color", before);
  });

  test("disabled story blocks interaction with dimmed styles", async ({ page }) => {
    await gotoStory(page, "components-selectmenu--disabled");
    const trigger = page.getByRole("button", { name: "Status" });
    await expect(trigger).toBeDisabled();
    await expect(trigger).toHaveCSS("opacity", "0.5");
    await expect(trigger).toHaveCSS("pointer-events", "none");
    await expect(page.getByRole("listbox")).toBeHidden();
  });

  test("left-aligned story anchors the menu to the left edge", async ({ page }) => {
    await gotoStory(page, "components-selectmenu--left-aligned");
    await page.getByRole("button", { name: "Status" }).click();
    const listbox = page.getByRole("listbox");
    await expect(listbox).toBeVisible();
    await expect(listbox).toHaveCSS("left", "0px");
  });
});
