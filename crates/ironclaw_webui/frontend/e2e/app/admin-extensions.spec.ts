// Admin, extensions, automations, logs, and jobs surfaces against the demo
// fixtures (src/demo/fixtures/system/* and src/demo/fixtures/work/*).
import { expect, test } from "@playwright/test";

function adminUserRow(page, name) {
  return page
    .getByRole("button", { name })
    .locator(
      "xpath=ancestor::div[.//button[@data-testid='admin-user-suspend'] or .//button[@data-testid='admin-user-activate']][1]"
    );
}

test.describe("admin (demo mode)", () => {
  test("users tab renders fixture users and the status filter works", async ({
    page,
  }) => {
    await page.goto("/admin/users");

    await expect(page.getByText("Users (6 / 6)")).toBeVisible();
    for (const name of [
      "Avery Chen",
      "Mira Oduya",
      "Jonas Lindqvist",
      "Priya Raman",
      "Tomás Vega",
      "Noor Hassan",
    ]) {
      await expect(page.getByRole("button", { name })).toBeVisible();
    }

    // Tomás Vega is the only suspended fixture user.
    await page.getByRole("button", { name: "Suspended", exact: true }).click();
    await expect(page.getByText("Users (1 / 6)")).toBeVisible();
    await expect(page.getByRole("button", { name: "Tomás Vega" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Avery Chen" })).toBeHidden();
  });

  test("role and status controls respond: promote a member, activate a suspended user", async ({
    page,
  }) => {
    await page.goto("/admin/users");

    // Promote Priya Raman (member → admin); the demo route echoes the change.
    const priyaRow = adminUserRow(page, "Priya Raman");
    await expect(priyaRow.getByText("Member", { exact: true })).toBeVisible();
    await priyaRow.getByTestId("admin-user-role").click();
    await expect(priyaRow.getByText("Admin", { exact: true })).toBeVisible();
    await expect(priyaRow.getByTestId("admin-user-role")).toHaveText("Demote");

    // Activate the suspended Tomás Vega.
    const tomasRow = adminUserRow(page, "Tomás Vega");
    await expect(tomasRow.getByText("Suspended", { exact: true })).toBeVisible();
    await tomasRow.getByTestId("admin-user-activate").click();
    await expect(tomasRow.getByText("Active", { exact: true })).toBeVisible();
    await expect(tomasRow.getByTestId("admin-user-suspend")).toBeVisible();
  });

  test("configuration tab renders the fixture extension-configuration groups", async ({
    page,
  }) => {
    await page.goto("/admin/configuration");

    await expect(page.getByText("Slack app credentials")).toBeVisible();
    await expect(page.getByText("GitHub App", { exact: true })).toBeVisible();
  });
});

test.describe("extensions (demo mode)", () => {
  test("registry tab renders installed + available fixture extensions and search filters", async ({
    page,
  }) => {
    await page.goto("/extensions/registry");

    await expect(page.getByText("Installed", { exact: true })).toBeVisible();
    for (const id of [
      "nearai.slack",
      "nearai.telegram",
      "nearai.github",
      "community.postgres-mcp",
      "nearai.browser",
    ]) {
      await expect(
        page.locator(`[data-testid="extension-card"][data-extension-id="${id}"]`)
      ).toBeVisible();
    }

    await expect(page.getByText("Available extensions")).toBeVisible();
    for (const id of ["nearai.discord", "community.linear-mcp", "nearai.weather"]) {
      await expect(
        page.locator(`[data-testid="extension-card"][data-extension-id="${id}"]`)
      ).toBeVisible();
    }

    // Search narrows the catalog.
    await page.getByPlaceholder("Search extensions…").fill("weather");
    await expect(
      page.locator('[data-testid="extension-card"][data-extension-id="nearai.weather"]')
    ).toBeVisible();
    await expect(
      page.locator('[data-testid="extension-card"][data-extension-id="nearai.slack"]')
    ).toBeHidden();
  });

  test("channels tab renders installed messaging channels and available channel registry", async ({
    page,
  }) => {
    await page.goto("/extensions/channels");

    await expect(page.getByText("Messaging channels")).toBeVisible();
    await expect(
      page.locator('[data-testid="extension-card"][data-extension-id="nearai.slack"]')
    ).toBeVisible();
    await expect(
      page.locator('[data-testid="extension-card"][data-extension-id="nearai.telegram"]')
    ).toBeVisible();

    await expect(page.getByText("Available channels")).toBeVisible();
    await expect(
      page.locator('[data-testid="extension-card"][data-extension-id="nearai.discord"]')
    ).toBeVisible();
    await expect(
      page.locator('[data-testid="extension-card"][data-extension-id="nearai.email"]')
    ).toBeVisible();
  });

  test("tools tab renders tool-surface extensions", async ({ page }) => {
    await page.goto("/extensions/tools");

    for (const id of ["nearai.github", "community.postgres-mcp", "nearai.browser"]) {
      await expect(
        page.locator(`[data-testid="extension-card"][data-extension-id="${id}"]`)
      ).toBeVisible();
    }
    // GitHub card exposes its capability count from the fixture tool list.
    await expect(
      page
        .locator('[data-testid="extension-card"][data-extension-id="nearai.github"]')
        .getByText("5 capabilities")
    ).toBeVisible();
  });
});

test.describe("automations (demo mode)", () => {
  test("automations list renders fixture rows and selecting a row updates the detail panel", async ({
    page,
  }) => {
    await page.goto("/automations");

    for (const id of [
      "auto-morning-brief",
      "auto-pr-digest",
      "auto-dep-audit",
      "auto-log-compaction",
      "auto-release-reminder",
      "auto-social-sweep",
    ]) {
      await expect(
        page.locator(`[data-testid="automation-row"][data-automation-id="${id}"]`)
      ).toBeVisible();
    }

    await page
      .locator('[data-testid="automation-name-button"][data-automation-id="auto-pr-digest"]')
      .click();
    await expect(
      page.getByTestId("automation-detail-panel").getByTestId("automation-detail-title")
    ).toHaveText("Open PR digest");
  });

  test("pausing and resuming an automation flips its action button", async ({
    page,
  }) => {
    await page.goto("/automations");

    // Community mentions sweep is seeded paused → resume it.
    await page
      .locator('[data-testid="automation-name-button"][data-automation-id="auto-social-sweep"]')
      .click();
    const detail = page.getByTestId("automation-detail-panel");
    await detail
      .getByRole("button", { name: "Resume: Community mentions sweep" })
      .click();

    // The demo route flips the state to scheduled; the action becomes Pause.
    await expect(
      detail.getByRole("button", { name: "Pause: Community mentions sweep" })
    ).toBeVisible();

    // And pause it again.
    await detail
      .getByRole("button", { name: "Pause: Community mentions sweep" })
      .click();
    await expect(
      detail.getByRole("button", { name: "Resume: Community mentions sweep" })
    ).toBeVisible();
  });
});

test.describe("logs and jobs (demo mode)", () => {
  test("logs page renders fixture entries and the level filter narrows them", async ({
    page,
  }) => {
    await page.goto("/logs");

    const entries = page.getByTestId("logs-entry");
    await expect(entries.first()).toBeVisible();
    await expect(
      page
        .getByTestId("logs-entry-message")
        .filter({ hasText: "Gateway listening on 0.0.0.0:8080" })
    ).toBeVisible();

    // Filter to errors only: every remaining row is an ERROR row.
    await page.locator("select").first().selectOption("error");
    await expect(
      page
        .getByTestId("logs-entry-message")
        .filter({ hasText: "job-7f3a timed out after 1800 s" })
    ).toBeVisible();
    await expect(
      page
        .getByTestId("logs-entry-message")
        .filter({ hasText: "Gateway listening on 0.0.0.0:8080" })
    ).toBeHidden();
  });

  test("jobs page renders fixture jobs and search filters the queue", async ({
    page,
  }) => {
    await page.goto("/jobs");

    await expect(page.getByText("Job queue")).toBeVisible();
    await expect(
      page.getByText("Sandbox build: ironclaw v0.9 release candidate")
    ).toBeVisible();
    await expect(page.getByText("Generate API docs for @ironclaw/ui")).toBeVisible();

    await page.getByPlaceholder("Search job title or UUID").fill("Nightly");
    await expect(page.getByText("Nightly dependency audit")).toBeVisible();
    await expect(
      page.getByText("Sandbox build: ironclaw v0.9 release candidate")
    ).toBeHidden();
  });
});
