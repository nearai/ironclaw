/**
 * Extras/InputOTP — digits land in slots and auto-advance, completion
 * callback, Backspace/arrow-key movement, non-digit rejection, custom
 * length, disabled state, hover border shift, focus-visible ring, theming.
 */
import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const SIX = "extras-inputotp--six-digits";
const FOUR = "extras-inputotp--four-digits";
const DISABLED = "extras-inputotp--disabled";

test.describe("extras input-otp", () => {
  test("renders six empty cells (dark theme)", async ({ page }) => {
    await gotoStory(page, SIX);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    const group = page.getByRole("group", { name: "One-time code" });
    await expect(group).toBeVisible();
    await expect(group.getByRole("textbox")).toHaveCount(6);
    await expect(
      page.getByRole("textbox", { name: "Digit 1 of 6" })
    ).toHaveValue("");
    await expect(page.getByText("Typed: —")).toBeVisible();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, SIX, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page.getByRole("group", { name: "One-time code" })
    ).toBeVisible();
  });

  test("typed digits fill the slots and advance focus to completion", async ({
    page,
  }) => {
    await gotoStory(page, SIX);
    await page.getByRole("textbox", { name: "Digit 1 of 6" }).click();
    await page.keyboard.type("123456");
    for (let i = 0; i < 6; i += 1) {
      await expect(
        page.getByRole("textbox", { name: `Digit ${i + 1} of 6` })
      ).toHaveValue(String(i + 1));
    }
    await expect(page.getByText("Code complete ✓")).toBeVisible();
    await expect(
      page.getByRole("textbox", { name: "Digit 6 of 6" })
    ).toBeFocused();
  });

  test("Backspace clears the current digit, then walks back", async ({
    page,
  }) => {
    await gotoStory(page, SIX);
    await page.getByRole("textbox", { name: "Digit 1 of 6" }).click();
    await page.keyboard.type("123");
    await expect(page.getByText("Typed: 123")).toBeVisible();
    // Focus sits on cell 4 (empty) → Backspace deletes cell 3 and moves back.
    await page.keyboard.press("Backspace");
    await expect(page.getByText("Typed: 12")).toBeVisible();
    await expect(
      page.getByRole("textbox", { name: "Digit 3 of 6" })
    ).toBeFocused();
    // Cell 3 is now empty → another Backspace deletes cell 2.
    await page.keyboard.press("Backspace");
    await expect(page.getByText("Typed: 1")).toBeVisible();
  });

  test("arrow keys move focus between cells", async ({ page }) => {
    await gotoStory(page, SIX);
    await page.getByRole("textbox", { name: "Digit 1 of 6" }).click();
    await page.keyboard.press("ArrowRight");
    await expect(
      page.getByRole("textbox", { name: "Digit 2 of 6" })
    ).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await expect(
      page.getByRole("textbox", { name: "Digit 3 of 6" })
    ).toBeFocused();
    await page.keyboard.press("ArrowLeft");
    await expect(
      page.getByRole("textbox", { name: "Digit 2 of 6" })
    ).toBeFocused();
  });

  test("non-digit input is rejected in numeric mode", async ({ page }) => {
    await gotoStory(page, SIX);
    const first = page.getByRole("textbox", { name: "Digit 1 of 6" });
    await first.click();
    await page.keyboard.type("ab");
    await expect(first).toHaveValue("");
    await expect(page.getByText("Typed: —")).toBeVisible();
  });

  test("four-digit story completes after four digits", async ({ page }) => {
    await gotoStory(page, FOUR);
    const group = page.getByRole("group", { name: "One-time code" });
    await expect(group.getByRole("textbox")).toHaveCount(4);
    await group.getByRole("textbox", { name: "Digit 1 of 4" }).click();
    await page.keyboard.type("9876");
    await expect(page.getByText("Code complete ✓")).toBeVisible();
  });

  test("disabled cells carry the value, opacity 0.5, and reject input", async ({
    page,
  }) => {
    await gotoStory(page, DISABLED);
    const first = page.getByRole("textbox", { name: "Digit 1 of 6" });
    const second = page.getByRole("textbox", { name: "Digit 2 of 6" });
    await expect(first).toHaveValue("4");
    await expect(second).toHaveValue("2");
    await expect(first).toBeDisabled();
    await expect(first).toHaveCSS("opacity", "0.5");
    await expect(
      page.getByRole("textbox", { name: "Digit 3 of 6" })
    ).toBeDisabled();
  });

  test("hover shifts a cell border color toward the accent", async ({
    page,
  }) => {
    await gotoStory(page, SIX);
    const first = page.getByRole("textbox", { name: "Digit 1 of 6" });
    const before = await computedStyle(first, "border-color");
    await first.hover();
    await expect(first).not.toHaveCSS("border-color", before);
  });

  test("focus-visible: keyboard focus shows a ring (box-shadow)", async ({
    page,
  }) => {
    await gotoStory(page, SIX);
    const first = page.getByRole("textbox", { name: "Digit 1 of 6" });
    await page.keyboard.press("Tab");
    await expect(first).toBeFocused();
    await expect(first).not.toHaveCSS("box-shadow", "none");
  });
});
