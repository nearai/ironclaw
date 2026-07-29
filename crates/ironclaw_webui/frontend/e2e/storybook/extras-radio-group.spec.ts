import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT_STORY = "extras-radiogroup--default";
const DISABLED_STORY = "extras-radiogroup--disabled";

test.describe("extras/radio-group", () => {
  test("renders a radiogroup with the default option checked", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(
      page.getByRole("radiogroup", { name: "Run mode" })
    ).toBeVisible();
    await expect(page.getByRole("radio")).toHaveCount(3);

    const balanced = page.getByRole("radio", { name: "Balanced" });
    await expect(balanced).toHaveAttribute("aria-checked", "true");
    await expect(balanced).toHaveAttribute("data-state", "checked");
    await expect(
      page.getByRole("radio", { name: "Fast — lower quality" })
    ).toHaveAttribute("aria-checked", "false");
  });

  test("clicking another option moves the checked state", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const fast = page.getByRole("radio", { name: "Fast — lower quality" });
    await fast.click();
    await expect(fast).toHaveAttribute("data-state", "checked");
    await expect(
      page.getByRole("radio", { name: "Balanced" })
    ).toHaveAttribute("data-state", "unchecked");
  });

  test("arrow keys move the checked state between items", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const balanced = page.getByRole("radio", { name: "Balanced" });
    const thorough = page.getByRole("radio", { name: "Thorough — slower" });

    // Tab lands on the currently checked item (roving tabindex).
    await page.keyboard.press("Tab");
    await expect(balanced).toBeFocused();

    // Radix selects the newly focused item while the arrow key is held, so
    // keep the key down across the assertion instead of a fast press (a
    // quick keyup can race the focus handler and skip the auto-select).
    await page.keyboard.down("ArrowDown");
    await expect(thorough).toBeFocused();
    await expect(thorough).toHaveAttribute("aria-checked", "true");
    await page.keyboard.up("ArrowDown");
    await expect(balanced).toHaveAttribute("aria-checked", "false");

    await page.keyboard.down("ArrowUp");
    await expect(balanced).toHaveAttribute("aria-checked", "true");
    await page.keyboard.up("ArrowUp");

    // Space checks the focused item.
    await page.keyboard.press("Space");
    await expect(balanced).toHaveAttribute("aria-checked", "true");
  });

  test("keyboard focus shows a focus ring", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const balanced = page.getByRole("radio", { name: "Balanced" });
    await expect(balanced).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(balanced).toBeFocused();
    await expect(balanced).not.toHaveCSS("box-shadow", "none");
  });

  test("hover changes an item's border color", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY);
    const fast = page.getByRole("radio", { name: "Fast — lower quality" });
    const before = await computedStyle(fast, "border-color");
    await fast.hover();
    await expect(fast).not.toHaveCSS("border-color", before);
  });

  test("disabled group items are disabled, dimmed, and keep state", async ({
    page,
  }) => {
    await gotoStory(page, DISABLED_STORY);
    const selected = page.getByRole("radio", { name: "Selected, disabled" });
    const unselected = page.getByRole("radio", {
      name: "Disabled",
      exact: true,
    });
    await expect(selected).toBeDisabled();
    await expect(selected).toHaveAttribute("data-state", "checked");
    await expect(selected).toHaveCSS("opacity", "0.5");
    await expect(unselected).toBeDisabled();
    await expect(unselected).toHaveAttribute("data-state", "unchecked");
    await expect(unselected).toHaveCSS("opacity", "0.5");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page.getByRole("radio", { name: "Balanced" })
    ).toHaveAttribute("data-state", "checked");
  });
});
