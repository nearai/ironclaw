import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

test.describe("IconButton", () => {
  test("ghost renders in dark theme with type=button", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--ghost");
    const button = page.getByRole("button", { name: "Notifications" });
    await expect(button).toBeVisible();
    await expect(button).toHaveAttribute("type", "button");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("ghost renders in light theme", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--ghost", { theme: "light" });
    await expect(page.getByRole("button", { name: "Notifications" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("Tab focuses the button and shows a focus-visible ring", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--ghost");
    const button = page.getByRole("button", { name: "Notifications" });
    await page.keyboard.press("Tab");
    await expect(button).toBeFocused();
    await expect(button).not.toHaveCSS("box-shadow", "none");
  });

  test("Enter activates the focused button", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--ghost");
    const button = page.getByRole("button", { name: "Notifications" });
    await button.evaluate((el) => {
      el.addEventListener("click", () => el.setAttribute("data-activated", "true"));
    });
    await page.keyboard.press("Tab");
    await page.keyboard.press("Enter");
    await expect(button).toHaveAttribute("data-activated", "true");
  });

  test("background changes on hover", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--ghost");
    const button = page.getByRole("button", { name: "Notifications" });
    const before = await computedStyle(button, "background-color");
    await button.hover();
    await expect(button).not.toHaveCSS("background-color", before);
  });

  test("active state carries an accent-tinted background", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--states");
    const plain = page.getByRole("button", { name: "Default" });
    const selected = page.getByRole("button", { name: "Selected" });
    await expect(plain).toBeVisible();
    const plainBackground = await computedStyle(plain, "background-color");
    await expect(selected).not.toHaveCSS("background-color", plainBackground);
  });

  test("disabled story sets disabled attribute and dimmed opacity", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--disabled");
    const button = page.getByRole("button", { name: "Notifications" });
    await expect(button).toBeDisabled();
    await expect(button).toHaveCSS("opacity", "0.5");
    await expect(button).toHaveCSS("pointer-events", "none");
  });

  test("as-anchor story renders a link with href", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--as-anchor");
    const link = page.getByRole("link", { name: "Docs" });
    await expect(link).toBeVisible();
    await expect(link).toHaveAttribute("href", "https://example.com");
  });

  test("plain variant story renders with custom colors", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--plain-with-custom-colors");
    const button = page.getByRole("button", { name: "Attestation" });
    await expect(button).toBeVisible();
    await expect(button).not.toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  });

  test("header row renders all four actions", async ({ page }) => {
    await gotoStory(page, "components-iconbutton--header-row");
    await expect(page.getByRole("button")).toHaveCount(4);
    for (const name of ["Toggle sidebar", "Notifications", "Logs", "Docs"]) {
      await expect(page.getByRole("button", { name })).toBeVisible();
    }
  });
});
