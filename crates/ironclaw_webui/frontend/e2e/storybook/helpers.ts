/**
 * Shared helpers for the Storybook component e2e specs.
 *
 * Stories are driven directly through Storybook's iframe.html so each spec
 * lands on the bare story render (no manager chrome). Story ids follow
 * Storybook's title slug convention: "Extras/ContextMenu" story "Default"
 * → "extras-contextmenu--default".
 */
import type { Page } from "@playwright/test";

export async function gotoStory(
  page: Page,
  storyId: string,
  options: { theme?: "dark" | "light" } = {}
) {
  const theme = options.theme ?? "dark";
  await page.goto(
    `/iframe.html?id=${storyId}&viewMode=story&globals=theme:${theme}`
  );
  // Storybook mounts the story into #storybook-root; wait for real content.
  await page.locator("#storybook-root > *").first().waitFor();
}

/** Computed-style helper: read one CSS property from a locator. */
export async function computedStyle(
  locator: import("@playwright/test").Locator,
  property: string
): Promise<string> {
  return locator.evaluate(
    (element, prop) => getComputedStyle(element).getPropertyValue(prop),
    property
  );
}
