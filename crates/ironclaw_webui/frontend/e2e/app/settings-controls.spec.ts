// Settings pages against the demo fixtures (src/demo/fixtures/system/*):
// each tab renders its fixture data AND at least one real control is driven,
// asserting the UI response the demo routes echo back.
import { expect, test } from "@playwright/test";

test.describe("settings (demo mode)", () => {
  test("inference tab lists fixture providers and switching the active provider works", async ({
    page,
  }) => {
    await page.goto("/settings/inference");

    // Summary card reflects the fixture active selection.
    await expect(page.getByText("anthropic", { exact: true }).first()).toBeVisible();
    await expect(page.getByText("claude-sonnet-4-5").first()).toBeVisible();

    // All four fixture providers render as cards.
    for (const id of ["anthropic", "nearai", "openai", "workstation-ollama"]) {
      await expect(
        page.locator(`[data-testid="llm-provider-card"][data-provider-id="${id}"]`)
      ).toBeVisible();
    }

    // NEAR AI is configured (api_key_set) — activate it via "Use".
    const nearaiCard = page.locator(
      '[data-testid="llm-provider-card"][data-provider-id="nearai"]'
    );
    await nearaiCard.getByRole("button", { name: "Use", exact: true }).click();

    await expect(nearaiCard.getByText("Active", { exact: true })).toBeVisible();
    // The summary card follows the new snapshot.
    await expect(page.getByText("nearai", { exact: true }).first()).toBeVisible();
    await expect(page.getByText("deepseek-v3.2").first()).toBeVisible();
  });

  test("tools tab: auto-approve switch toggles and a per-tool permission select saves", async ({
    page,
  }) => {
    await page.goto("/settings/tools");

    // Fixture tool rows render.
    const shellRow = page.locator('[data-tool-name="builtin.shell"]');
    await expect(shellRow).toBeVisible();
    await expect(
      page.locator('[data-tool-name="builtin.http"]')
    ).toBeVisible();

    // Global auto-approve switch: fixture starts enabled; toggling saves.
    const autoApprove = page.getByRole("switch", {
      name: "Always allow eligible tools",
    });
    await expect(autoApprove).toHaveAttribute("aria-checked", "true");
    await autoApprove.click();
    await expect(autoApprove).toHaveAttribute("aria-checked", "false");
    await expect(page.getByRole("status").getByText("saved")).toBeVisible();

    // Per-tool permission: builtin.shell (default "Follow global") →
    // "Always allow"; the row confirms the persisted override.
    await shellRow
      .getByRole("button", { name: "Permission for builtin.shell" })
      .click();
    await page.getByRole("option", { name: "Always allow" }).click();

    await expect(shellRow.getByText("saved", { exact: true })).toBeVisible();
    await expect(
      shellRow.getByRole("button", { name: "Permission for builtin.shell" })
    ).toContainText("Always allow");
    await expect(shellRow.getByText("per-tool override")).toBeVisible();
  });

  test("skills tab renders fixture skills and the global auto-activation default toggles", async ({
    page,
  }) => {
    await page.goto("/settings/skills");

    // Fixture skills render in their source groups.
    await expect(page.getByText("Your skills", { exact: true })).toBeVisible();
    await expect(page.getByText("release-notes", { exact: true })).toBeVisible();
    await expect(page.getByText("pr-triage", { exact: true })).toBeVisible();
    await expect(page.getByText("System skills", { exact: true })).toBeVisible();
    await expect(
      page.getByText("incident-response", { exact: true })
    ).toBeVisible();
    await expect(
      page.getByText("Workspace skills", { exact: true })
    ).toBeVisible();
    await expect(page.getByText("brand-voice", { exact: true })).toBeVisible();

    // Global default auto-activation: fixture starts ON; toggle it off.
    await expect(
      page.getByText("Default skill auto-activation enabled")
    ).toBeVisible();
    await page.getByRole("button", { name: "Default: On" }).click();

    await expect(
      page.getByText("Default skill auto-activation disabled")
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Default: Off" })).toBeVisible();
    await expect(page.getByTestId("skill-action-result")).toContainText(
      "Automatic skill activation disabled."
    );
  });

  test("language tab switches the interface language to Spanish", async ({
    page,
  }) => {
    await page.goto("/settings/language");

    await expect(page.getByRole("heading", { name: "Language" })).toBeVisible();
    await expect(page.getByText("Current language")).toBeVisible();

    await page.getByRole("button", { name: /Español/ }).click();

    // A visible string re-renders in Spanish once the pack loads; the
    // selection persists via localStorage (fresh context per test resets it).
    await expect(page.getByRole("heading", { name: "Idioma" })).toBeVisible();
    const storedLanguage = await page.evaluate(() =>
      localStorage.getItem("ironclaw_language")
    );
    expect(storedLanguage).toBe("es");
  });

  test("trace commons tab renders fixture credits and authorizing a held trace clears it", async ({
    page,
  }) => {
    await page.goto("/settings/traces");

    await expect(
      page.getByRole("heading", { name: "Trace Commons credits" })
    ).toBeVisible();
    await expect(page.getByText("Enrolled", { exact: true })).toBeVisible();
    await expect(page.getByText("86.25", { exact: true })).toBeVisible();
    await expect(
      page.getByText("57 submitted, 51 accepted of 61 total")
    ).toBeVisible();

    // One fixture submission is held for review; authorizing removes it.
    await expect(page.getByText("Held for review")).toBeVisible();
    await expect(page.getByText("sub-9f21c4d8-hold").first()).toBeVisible();

    await page.getByRole("button", { name: "Authorize", exact: true }).click();

    await expect(page.getByText("Held for review")).toBeHidden();
    // The demo fixture counts the authorized hold as a new submission.
    await expect(
      page.getByText("58 submitted, 51 accepted of 61 total")
    ).toBeVisible();
  });
});
