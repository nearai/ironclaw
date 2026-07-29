/**
 * Sidebar keyboard accessibility and shell affordances (demo mode).
 *
 * Covers Tab order from a fresh page load through the sidebar's interactive
 * items, the focus-visible ring (rendered as a box-shadow via Tailwind's
 * ring utilities), Enter-key navigation on nav links, the header's sidebar
 * collapse/expand toggle (src/components/page-header.tsx), and the active
 * route's accent-highlighted nav item (react-router NavLink aria-current).
 */
import { expect, test, type Page } from "@playwright/test";

function sidebar(page: Page) {
  return page.locator("#gateway-sidebar");
}

async function focusedBoxShadow(page: Page) {
  return page.evaluate(() => {
    const el = document.activeElement;
    if (!(el instanceof HTMLElement)) return "none";
    return window.getComputedStyle(el).boxShadow;
  });
}

test.describe("sidebar keyboard navigation", () => {
  test("Tab reaches sidebar items in order with a visible focus ring", async ({ page }) => {
    await page.goto("/chat");
    await expect(sidebar(page)).toBeVisible();

    // Tab order from page load: brand link → New chat → primary nav items.
    await page.keyboard.press("Tab");
    await expect(sidebar(page).getByRole("link", { name: "NEAR AI" })).toBeFocused();
    expect(await focusedBoxShadow(page)).not.toBe("none");

    await page.keyboard.press("Tab");
    await expect(sidebar(page).getByTestId("new-chat")).toBeFocused();
    expect(await focusedBoxShadow(page)).not.toBe("none");

    await page.keyboard.press("Tab");
    await expect(sidebar(page).getByTestId("nav-workspace")).toBeFocused();
    expect(await focusedBoxShadow(page)).not.toBe("none");

    await page.keyboard.press("Tab");
    await expect(sidebar(page).getByTestId("nav-automations")).toBeFocused();

    for (const name of ["Extensions", "Settings", "Admin"]) {
      await page.keyboard.press("Tab");
      await expect(sidebar(page).getByRole("link", { name })).toBeFocused();
      expect(await focusedBoxShadow(page)).not.toBe("none");
    }
  });

  test("Enter on a focused nav item navigates to its route", async ({ page }) => {
    await page.goto("/chat");
    await sidebar(page).getByTestId("nav-workspace").focus();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/workspace$/);
    await expect(page.getByRole("heading", { name: "Workspace" })).toBeVisible();
  });

  test("header toggle collapses and expands the sidebar", async ({ page }) => {
    await page.goto("/chat");
    const toggle = page.getByRole("button", { name: "Toggle sidebar" });
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(sidebar(page)).toBeVisible();

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(sidebar(page)).toBeHidden();

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(sidebar(page)).toBeVisible();
  });

  test("current route's nav item is marked as the active item", async ({ page }) => {
    await page.goto("/workspace");
    const workspaceItem = sidebar(page).getByTestId("nav-workspace");
    const automationsItem = sidebar(page).getByTestId("nav-automations");

    await expect(workspaceItem).toHaveAttribute("aria-current", "page");
    await expect(automationsItem).not.toHaveAttribute("aria-current", "page");

    // The active item also carries the accent-tinted background class.
    await expect(workspaceItem).toHaveClass(/accent-soft/);
    await expect(automationsItem).not.toHaveClass(/accent-soft/);

    await automationsItem.click();
    await expect(page).toHaveURL(/\/automations$/);
    await expect(automationsItem).toHaveAttribute("aria-current", "page");
    await expect(workspaceItem).not.toHaveAttribute("aria-current", "page");
  });
});
