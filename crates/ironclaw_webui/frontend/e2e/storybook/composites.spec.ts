import { expect, test } from "@playwright/test";
import { gotoStory } from "./helpers";

test.describe("Components/Breadcrumb", () => {
  test("renders the path with aria-current on the last crumb (dark)", async ({ page }) => {
    await gotoStory(page, "components-breadcrumb--default");
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
    await gotoStory(page, "components-breadcrumb--default", { theme: "light" });
    await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("crumbs are keyboard focusable with a focus ring", async ({ page }) => {
    await gotoStory(page, "components-breadcrumb--default");
    const first = page.getByRole("button", { name: "workspace" });
    await page.keyboard.press("Tab");
    await expect(first).toBeFocused();
    await expect(first).not.toHaveCSS("box-shadow", "none");
  });
});

test.describe("Composites/ConfirmDialog", () => {
  test("opens from the trigger and auto-focuses cancel", async ({ page }) => {
    await gotoStory(page, "composites-confirmdialog--default");
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
    await gotoStory(page, "composites-confirmdialog--default");
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
    await gotoStory(page, "composites-confirmdialog--default");
    await page.getByRole("button", { name: "Delete chat" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  });
});

test.describe("Composites/EmptyPanel", () => {
  test("renders title, description and CTA", async ({ page }) => {
    await gotoStory(page, "composites-emptypanel--default");
    await expect(page.getByRole("heading", { name: "Pick a file" })).toBeVisible();
    await expect(
      page.getByText("Select a file from the tree to preview its contents.")
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Refresh" })).toBeVisible();
  });

  test("dashed variant renders the drop-zone placeholder", async ({ page }) => {
    await gotoStory(page, "composites-emptypanel--dashed");
    await expect(
      page.getByText("No missions yet. Promote a thread to get started.")
    ).toBeVisible();
  });
});

test.describe("Composites/StatCard", () => {
  test("renders labels, values, badges and detail text", async ({ page }) => {
    await gotoStory(page, "composites-statcard--default");
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

test.describe("Composites/StatStrip", () => {
  test("interactive tiles toggle aria-pressed as filters change", async ({ page }) => {
    await gotoStory(page, "composites-statstrip--filterable");
    const all = page.getByRole("button", { name: /All/ });
    const running = page.getByRole("button", { name: /Running/ });
    await expect(all).toHaveAttribute("aria-pressed", "true");
    await expect(running).toHaveAttribute("aria-pressed", "false");

    await running.click();
    await expect(running).toHaveAttribute("aria-pressed", "true");
    await expect(all).toHaveAttribute("aria-pressed", "false");
  });

  test("static strip renders tiles without buttons", async ({ page }) => {
    await gotoStory(page, "composites-statstrip--default");
    await expect(page.getByText("Scheduled")).toBeVisible();
    await expect(page.getByText("Failures")).toBeVisible();
    await expect(page.getByRole("button")).toHaveCount(0);
  });
});

test.describe("Components/FlowList", () => {
  test("renders numbered steps with titles and descriptions", async ({ page }) => {
    await gotoStory(page, "components-flowlist--default");
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

test.describe("Composites/SectionHeader", () => {
  test("renders eyebrow, title, description and actions", async ({ page }) => {
    await gotoStory(page, "composites-sectionheader--with-actions");
    await expect(page.getByText("Explorer")).toBeVisible();
    await expect(page.getByRole("heading", { name: "Job queue" })).toBeVisible();
    await expect(
      page.getByText("Search by title or ID, jump into a run, and stop active work.")
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Refresh" })).toBeVisible();
    await expect(page.getByRole("group", { name: "Filter" })).toBeVisible();
  });
});

test.describe("Composites/SegmentedControl", () => {
  test("selects options via click and reflects aria-pressed", async ({ page }) => {
    await gotoStory(page, "composites-segmentedcontrol--default");
    const group = page.getByRole("group", { name: "Filter automations" });
    await expect(group).toBeVisible();

    const all = group.getByRole("button", { name: "All" });
    const active = group.getByRole("button", { name: "Active" });
    await expect(all).toHaveAttribute("aria-pressed", "true");

    await active.click();
    await expect(active).toHaveAttribute("aria-pressed", "true");
    await expect(all).toHaveAttribute("aria-pressed", "false");

    await expect(group.getByRole("button", { name: "Completed" })).toBeDisabled();
  });
});

test.describe("Composites/DetailList", () => {
  test("renders semantic definition rows", async ({ page }) => {
    await gotoStory(page, "composites-detaillist--rows");
    await expect(page.getByText("usr-9f31c2")).toBeVisible();
    await expect(page.getByText("Email")).toBeVisible();
    await expect(page.getByText("active")).toBeVisible();
    await expect(page.locator("dl")).toHaveCount(1);
    await expect(page.locator("dt")).toHaveCount(4);
  });
});

test.describe("Composites/Toolbar", () => {
  test("search clears via the trailing button", async ({ page }) => {
    await gotoStory(page, "composites-toolbar--default");
    const input = page.getByRole("searchbox", { name: "Search jobs" });
    await input.fill("nightly");
    const clear = page.getByRole("button", { name: "Clear search" });
    await expect(clear).toBeVisible();
    await clear.click();
    await expect(input).toHaveValue("");
    await expect(clear).toBeHidden();
  });
});

test.describe("Composites/VerticalTabs", () => {
  test("marks the active section and switches on click", async ({ page }) => {
    await gotoStory(page, "composites-verticaltabs--default");
    const nav = page.getByRole("navigation", { name: "Settings sections" });
    const inference = nav.getByRole("button", { name: "Inference" });
    const tools = nav.getByRole("button", { name: /Tools/ });
    await expect(inference).toHaveAttribute("aria-current", "true");

    await tools.click();
    await expect(tools).toHaveAttribute("aria-current", "true");
    await expect(inference).not.toHaveAttribute("aria-current", "true");
    await expect(nav.getByText("12")).toBeVisible();
  });

  test("mobile disclosure lists every section", async ({ page }) => {
    await gotoStory(page, "composites-verticaltabs--mobile");
    const summary = page.locator("summary");
    await expect(summary).toContainText("Inference");
    await summary.click();
    await expect(page.getByRole("button", { name: "Language" })).toBeVisible();
  });
});

test.describe("Composites/SkeletonList", () => {
  test("labelled list exposes a status live region", async ({ page }) => {
    await gotoStory(page, "composites-skeletonlist--default");
    await expect(page.getByRole("status")).toHaveAttribute(
      "aria-label",
      "Loading automations"
    );
  });
});

test.describe("Composites/CodePanel", () => {
  test("renders payloads in a mono panel", async ({ page }) => {
    await gotoStory(page, "composites-codepanel--default");
    await expect(page.getByText('"job_id": "job-7f3a"')).toBeVisible();
  });
});
