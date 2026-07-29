/**
 * Sidebar navigation coverage for the agent workspace shell (demo mode).
 *
 * Exercises every non-hidden primary route from src/app/routes.ts: plain nav
 * items (Workspace, Automations) navigate directly, while the expandable
 * items (Extensions, Settings, Admin) navigate to their first sub-route and
 * reveal a sub-route list that is exercised by clicking a second sub-item.
 * Chat has no primary nav entry — it is reached via the brand logo link.
 */
import { expect, test, type Page } from "@playwright/test";

function sidebar(page: Page) {
  return page.locator("#gateway-sidebar");
}

test.describe("sidebar navigation", () => {
  test("initial load lands on /chat with sidebar navigation visible", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveURL(/\/chat$/);
    await expect(sidebar(page)).toBeVisible();
    await expect(sidebar(page).getByRole("navigation")).toBeVisible();
    // All non-hidden primary routes are represented (demo session is admin).
    await expect(sidebar(page).getByTestId("nav-workspace")).toBeVisible();
    await expect(sidebar(page).getByTestId("nav-automations")).toBeVisible();
    await expect(sidebar(page).getByRole("link", { name: "Extensions" })).toBeVisible();
    await expect(sidebar(page).getByRole("link", { name: "Settings" })).toBeVisible();
    await expect(sidebar(page).getByRole("link", { name: "Admin" })).toBeVisible();
    // Hidden routes stay out of the sidebar nav.
    for (const hidden of ["projects", "jobs", "routines", "missions", "logs"]) {
      await expect(sidebar(page).getByTestId(`nav-${hidden}`)).toHaveCount(0);
    }
  });

  test("brand logo link returns to /chat", async ({ page }) => {
    await page.goto("/workspace");
    await expect(page.getByRole("heading", { name: "Workspace" })).toBeVisible();
    await sidebar(page).getByRole("link", { name: "NEAR AI" }).click();
    await expect(page).toHaveURL(/\/chat$/);
    await expect(
      page.getByRole("heading", { name: "Hello, what do you need help with?" })
    ).toBeVisible();
  });

  test("workspace nav item renders the workspace page", async ({ page }) => {
    await page.goto("/");
    await sidebar(page).getByTestId("nav-workspace").click();
    await expect(page).toHaveURL(/\/workspace$/);
    await expect(page.getByRole("heading", { name: "Workspace" })).toBeVisible();
  });

  test("automations nav item renders the automations page", async ({ page }) => {
    await page.goto("/");
    await sidebar(page).getByTestId("nav-automations").click();
    await expect(page).toHaveURL(/\/automations$/);
    await expect(
      page.getByRole("heading", { name: "Automations", exact: true })
    ).toBeVisible();
  });

  test("extensions item expands sub-routes and navigates to a sub-item", async ({ page }) => {
    await page.goto("/");
    const extensionsItem = sidebar(page).getByRole("link", { name: "Extensions" });
    // Sub-route list is collapsed until the section route is active.
    await expect(sidebar(page).getByRole("link", { name: "Registry" })).toHaveCount(0);
    await extensionsItem.click();
    // Expandable items navigate to their first sub-route.
    await expect(page).toHaveURL(/\/extensions\/registry$/);
    await expect(page.getByRole("heading", { name: "Available extensions" })).toBeVisible();
    // The expanded list shows every extensions sub-route.
    for (const label of ["Registry", "Channels", "Tools"]) {
      await expect(sidebar(page).getByRole("link", { name: label })).toBeVisible();
    }
    await sidebar(page).getByRole("link", { name: "Channels" }).click();
    await expect(page).toHaveURL(/\/extensions\/channels$/);
    // Breadcrumb in the page header reflects the sub-route.
    await expect(page.getByRole("banner")).toContainText("Extensions");
    await expect(page.getByRole("banner")).toContainText("Channels");
  });

  test("settings item expands sub-routes and navigates to a sub-item", async ({ page }) => {
    await page.goto("/");
    await expect(sidebar(page).getByRole("link", { name: "Appearance" })).toHaveCount(0);
    await sidebar(page).getByRole("link", { name: "Settings" }).click();
    // Demo session is admin, so the first settings sub-route is Inference.
    await expect(page).toHaveURL(/\/settings\/inference$/);
    await expect(page.getByRole("heading", { name: "LLM providers" })).toBeVisible();
    for (const label of ["Inference", "Appearance", "Tools", "Skills", "Trace Commons", "Language"]) {
      await expect(
        sidebar(page).getByRole("navigation").getByRole("link", { name: label })
      ).toBeVisible();
    }
    await sidebar(page).getByRole("link", { name: "Appearance" }).click();
    await expect(page).toHaveURL(/\/settings\/appearance$/);
    await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();
  });

  test("admin item expands sub-routes and navigates to a sub-item", async ({ page }) => {
    await page.goto("/");
    await expect(sidebar(page).getByRole("link", { name: "Configuration" })).toHaveCount(0);
    await sidebar(page).getByRole("link", { name: "Admin" }).click();
    await expect(page).toHaveURL(/\/admin\/users$/);
    await expect(page.getByRole("heading", { name: /^Users \(/ })).toBeVisible();
    for (const label of ["Users", "Configuration"]) {
      await expect(
        sidebar(page).getByRole("navigation").getByRole("link", { name: label })
      ).toBeVisible();
    }
    await sidebar(page).getByRole("link", { name: "Configuration" }).click();
    await expect(page).toHaveURL(/\/admin\/configuration$/);
    await expect(
      page.getByRole("heading", { name: "Extension configuration" })
    ).toBeVisible();
  });
});
