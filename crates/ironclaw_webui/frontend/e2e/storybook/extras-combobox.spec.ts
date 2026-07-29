/**
 * Extras/Combobox — trigger open/close (aria-expanded), typed filtering,
 * arrow-key highlight movement (skips disabled options), Enter selection,
 * Escape dismissal, empty message, disabled trigger, hover/focus states,
 * theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT = "extras-combobox--default";
const DISABLED = "extras-combobox--disabled";
const EMPTY = "extras-combobox--empty";

test.describe("extras combobox", () => {
  test("renders the selected value closed (dark theme)", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const trigger = page.getByRole("button", { name: "Region" });
    await expect(trigger).toBeVisible();
    await expect(trigger).toContainText("EU Central (Frankfurt)");
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(trigger).toHaveAttribute("aria-haspopup", "listbox");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("button", { name: "Region" })).toBeVisible();
  });

  test("click opens the listbox with the search input focused", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Region" });
    await trigger.click();
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    const input = page.getByRole("combobox");
    await expect(input).toBeFocused();
    await expect(page.getByRole("listbox")).toBeVisible();
    await expect(page.getByRole("option")).toHaveCount(6);
    await expect(
      page.getByRole("option", { name: /EU Central/ })
    ).toHaveAttribute("aria-selected", "true");
  });

  test("typing filters options; Enter selects the highlighted one", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Region" });
    await trigger.click();
    await page.keyboard.type("us");
    await expect(page.getByRole("option")).toHaveCount(2);
    // ArrowDown moves the active highlight to US West.
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect(trigger).toContainText("US West (Oregon)");
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(trigger).toBeFocused();
  });

  test("arrow navigation skips the disabled option", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    await page.getByRole("button", { name: "Region" }).click();
    const input = page.getByRole("combobox");
    const tokyo = page.getByRole("option", { name: /Tokyo/ });
    await expect(tokyo).toHaveAttribute("aria-disabled", "true");
    await expect(tokyo).toBeDisabled();
    // ArrowUp from the first option wraps to the last enabled one (Mumbai),
    // skipping the disabled Tokyo entry.
    await page.keyboard.press("ArrowUp");
    const mumbai = page.getByRole("option", { name: /Mumbai/ });
    const mumbaiId = await mumbai.getAttribute("id");
    await expect(input).toHaveAttribute(
      "aria-activedescendant",
      mumbaiId ?? ""
    );
  });

  test("Escape closes the popover and restores trigger focus", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Region" });
    await trigger.click();
    await expect(page.getByRole("listbox")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("listbox")).toBeHidden();
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await expect(trigger).toBeFocused();
  });

  test("empty options render the empty message", async ({ page }) => {
    await gotoStory(page, EMPTY);
    await page.getByRole("button", { name: "Region" }).click();
    await expect(page.getByText("No regions configured")).toBeVisible();
    await expect(page.getByRole("option")).toHaveCount(0);
  });

  test("disabled trigger has reduced opacity and never opens", async ({
    page,
  }) => {
    await gotoStory(page, DISABLED);
    const trigger = page.getByRole("button", { name: "Region" });
    await expect(trigger).toBeDisabled();
    await expect(trigger).toHaveCSS("opacity", "0.5");
    await trigger.click({ force: true });
    await expect(page.getByRole("listbox")).toHaveCount(0);
  });

  test("hover shifts the trigger border color toward the accent", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Region" });
    const before = await computedStyle(trigger, "border-color");
    await trigger.hover();
    await expect(trigger).not.toHaveCSS("border-color", before);
  });

  test("focus-visible: keyboard focus shows a ring (box-shadow)", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    const trigger = page.getByRole("button", { name: "Region" });
    await page.keyboard.press("Tab");
    await expect(trigger).toBeFocused();
    await expect(trigger).not.toHaveCSS("box-shadow", "none");
  });
});
