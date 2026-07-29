import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const STORY = "extras-tabs--default";

test.describe("extras/tabs", () => {
  test("renders tablist with the default tab active", async ({ page }) => {
    await gotoStory(page, STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByRole("tablist")).toBeVisible();
    const overview = page.getByRole("tab", { name: "Overview" });
    await expect(overview).toHaveAttribute("data-state", "active");
    await expect(overview).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("tab", { name: "Logs" })).toHaveAttribute(
      "data-state",
      "inactive"
    );
    await expect(page.getByRole("tabpanel")).toContainText(
      "High-level run metrics"
    );
  });

  test("arrow keys move activation and switch panels, skipping disabled", async ({
    page,
  }) => {
    await gotoStory(page, STORY);
    await page.keyboard.press("Tab");
    const overview = page.getByRole("tab", { name: "Overview" });
    await expect(overview).toBeFocused();

    await page.keyboard.press("ArrowRight");
    const logs = page.getByRole("tab", { name: "Logs" });
    await expect(logs).toHaveAttribute("data-state", "active");
    await expect(logs).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("tabpanel")).toContainText(
      "Structured log stream"
    );

    await page.keyboard.press("ArrowRight");
    await expect(page.getByRole("tab", { name: "Settings" })).toHaveAttribute(
      "data-state",
      "active"
    );
    await expect(page.getByRole("tabpanel")).toContainText(
      "Per-run configuration"
    );

    // Danger is disabled, so the next arrow wraps back to Overview.
    await page.keyboard.press("ArrowRight");
    await expect(overview).toHaveAttribute("data-state", "active");
  });

  test("disabled tab is disabled and dimmed", async ({ page }) => {
    await gotoStory(page, STORY);
    const danger = page.getByRole("tab", { name: "Danger" });
    await expect(danger).toBeDisabled();
    await expect(danger).toHaveCSS("opacity", "0.5");
    await expect(danger).toHaveAttribute("data-state", "inactive");
  });

  test("hovering an inactive tab changes its text color", async ({ page }) => {
    await gotoStory(page, STORY);
    const logs = page.getByRole("tab", { name: "Logs" });
    const before = await computedStyle(logs, "color");
    await logs.hover();
    await expect(logs).not.toHaveCSS("color", before);
  });

  test("keyboard focus shows a focus ring", async ({ page }) => {
    await gotoStory(page, STORY);
    const overview = page.getByRole("tab", { name: "Overview" });
    const before = await computedStyle(overview, "box-shadow");
    await page.keyboard.press("Tab");
    await expect(overview).toBeFocused();
    await expect(overview).not.toHaveCSS("box-shadow", before);
    await expect(overview).not.toHaveCSS("box-shadow", "none");
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("tablist")).toBeVisible();
    await expect(page.getByRole("tab", { name: "Overview" })).toHaveAttribute(
      "data-state",
      "active"
    );
  });
});
