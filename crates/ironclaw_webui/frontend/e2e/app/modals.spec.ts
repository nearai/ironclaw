// Modal / confirm-dialog behavior across surfaces. Demo fixtures are
// in-memory per page load, so a confirmed mutation (thread/automation delete,
// user suspend) persists only within the test's own browser context.
import { expect, test } from "@playwright/test";

test.describe("modals and confirm dialogs (demo mode)", () => {
  test("thread delete confirm: Escape and Cancel keep the thread, Confirm removes it", async ({
    page,
  }) => {
    await page.goto("/chat");

    const threadButton = page.getByRole("button", {
      name: "Wire up the Slack extension",
    });
    await expect(threadButton).toBeVisible();

    const deleteTrigger = page.locator(
      '[data-testid="thread-delete"][data-thread-id="thread-onboarding"]'
    );
    const dialog = page.getByRole("dialog", { name: "Delete chat" });

    // Escape closes without deleting.
    await threadButton.hover();
    await deleteTrigger.click();
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Delete this chat?")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(threadButton).toBeVisible();

    // Cancel closes without deleting.
    await threadButton.hover();
    await deleteTrigger.click();
    await expect(dialog).toBeVisible();
    await dialog.getByTestId("confirm-dialog-cancel").click();
    await expect(dialog).toBeHidden();
    await expect(threadButton).toBeVisible();

    // Confirm deletes: the row disappears from the sidebar.
    await threadButton.hover();
    await deleteTrigger.click();
    await dialog.getByTestId("confirm-dialog-confirm").click();
    await expect(dialog).toBeHidden();
    await expect(threadButton).toBeHidden();
  });

  test("extensions configure modal opens for a setup-needed extension and closes on Escape / close button", async ({
    page,
  }) => {
    await page.goto("/extensions/registry");

    // PostgreSQL is installed but setup_needed → its card's primary action
    // opens the configure modal.
    const postgresCard = page.locator(
      '[data-testid="extension-card"][data-extension-id="community.postgres-mcp"]'
    );
    await postgresCard.getByRole("button", { name: "Configure" }).click();

    const dialog = page.getByRole("dialog", { name: "Configure PostgreSQL" });
    await expect(dialog).toBeVisible();
    await expect(
      dialog.getByText("PostgreSQL connection string")
    ).toBeVisible();

    // Escape closes.
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();

    // Reopen and close via the header close button.
    await postgresCard.getByRole("button", { name: "Configure" }).click();
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(dialog).toBeHidden();
  });

  test("admin suspend confirm: cancel keeps the user active, confirm suspends", async ({
    page,
  }) => {
    await page.goto("/admin/users");

    // Nearest ancestor of the user's name button that also carries the row
    // actions (suspend/role buttons).
    const row = page
      .getByRole("button", { name: "Jonas Lindqvist" })
      .locator(
        "xpath=ancestor::div[.//button[@data-testid='admin-user-suspend'] or .//button[@data-testid='admin-user-activate']][1]"
      );
    await expect(row.getByText("Active", { exact: true })).toBeVisible();

    const dialog = page.getByTestId("admin-user-confirm-dialog");

    // Cancel: no state change.
    await row.getByTestId("admin-user-suspend").click();
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Suspend user")).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toBeHidden();
    await expect(row.getByText("Active", { exact: true })).toBeVisible();

    // Confirm: the row flips to Suspended and offers Activate.
    await row.getByTestId("admin-user-suspend").click();
    await dialog.getByTestId("admin-user-confirm-submit").click();
    await expect(dialog).toBeHidden();
    await expect(row.getByText("Suspended", { exact: true })).toBeVisible();
    await expect(row.getByTestId("admin-user-activate")).toBeVisible();
  });

  test("automation delete confirm removes the automation from the table", async ({
    page,
  }) => {
    await page.goto("/automations");

    // Select the Morning briefing row explicitly (the default selection is
    // the automation with the soonest next run).
    await page
      .locator('[data-testid="automation-name-button"][data-automation-id="auto-morning-brief"]')
      .click();
    const detail = page.getByTestId("automation-detail-panel");
    await expect(detail.getByTestId("automation-detail-title")).toHaveText(
      "Morning briefing"
    );

    await detail
      .getByRole("button", { name: "Delete: Morning briefing" })
      .click();

    const dialog = page.getByRole("dialog", { name: "Delete: Morning briefing" });
    await expect(dialog).toBeVisible();

    // Cancel keeps the row.
    await dialog.getByTestId("confirm-dialog-cancel").click();
    await expect(dialog).toBeHidden();
    await expect(
      page.locator('[data-testid="automation-row"][data-automation-id="auto-morning-brief"]')
    ).toBeVisible();

    // Confirm removes it.
    await detail
      .getByRole("button", { name: "Delete: Morning briefing" })
      .click();
    await dialog.getByTestId("confirm-dialog-confirm").click();
    await expect(
      page.locator('[data-testid="automation-row"][data-automation-id="auto-morning-brief"]')
    ).toBeHidden();
  });
});
