import { expect, test } from "@playwright/test";
import { computedStyle, gotoStory } from "./helpers";

const PRIMITIVES_STORY = "extras-table--primitives";
const DATA_TABLE_STORY = "extras-table--data-table-story";
const EMPTY_STORY = "extras-table--data-table-empty";

test.describe("extras/table", () => {
  test("primitives: renders caption, headers, body rows, and footer", async ({
    page,
  }) => {
    await gotoStory(page, PRIMITIVES_STORY);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByRole("table")).toBeVisible();
    await expect(page.getByText("Recent agent runs.")).toBeVisible();
    for (const header of ["Run", "Agent", "Tokens"]) {
      await expect(
        page.getByRole("columnheader", { name: header, exact: true })
      ).toBeVisible();
    }
    // Header row + 4 data rows + footer row.
    await expect(page.getByRole("row")).toHaveCount(6);
    await expect(
      page.getByRole("cell", { name: "run-4821", exact: true })
    ).toBeVisible();
    await expect(page.getByRole("row").last()).toContainText("Total");
  });

  test("primitives: hovering a body row changes its background", async ({
    page,
  }) => {
    await gotoStory(page, PRIMITIVES_STORY);
    const row = page.getByRole("row").filter({ hasText: "run-4821" });
    const before = await computedStyle(row, "background-color");
    await row.hover();
    await expect(row).not.toHaveCSS("background-color", before);
  });

  test("data table: renders status badges and right-aligned numbers", async ({
    page,
  }) => {
    await gotoStory(page, DATA_TABLE_STORY);
    await expect(page.getByRole("table")).toBeVisible();
    await expect(page.getByText("failed", { exact: true })).toBeVisible();
    await expect(page.getByText("running", { exact: true })).toBeVisible();
    await expect(page.getByText("done", { exact: true })).toHaveCount(2);

    const tokensCell = page.getByRole("cell", {
      name: "122,400",
      exact: true,
    });
    await expect(tokensCell).toBeVisible();
    await expect(tokensCell).toHaveCSS("text-align", "right");
  });

  test("data table: empty state renders the placeholder row", async ({
    page,
  }) => {
    await gotoStory(page, EMPTY_STORY);
    await expect(page.getByText("No runs yet")).toBeVisible();
    // Header row + single empty-state row.
    await expect(page.getByRole("row")).toHaveCount(2);
  });

  test("renders in the light theme", async ({ page }) => {
    await gotoStory(page, PRIMITIVES_STORY, { theme: "light" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.getByRole("table")).toBeVisible();
  });
});
