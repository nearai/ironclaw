import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const IMPERATIVE_STORY = "extras-toast--imperative";
const COMPOSED_STORY = "extras-toast--composed";

/** Toast roots render as list items inside the Radix viewport. */
function toastWithText(page: import("@playwright/test").Page, text: string) {
  return page.locator("li").filter({ hasText: text });
}

test.describe("extras/toast", () => {
  test("imperative: button triggers a toast and close dismisses it", async ({
    page,
  }) => {
    await gotoStory(page, IMPERATIVE_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await page.getByRole("button", { name: "Default", exact: true }).click();

    const item = toastWithText(page, "Run started");
    await expect(item).toBeVisible();
    await expect(item).toContainText("researcher · run-4822");
    await expect(item).toHaveAttribute("data-state", "open");

    await item.getByRole("button", { name: "Dismiss" }).click();
    await expect(toastWithText(page, "Run started")).toHaveCount(0);
  });

  test("imperative: toned toasts render with their titles", async ({
    page,
  }) => {
    await gotoStory(page, IMPERATIVE_STORY);
    await page.getByRole("button", { name: "Danger", exact: true }).click();
    await expect(toastWithText(page, "Run failed")).toBeVisible();

    await page.getByRole("button", { name: "Positive", exact: true }).click();
    await expect(toastWithText(page, "Run complete")).toBeVisible();
    // Both toasts stack in the viewport.
    await expect(page.locator('li[data-state="open"]')).toHaveCount(2);
  });

  test("composed: renders open with title, description, and working action", async ({
    page,
  }) => {
    await gotoStory(page, COMPOSED_STORY);
    const item = toastWithText(page, "Deploy finished");
    await expect(item).toBeVisible();
    await expect(item).toContainText("webui-v2 → production");

    const action = item.getByRole("button", { name: "View" });
    await expect(action).toBeVisible();
    await action.click();
    await expect(toastWithText(page, "Deploy finished")).toHaveCount(0);

    // Reopen via the story button, then dismiss with the close button.
    await page.getByRole("button", { name: "Show composed toast" }).click();
    await expect(item).toBeVisible();
    await item.getByRole("button", { name: "Dismiss" }).click();
    await expect(toastWithText(page, "Deploy finished")).toHaveCount(0);
  });

  test("composed: hovering the action button changes its background", async ({
    page,
  }) => {
    await gotoStory(page, COMPOSED_STORY);
    const action = toastWithText(page, "Deploy finished").getByRole("button", {
      name: "View",
    });
    const before = await computedStyle(action, "background-color");
    await action.hover();
    await expect(action).not.toHaveCSS("background-color", before);
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, COMPOSED_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(toastWithText(page, "Deploy finished")).toBeVisible();
  });
});
