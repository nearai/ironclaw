/**
 * Direct-URL route coverage for the agent workspace shell (demo mode).
 *
 * Every registered route from src/app/routes.ts (including hidden ones that
 * are only reachable by URL) must load without a blank page or the route
 * error boundary, and render meaningful page content. Also covers the
 * unknown-URL redirect to /chat and the /welcome onboarding page.
 */
import { expect, test, type Page } from "@playwright/test";

async function expectNoErrorState(page: Page) {
  // Route chunk-load failures render this boundary (src/app/route-load-boundary.tsx).
  await expect(page.getByText("This page couldn't be loaded")).toBeHidden();
  await expect(page.getByTestId("session-check-error")).toBeHidden();
}

type RouteCase = {
  path: string;
  assert: (page: Page) => Promise<void>;
};

const routeCases: RouteCase[] = [
  {
    path: "/chat",
    assert: async (page) => {
      await expect(
        page.getByRole("heading", { name: "Hello, what do you need help with?" })
      ).toBeVisible();
    },
  },
  {
    path: "/workspace",
    assert: async (page) => {
      await expect(page.getByRole("heading", { name: "Workspace" })).toBeVisible();
    },
  },
  {
    path: "/projects",
    assert: async (page) => {
      await expect(page.getByRole("heading", { name: "Scoped projects" })).toBeVisible();
    },
  },
  {
    path: "/jobs",
    assert: async (page) => {
      await expect(page.getByRole("heading", { name: "Job queue" })).toBeVisible();
    },
  },
  {
    path: "/routines",
    assert: async (page) => {
      await expect(
        page.getByRole("heading", { name: "Routines", exact: true })
      ).toBeVisible();
    },
  },
  {
    path: "/automations",
    assert: async (page) => {
      await expect(
        page.getByRole("heading", { name: "Automations", exact: true })
      ).toBeVisible();
    },
  },
  {
    path: "/missions",
    assert: async (page) => {
      await expect(page.getByRole("heading", { name: "Execution loops" })).toBeVisible();
    },
  },
  {
    path: "/extensions",
    assert: async (page) => {
      await expect(
        page.getByRole("heading", { name: "Installed", exact: true })
      ).toBeVisible();
    },
  },
  {
    path: "/extensions/registry",
    assert: async (page) => {
      await expect(page.getByRole("heading", { name: "Available extensions" })).toBeVisible();
    },
  },
  {
    path: "/logs",
    assert: async (page) => {
      // The logs surface has no heading; its toolbar controls are the landmark.
      await expect(page.getByRole("banner")).toContainText("Logs");
      await expect(page.getByText("Auto-scroll")).toBeVisible();
      await expect(page.getByPlaceholder("Filter by target…")).toBeVisible();
    },
  },
  {
    path: "/settings",
    assert: async (page) => {
      await expect(page.getByRole("heading", { name: "LLM providers" })).toBeVisible();
    },
  },
  {
    path: "/settings/appearance",
    assert: async (page) => {
      await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();
      await expect(page.getByRole("radiogroup")).toBeVisible();
    },
  },
  {
    path: "/admin",
    assert: async (page) => {
      await expect(page.getByRole("heading", { name: /^Users \(/ })).toBeVisible();
    },
  },
];

test.describe("registered routes load by direct URL", () => {
  for (const { path, assert } of routeCases) {
    test(`${path} renders page content`, async ({ page }) => {
      await page.goto(path);
      await expect(page).toHaveURL(new RegExp(`${path.replaceAll("/", "\\/")}$`));
      await assert(page);
      await expectNoErrorState(page);
    });
  }

  test("/welcome renders the onboarding page", async ({ page }) => {
    await page.goto("/welcome");
    await expect(page).toHaveURL(/\/welcome$/);
    await expect(page.getByRole("heading", { name: "Welcome to IronClaw" })).toBeVisible();
    await expectNoErrorState(page);
  });

  test("unknown URL redirects to /chat", async ({ page }) => {
    await page.goto("/this-route-does-not-exist");
    await expect(page).toHaveURL(/\/chat$/);
    await expect(
      page.getByRole("heading", { name: "Hello, what do you need help with?" })
    ).toBeVisible();
  });
});
