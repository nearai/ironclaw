import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

test.describe("Composites/Breadcrumb", () => {
  test("renders the path with aria-current on the last crumb (dark)", async ({ page }) => {
    await gotoStory(page, "composites-overview--breadcrumb-story");
    const nav = page.getByRole("navigation", { name: "Workspace" });
    await expect(nav).toBeVisible();
    await expect(nav.getByRole("button")).toHaveCount(4);
    await expect(nav.locator('[aria-current="page"]')).toHaveCount(1);
    await expect(nav.getByRole("button", { name: "2026-q3-summary.md" })).toHaveAttribute(
      "aria-current",
      "page"
    );
    await expect(nav.getByText("/", { exact: true })).toHaveCount(3);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("renders in light theme", async ({ page }) => {
    await gotoStory(page, "composites-overview--breadcrumb-story", { theme: "light" });
    await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("crumbs are keyboard focusable with a focus ring", async ({ page }) => {
    await gotoStory(page, "composites-overview--breadcrumb-story");
    const first = page.getByRole("button", { name: "workspace" });
    await page.keyboard.press("Tab");
    await expect(first).toBeFocused();
    await expect(first).not.toHaveCSS("box-shadow", "none");
  });
});

test.describe("Composites/ConfirmDialog", () => {
  test("opens from the trigger and auto-focuses cancel", async ({ page }) => {
    await gotoStory(page, "composites-overview--confirm-dialog-story");
    await page.getByRole("button", { name: "Delete chat" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(page.getByRole("heading", { name: "Delete chat" })).toBeVisible();
    await expect(
      page.getByText("This permanently removes the conversation.")
    ).toBeVisible();
    await expect(page.getByTestId("confirm-dialog-cancel")).toBeFocused();
  });

  test("confirm and cancel both close the dialog", async ({ page }) => {
    await gotoStory(page, "composites-overview--confirm-dialog-story");
    const trigger = page.getByRole("button", { name: "Delete chat" });
    const dialog = page.getByRole("dialog");

    await trigger.click();
    await page.getByTestId("confirm-dialog-confirm").click();
    await expect(dialog).toBeHidden();

    await trigger.click();
    await page.getByTestId("confirm-dialog-cancel").click();
    await expect(dialog).toBeHidden();
  });

  test("Escape closes the dialog", async ({ page }) => {
    await gotoStory(page, "composites-overview--confirm-dialog-story");
    await page.getByRole("button", { name: "Delete chat" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  });
});

test.describe("Composites/EmptyPanel", () => {
  test("renders title, description and CTA", async ({ page }) => {
    await gotoStory(page, "composites-overview--empty-panel-story");
    await expect(page.getByRole("heading", { name: "Pick a file" })).toBeVisible();
    await expect(
      page.getByText("Select a file from the tree to preview its contents.")
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Refresh" })).toBeVisible();
  });
});

test.describe("Composites/StatCard", () => {
  test("renders labels, values, badges and detail text", async ({ page }) => {
    await gotoStory(page, "composites-overview--stat-cards");
    await expect(page.getByText("Active runs")).toBeVisible();
    await expect(page.getByText("12", { exact: true })).toBeVisible();
    await expect(page.getByText("live")).toBeVisible();
    await expect(page.getByText("Failures (24h)")).toBeVisible();
    await expect(page.getByText("failing")).toBeVisible();
    await expect(page.getByText("Retry from the runs tab.")).toBeVisible();
    await expect(page.getByText("Last deploy")).toBeVisible();
    await expect(page.getByText("Jul 26")).toBeVisible();
    await expect(page.getByText("idle")).toBeVisible();
  });
});

test.describe("Composites/FlowList", () => {
  test("renders numbered steps with titles and descriptions", async ({ page }) => {
    await gotoStory(page, "composites-overview--flow-list-story");
    for (const number of ["01", "02", "03"]) {
      await expect(page.getByText(number, { exact: true })).toBeVisible();
    }
    await expect(page.getByText("Connect a provider")).toBeVisible();
    await expect(page.getByText("Start a chat")).toBeVisible();
    await expect(page.getByText("Automate", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Promote a recurring prompt into an automation.")
    ).toBeVisible();
  });
});

test.describe("Composites/Headings", () => {
  test("renders SectionHeader and SubLabel", async ({ page }) => {
    await gotoStory(page, "composites-overview--headings");
    await expect(page.getByRole("heading", { name: "Automations" })).toBeVisible();
    await expect(
      page.getByText("Recurring agent runs and their history.")
    ).toBeVisible();
    await expect(page.getByText("Delivery defaults")).toBeVisible();
  });
});
