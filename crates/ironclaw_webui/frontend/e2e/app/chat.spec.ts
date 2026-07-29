// Chat page flows against the demo-mode build (src/demo fixtures).
//
// Demo send behavior (src/demo/routes/core.ts): POSTing a message appends the
// user message to the in-memory timeline, then plays a run lifecycle over the
// inert SSE stream — "accepted" (~350ms) followed by a "final_reply" (~1400ms)
// whose text explains this is demo mode. Both the optimistic user bubble and
// the synthetic assistant reply are asserted below.
import { expect, test } from "@playwright/test";

const DEMO_REPLY_SNIPPET = "demo mode";

test.describe("chat page (demo mode)", () => {
  test("hero composer renders on a fresh /chat with send disabled until there is text", async ({
    page,
  }) => {
    await page.goto("/chat");

    await expect(
      page.getByRole("heading", { name: "Hello, what do you need help with?" })
    ).toBeVisible();

    const composer = page.getByTestId("chat-composer");
    await expect(composer).toBeVisible();
    await expect(composer).toHaveAttribute(
      "placeholder",
      "Triage my inbox every morning"
    );

    const sendButton = page.getByRole("button", { name: "Send message" });
    await expect(sendButton).toBeDisabled();

    await composer.fill("draft");
    await expect(sendButton).toBeEnabled();
  });

  test("Enter sends in an existing thread: user bubble renders, composer clears, demo assistant replies", async ({
    page,
  }) => {
    await page.goto("/chat/thread-onboarding");

    // Seeded timeline renders first.
    await expect(
      page.getByTestId("msg-user").filter({
        hasText: "Help me connect Slack so alerts land in #ops.",
      })
    ).toBeVisible();

    const composer = page.getByTestId("chat-composer");
    await composer.fill("Hello from the e2e suite");
    await composer.press("Enter");

    // The composer clears synchronously on submit.
    await expect(composer).toHaveValue("");

    // Optimistic user bubble.
    await expect(
      page.getByTestId("msg-user").filter({ hasText: "Hello from the e2e suite" })
    ).toBeVisible();

    // Synthetic assistant reply arrives over the demo SSE stream (~1.4s).
    await expect(
      page.getByTestId("msg-assistant").filter({ hasText: DEMO_REPLY_SNIPPET })
    ).toBeVisible();
  });

  test("send button on the hero composer creates a thread and renders the user bubble", async ({
    page,
  }) => {
    await page.goto("/chat");

    const composer = page.getByTestId("chat-composer");
    await composer.fill("Start a brand new conversation");
    await page.getByRole("button", { name: "Send message" }).click();

    // The demo message response carries no thread_id, so the SPA renders the
    // new conversation in place (no /chat/:id navigation) and — because the
    // per-thread SSE stream is keyed off the route — the synthetic assistant
    // reply is not delivered to this view. The eager POST /threads still adds
    // the new thread to the sidebar.
    await expect(composer).toHaveValue("");
    await expect(
      page
        .getByTestId("msg-user")
        .filter({ hasText: "Start a brand new conversation" })
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "New conversation" })
    ).toBeVisible();
  });

  test("Shift+Enter inserts a newline without sending", async ({ page }) => {
    await page.goto("/chat/thread-onboarding");

    const composer = page.getByTestId("chat-composer");
    const userMessages = page.getByTestId("msg-user");
    // Wait for the seeded timeline before counting, then assert no new bubble.
    await expect(userMessages).toHaveCount(1);

    await composer.click();
    await composer.pressSequentially("first line");
    await composer.press("Shift+Enter");
    await composer.pressSequentially("second line");

    await expect(composer).toHaveValue("first line\nsecond line");
    await expect(userMessages).toHaveCount(1);
  });

  test("? opens the keyboard shortcuts dialog and Escape closes it", async ({
    page,
  }) => {
    await page.goto("/chat/thread-onboarding");
    await expect(page.getByTestId("chat-composer")).toBeVisible();

    // The shortcut is ignored while an input/textarea has focus, so make sure
    // focus is on the document body first.
    await page.locator("body").click();
    await page.keyboard.press("?");

    const dialog = page.getByRole("dialog", { name: "Keyboard shortcuts" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Send message")).toBeVisible();
    await expect(dialog.getByText("New line")).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  });
});
