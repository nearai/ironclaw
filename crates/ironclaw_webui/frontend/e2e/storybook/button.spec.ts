import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

test.describe("Button", () => {
  test("primary renders in dark theme", async ({ page }) => {
    await gotoStory(page, "components-button--primary");
    await expect(page.getByRole("button", { name: "Button" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("primary renders in light theme", async ({ page }) => {
    await gotoStory(page, "components-button--primary", { theme: "light" });
    await expect(page.getByRole("button", { name: "Button" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("Tab reaches the button and Enter/Space activate it", async ({ page }) => {
    await gotoStory(page, "components-button--primary");
    const button = page.getByRole("button", { name: "Button" });
    await button.evaluate((el) => {
      el.addEventListener("click", () => {
        const count = Number(el.getAttribute("data-clicks") ?? "0") + 1;
        el.setAttribute("data-clicks", String(count));
      });
    });
    await page.keyboard.press("Tab");
    await expect(button).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(button).toHaveAttribute("data-clicks", "1");
    await page.keyboard.press("Space");
    await expect(button).toHaveAttribute("data-clicks", "2");
  });

  test("keyboard focus shows a focus-visible ring", async ({ page }) => {
    await gotoStory(page, "components-button--secondary");
    const button = page.getByRole("button", { name: "Button" });
    await page.keyboard.press("Tab");
    await expect(button).toBeFocused();
    await expect(button).not.toHaveCSS("box-shadow", "none");
  });

  test("secondary background changes on hover", async ({ page }) => {
    await gotoStory(page, "components-button--secondary");
    const button = page.getByRole("button", { name: "Button" });
    const before = await computedStyle(button, "background-color");
    await button.hover();
    await expect(button).not.toHaveCSS("background-color", before);
  });

  test("ghost background changes on hover", async ({ page }) => {
    await gotoStory(page, "components-button--ghost");
    const button = page.getByRole("button", { name: "Button" });
    const before = await computedStyle(button, "background-color");
    await button.hover();
    await expect(button).not.toHaveCSS("background-color", before);
  });

  test("outline and danger variants render", async ({ page }) => {
    await gotoStory(page, "components-button--outline");
    await expect(page.getByRole("button", { name: "Button" })).toBeVisible();
    await gotoStory(page, "components-button--danger");
    await expect(page.getByRole("button", { name: "Button" })).toBeVisible();
  });

  test("disabled story sets disabled attribute and dimmed opacity", async ({ page }) => {
    await gotoStory(page, "components-button--disabled");
    const button = page.getByRole("button", { name: "Button" });
    await expect(button).toBeDisabled();
    await expect(button).toHaveCSS("opacity", "0.5");
    // disabled:pointer-events-none suppresses hover/click hit-testing.
    await expect(button).toHaveCSS("pointer-events", "none");
  });

  test("loading story is aria-busy, disabled, and shows the spinner", async ({ page }) => {
    await gotoStory(page, "components-button--loading");
    const button = page.getByRole("button", { name: "Saving" });
    await expect(button).toHaveAttribute("aria-busy", "true");
    await expect(button).toBeDisabled();
    await expect(page.getByRole("status", { name: "Loading" })).toBeVisible();
  });

  test("as-link story renders an anchor with href", async ({ page }) => {
    await gotoStory(page, "components-button--as-link");
    const link = page.getByRole("link", { name: "Open docs" });
    await expect(link).toBeVisible();
    await expect(link).toHaveAttribute("href", "https://example.com");
  });

  test("sizes story renders text and icon sizes", async ({ page }) => {
    await gotoStory(page, "components-button--sizes");
    await expect(page.getByRole("button", { name: "Small" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Medium" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Large" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Add" })).toHaveCount(2);
  });

  test("all-variants grid renders 15 buttons with loading/disabled states", async ({ page }) => {
    await gotoStory(page, "components-button--all-variants");
    await expect(page.getByRole("button")).toHaveCount(15);
    for (const variant of ["primary", "outline", "secondary", "ghost", "danger"]) {
      await expect(page.getByRole("button", { name: variant, exact: true })).toBeVisible();
    }
    const loadingButtons = page.getByRole("button", { name: "loading" });
    await expect(loadingButtons).toHaveCount(5);
    for (let index = 0; index < 5; index += 1) {
      await expect(loadingButtons.nth(index)).toHaveAttribute("aria-busy", "true");
      await expect(loadingButtons.nth(index)).toBeDisabled();
    }
    const disabledButtons = page.getByRole("button", { name: "disabled" });
    await expect(disabledButtons).toHaveCount(5);
    for (let index = 0; index < 5; index += 1) {
      await expect(disabledButtons.nth(index)).toBeDisabled();
    }
  });
});
