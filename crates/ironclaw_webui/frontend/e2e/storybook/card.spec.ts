import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

test.describe("Card", () => {
  test("default card renders content on a solid rounded surface (dark)", async ({ page }) => {
    await gotoStory(page, "components-card--default");
    const card = page.getByText("Card content");
    await expect(card).toBeVisible();
    await expect(card).not.toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    await expect(card).not.toHaveCSS("border-radius", "0px");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("default card renders in light theme", async ({ page }) => {
    await gotoStory(page, "components-card--default", { theme: "light" });
    await expect(page.getByText("Card content")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("variants story renders all four surfaces with distinct backgrounds", async ({ page }) => {
    await gotoStory(page, "components-card--variants");
    for (const variant of ["default", "bordered", "subtle", "inset"]) {
      await expect(page.getByText(variant, { exact: true })).toBeVisible();
    }
    // CardLabel sits directly inside the Card, so its parent is the panel.
    const defaultCard = page.getByText("default", { exact: true }).locator("..");
    const insetCard = page.getByText("inset", { exact: true }).locator("..");
    const defaultBackground = await computedStyle(defaultCard, "background-color");
    await expect(insetCard).not.toHaveCSS("background-color", defaultBackground);
  });

  test("composed story renders header, body and footer with dividers", async ({ page }) => {
    await gotoStory(page, "components-card--composed");
    await expect(page.getByText("Settings", { exact: true })).toBeVisible();
    await expect(page.getByText("Workspace access")).toBeVisible();
    await expect(
      page.getByText("Header, body, and footer sections compose freely with dividers.")
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Save" })).toBeVisible();
    const header = page.getByText("Settings", { exact: true }).locator("..");
    await expect(header).toHaveCSS("border-bottom-width", "1px");
    const footer = page.getByRole("button", { name: "Save" }).locator("..");
    await expect(footer).toHaveCSS("border-top-width", "1px");
  });
});
