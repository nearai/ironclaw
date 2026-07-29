import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const COMPOSED_STORY = "extras-pagination--composed";
const SIMPLE_STORY = "extras-pagination--simple";
const FEW_PAGES_STORY = "extras-pagination--few-pages";

test.describe("extras/pagination", () => {
  test("composed: renders nav landmark with active page and disabled previous", async ({
    page,
  }) => {
    await gotoStory(page, COMPOSED_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(
      page.getByRole("navigation", { name: "Pagination" })
    ).toBeVisible();

    const active = page.getByRole("button", { name: "1", exact: true });
    await expect(active).toHaveAttribute("aria-current", "page");
    await expect(
      page.getByRole("button", { name: "2", exact: true })
    ).not.toHaveAttribute("aria-current", "page");

    const previous = page.getByRole("button", { name: "Previous" });
    await expect(previous).toBeDisabled();
    await expect(previous).toHaveCSS("opacity", "0.5");

    // Ellipsis is decorative and hidden from assistive tech.
    await expect(page.locator('[aria-hidden="true"]', { hasText: "…" })).toBeVisible();
  });

  test("simple: clicking pages and next/previous moves aria-current", async ({
    page,
  }) => {
    await gotoStory(page, SIMPLE_STORY);
    await expect(
      page.getByRole("button", { name: "7", exact: true })
    ).toHaveAttribute("aria-current", "page");

    await page.getByRole("button", { name: "8", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "8", exact: true })
    ).toHaveAttribute("aria-current", "page");

    await page.getByRole("button", { name: "Next" }).click();
    await expect(
      page.getByRole("button", { name: "9", exact: true })
    ).toHaveAttribute("aria-current", "page");

    await page.getByRole("button", { name: "Previous" }).click();
    await expect(
      page.getByRole("button", { name: "8", exact: true })
    ).toHaveAttribute("aria-current", "page");
  });

  test("simple: pages are reachable by keyboard and Enter activates", async ({
    page,
  }) => {
    await gotoStory(page, SIMPLE_STORY);
    // Tab order: Previous, 1, 7-window start (6).
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button", { name: "Previous" })).toBeFocused();
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    const six = page.getByRole("button", { name: "6", exact: true });
    await expect(six).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(six).toHaveAttribute("aria-current", "page");
  });

  test("keyboard focus shows a focus ring", async ({ page }) => {
    await gotoStory(page, SIMPLE_STORY);
    const previous = page.getByRole("button", { name: "Previous" });
    await expect(previous).toHaveCSS("box-shadow", "none");
    await page.keyboard.press("Tab");
    await expect(previous).toBeFocused();
    await expect(previous).not.toHaveCSS("box-shadow", "none");
  });

  test("hovering an inactive page changes its background color", async ({
    page,
  }) => {
    await gotoStory(page, COMPOSED_STORY);
    const two = page.getByRole("button", { name: "2", exact: true });
    const before = await computedStyle(two, "background-color");
    await two.hover();
    await expect(two).not.toHaveCSS("background-color", before);
  });

  test("few pages: no ellipsis and previous enables after paging", async ({
    page,
  }) => {
    await gotoStory(page, FEW_PAGES_STORY);
    for (const label of ["1", "2", "3", "4"]) {
      await expect(
        page.getByRole("button", { name: label, exact: true })
      ).toBeVisible();
    }
    await expect(page.getByText("…")).toHaveCount(0);

    const previous = page.getByRole("button", { name: "Previous" });
    await expect(previous).toBeDisabled();
    await page.getByRole("button", { name: "Next" }).click();
    await expect(
      page.getByRole("button", { name: "2", exact: true })
    ).toHaveAttribute("aria-current", "page");
    await expect(previous).toBeEnabled();
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, COMPOSED_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page.getByRole("navigation", { name: "Pagination" })
    ).toBeVisible();
  });
});
