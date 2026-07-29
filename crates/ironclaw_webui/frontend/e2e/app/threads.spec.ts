// Sidebar thread list flows (src/components/sidebar-threads.tsx) against the
// demo fixtures in src/demo/routes/core.ts: four seeded threads, newest first.
import { expect, test } from "@playwright/test";

const FIXTURE_THREADS = [
  "Draft the v0.9 release notes",
  "Why did the sandbox job time out?",
  "Summarize yesterday's activity",
  "Wire up the Slack extension",
];

test.describe("sidebar threads (demo mode)", () => {
  test("fixture threads render under the Recent group", async ({ page }) => {
    await page.goto("/chat");

    await expect(page.getByText("Recent", { exact: true })).toBeVisible();
    for (const title of FIXTURE_THREADS) {
      await expect(page.getByRole("button", { name: title })).toBeVisible();
    }
  });

  test("clicking a thread navigates to /chat/:threadId and renders its timeline", async ({
    page,
  }) => {
    await page.goto("/chat");

    await page
      .getByRole("button", { name: "Why did the sandbox job time out?" })
      .click();

    await expect(page).toHaveURL(/\/chat\/thread-sandbox-triage$/);
    await expect(
      page.getByTestId("msg-user").filter({
        hasText: "Job job-7f3a timed out after 30m. Can you find out why?",
      })
    ).toBeVisible();
    await expect(
      page.getByTestId("msg-assistant").filter({
        hasText: "The build stage stalled resolving a git dependency",
      })
    ).toBeVisible();
  });

  test("pinning a thread moves it into the Pinned group and unpinning removes it", async ({
    page,
  }) => {
    await page.goto("/chat");

    const threadRow = page
      .getByRole("button", { name: "Wire up the Slack extension" })
      .locator("..");
    await expect(threadRow).toBeVisible();
    await expect(page.getByText("Pinned", { exact: true })).toBeHidden();

    await threadRow.hover();
    const pinButton = threadRow.getByRole("button", { name: "Pin", exact: true });
    await pinButton.click();

    await expect(page.getByText("Pinned", { exact: true })).toBeVisible();
    await expect(
      threadRow.getByRole("button", { name: "Unpin", exact: true })
    ).toHaveAttribute("aria-pressed", "true");

    await threadRow.hover();
    await threadRow.getByRole("button", { name: "Unpin", exact: true }).click();
    await expect(page.getByText("Pinned", { exact: true })).toBeHidden();
  });

  test("thread search filters the list and reports empty matches", async ({
    page,
  }) => {
    await page.goto("/chat");

    const search = page.getByPlaceholder("Search chats...");
    await search.fill("sandbox");

    await expect(
      page.getByRole("button", { name: "Why did the sandbox job time out?" })
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Draft the v0.9 release notes" })
    ).toBeHidden();

    await search.fill("zzz-no-such-thread");
    await expect(
      page.getByText('No chats match "zzz-no-such-thread"')
    ).toBeVisible();

    await search.fill("");
    await expect(
      page.getByRole("button", { name: "Draft the v0.9 release notes" })
    ).toBeVisible();
  });

  test("New chat returns to a fresh landing composer", async ({ page }) => {
    await page.goto("/chat/thread-standup");
    await expect(
      page.getByTestId("msg-user").filter({
        hasText: "Give me a standup summary of what ran yesterday.",
      })
    ).toBeVisible();

    await page.getByTestId("new-chat").click();

    await expect(page).toHaveURL(/\/chat$/);
    await expect(
      page.getByRole("heading", { name: "Hello, what do you need help with?" })
    ).toBeVisible();
    await expect(page.getByTestId("chat-composer")).toHaveValue("");
  });
});
