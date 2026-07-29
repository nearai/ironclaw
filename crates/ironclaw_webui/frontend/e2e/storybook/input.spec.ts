import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

test.describe("Input", () => {
  test("default input renders and accepts typing (dark)", async ({ page }) => {
    await gotoStory(page, "components-input--default");
    const input = page.getByPlaceholder("Type here…");
    await expect(input).toBeVisible();
    await input.fill("hello world");
    await expect(input).toHaveValue("hello world");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("default input renders in light theme", async ({ page }) => {
    await gotoStory(page, "components-input--default", { theme: "light" });
    await expect(page.getByPlaceholder("Type here…")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("Tab focuses the input and applies the focus ring + accent border", async ({ page }) => {
    await gotoStory(page, "components-input--default");
    const input = page.getByPlaceholder("Type here…");
    const borderBefore = await computedStyle(input, "border-color");
    await page.keyboard.press("Tab");
    await expect(input).toBeFocused();
    await expect(input).not.toHaveCSS("box-shadow", "none");
    await expect(input).not.toHaveCSS("border-color", borderBefore);
  });

  test("error input carries the danger border/ring classes", async ({ page }) => {
    await gotoStory(page, "components-input--error");
    const input = page.getByPlaceholder("Type here…");
    await expect(input).toBeVisible();
    // Note: the danger utilities lose to the base border/ring utilities in
    // the compiled CSS order, so no computed style actually changes — the
    // class wiring is the only observable effect of `error` today.
    await expect(input).toHaveClass(/border-\[var\(--v2-danger-text\)\]/);
    await gotoStory(page, "components-input--with-form-field");
    await expect(page.locator("#sb-key")).toHaveClass(/border-\[var\(--v2-danger-text\)\]/);
    await expect(page.locator("#sb-name")).not.toHaveClass(/danger/);
  });

  test("sizes story renders sm/md/lg heights", async ({ page }) => {
    await gotoStory(page, "components-input--sizes");
    await expect(page.getByPlaceholder("Small")).toHaveCSS("height", "36px");
    await expect(page.getByPlaceholder("Medium")).toBeVisible();
    await expect(page.getByPlaceholder("Large")).toHaveCSS("height", "54px");
  });

  test("textarea story renders and accepts multiline text", async ({ page }) => {
    await gotoStory(page, "components-input--textarea-story");
    const textarea = page.getByPlaceholder("Longer content…");
    await expect(textarea).toBeVisible();
    await textarea.fill("line one\nline two");
    await expect(textarea).toHaveValue("line one\nline two");
  });

  test("select story renders with default value and changes option", async ({ page }) => {
    await gotoStory(page, "components-input--select-story");
    const select = page.getByRole("combobox");
    await expect(select).toHaveValue("b");
    await select.selectOption("a");
    await expect(select).toHaveValue("a");
  });

  test("form field story wires label, hint and error message", async ({ page }) => {
    await gotoStory(page, "components-input--with-form-field");
    await expect(page.getByText("Display name")).toBeVisible();
    await expect(page.getByText("Shown in the sidebar.")).toBeVisible();
    // Label htmlFor → clicking the label focuses the input.
    await page.getByText("Display name").click();
    await expect(page.locator("#sb-name")).toBeFocused();
    const error = page.getByRole("alert");
    await expect(error).toHaveText("This key is invalid.");
    await expect(page.getByText("Standalone label")).toBeVisible();
  });
});
