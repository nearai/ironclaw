# Optional streaming cadence canary

Use this recipe only when the PR changes chat, SSE/WebSocket delivery,
incremental rendering, or live-update reconciliation. Use fresh `var` names
when the Node REPL already contains these bindings. Adapt role names after
inspecting the live DOM snapshot.

## Fresh-thread prompt

```text
Compare and contrast NEAR Protocol with Ethereum using current web research.
Briefly tell me what you will research before using at least three tools,
include another short progress update between tool phases, then write a
detailed final answer of at least ten short paragraphs. I am testing smooth
streaming, so stream naturally and do not make the answer terse.
```

## Fresh-thread sampler

Assume the browser skill produced a tab binding named `railwayTab`.

```js
var railwayPrompt = "Compare and contrast NEAR Protocol with Ethereum using current web research. Briefly tell me what you will research before using at least three tools, include another short progress update between tool phases, then write a detailed final answer of at least ten short paragraphs. I am testing smooth streaming, so stream naturally and do not make the answer terse.";
var railwayBox = railwayTab.playwright.getByRole("textbox", {
  name: "Ask IronClaw anything.",
});
await railwayBox.fill(railwayPrompt);
await railwayTab.playwright.getByRole("button", {
  name: "Send message",
}).click();
var railwayStarted = Date.now();
// Baseline AFTER submission: the user message and static post-submit DOM
// markup are not streaming evidence. Only growth after this signature counts
// as assistant response streaming.
var railwayPrevious = (
  await railwayTab.playwright.locator("main p").allTextContents()
).join("\n");
var railwayChanges = [];
while (Date.now() - railwayStarted < 55_000) {
  var railwayParts = await railwayTab.playwright
    .locator("main p")
    .allTextContents();
  var railwaySignature = railwayParts.join("\n");
  if (railwaySignature !== railwayPrevious) {
    railwayChanges.push({
      t: Date.now() - railwayStarted,
      len: railwaySignature.length,
      parts: railwayParts.length,
      tail: railwaySignature.slice(-140),
    });
    railwayPrevious = railwaySignature;
  }
  await new Promise((resolve) => setTimeout(resolve, 100));
}
nodeRepl.write(JSON.stringify({
  count: railwayChanges.length,
  first: railwayChanges.slice(0, 12),
  last: railwayChanges.slice(-20),
}));
```

## Existing-thread follow-up prompt

```text
Follow up by using at least two web searches to verify the newest roadmap
milestones for both protocols. Give one short progress sentence before the
tools, then a fresh final answer of at least eight short paragraphs. Keep the
final answer flowing naturally so I can observe streaming.
```

## Existing-thread sampler

```js
var railwayFollowup = "Follow up by using at least two web searches to verify the newest roadmap milestones for both protocols. Give one short progress sentence before the tools, then a fresh final answer of at least eight short paragraphs. Keep the final answer flowing naturally so I can observe streaming.";
var railwayFollowupBox = railwayTab.playwright.getByRole("textbox", {
  name: "Ask for follow-up changes",
});
await railwayFollowupBox.fill(railwayFollowup);
await railwayTab.playwright.getByRole("button", {
  name: "Send message",
}).click();
var railwayFollowupStarted = Date.now();
var railwayFollowupPrevious = (
  await railwayTab.playwright.locator("main p").allTextContents()
).join("\n");
var railwayFollowupChanges = [];
while (Date.now() - railwayFollowupStarted < 55_000) {
  var railwayFollowupParts = await railwayTab.playwright
    .locator("main p")
    .allTextContents();
  var railwayFollowupSignature = railwayFollowupParts.join("\n");
  if (railwayFollowupSignature !== railwayFollowupPrevious) {
    railwayFollowupChanges.push({
      t: Date.now() - railwayFollowupStarted,
      len: railwayFollowupSignature.length,
      parts: railwayFollowupParts.length,
      tail: railwayFollowupSignature.slice(-140),
    });
    railwayFollowupPrevious = railwayFollowupSignature;
  }
  await new Promise((resolve) => setTimeout(resolve, 100));
}
nodeRepl.write(JSON.stringify({
  count: railwayFollowupChanges.length,
  first: railwayFollowupChanges.slice(0, 12),
  last: railwayFollowupChanges.slice(-20),
}));
```

## Completion inspection

```js
var railwaySnapshot = await railwayTab.playwright.domSnapshot();
var railwayButtons = await railwayTab.playwright
  .getByRole("button")
  .allTextContents();
var railwayParagraphs = await railwayTab.playwright
  .locator("main p")
  .allTextContents();
nodeRepl.write(JSON.stringify({
  terminalComposer: railwaySnapshot.includes("Ask for follow-up changes"),
  activityButtons: railwayButtons.filter((text) =>
    text.includes("Activity -")
  ),
  paragraphCount: railwayParagraphs.length,
  tail: railwayParagraphs.slice(-10),
}));
```
