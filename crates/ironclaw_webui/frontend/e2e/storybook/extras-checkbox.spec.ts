/**
 * Extras/Checkbox — checked/unchecked/indeterminate/disabled states,
 * Space-key toggling, hover border shift, focus-visible ring, theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const DEFAULT = "extras-checkbox--default";
const STATES = "extras-checkbox--states";

test.describe("extras checkbox", () => {
  test("default story renders checked with a label (dark theme)", async ({
    page,
  }) => {
    await gotoStory(page, DEFAULT);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const checkbox = page.getByRole("checkbox", {
      name: "Email me run summaries",
    });
    await expect(checkbox).toBeVisible();
    await expect(checkbox).toHaveAttribute("data-state", "checked");
    await expect(checkbox).toHaveAttribute("aria-checked", "true");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, DEFAULT, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page.getByRole("checkbox", { name: "Email me run summaries" })
    ).toBeVisible();
  });

  test("keyboard: Tab focuses, Space toggles data-state", async ({ page }) => {
    await gotoStory(page, DEFAULT);
    const checkbox = page.getByRole("checkbox", {
      name: "Email me run summaries",
    });
    await page.keyboard.press("Tab");
    await expect(checkbox).toBeFocused();
    await page.keyboard.press("Space");
    await expect(checkbox).toHaveAttribute("data-state", "unchecked");
    await expect(checkbox).toHaveAttribute("aria-checked", "false");
    await page.keyboard.press("Space");
    await expect(checkbox).toHaveAttribute("data-state", "checked");
  });

  test("states story exposes all four Radix states", async ({ page }) => {
    await gotoStory(page, STATES);
    await expect(
      page.getByRole("checkbox", { name: "Unchecked" })
    ).toHaveAttribute("data-state", "unchecked");
    await expect(
      page.getByRole("checkbox", { name: "Checked", exact: true })
    ).toHaveAttribute("data-state", "checked");
    const indeterminate = page.getByRole("checkbox", { name: "Indeterminate" });
    await expect(indeterminate).toHaveAttribute("data-state", "indeterminate");
    await expect(indeterminate).toHaveAttribute("aria-checked", "mixed");
  });

  test("disabled checkbox has reduced opacity and does not toggle", async ({
    page,
  }) => {
    await gotoStory(page, STATES);
    const disabled = page.getByRole("checkbox", { name: "Disabled" });
    await expect(disabled).toBeDisabled();
    await expect(disabled).toHaveAttribute("data-disabled", "");
    await expect(disabled).toHaveCSS("opacity", "0.5");
    await disabled.click({ force: true });
    await expect(disabled).toHaveAttribute("data-state", "checked");
  });

  test("hover shifts the border color toward the accent", async ({ page }) => {
    await gotoStory(page, STATES);
    const checkbox = page.getByRole("checkbox", { name: "Unchecked" });
    const before = await computedStyle(checkbox, "border-color");
    await checkbox.hover();
    await expect(checkbox).not.toHaveCSS("border-color", before);
  });

  test("focus-visible: keyboard focus shows a ring (box-shadow)", async ({
    page,
  }) => {
    await gotoStory(page, STATES);
    const checkbox = page.getByRole("checkbox", { name: "Unchecked" });
    await page.keyboard.press("Tab");
    await expect(checkbox).toBeFocused();
    await expect(checkbox).not.toHaveCSS("box-shadow", "none");
  });
});
