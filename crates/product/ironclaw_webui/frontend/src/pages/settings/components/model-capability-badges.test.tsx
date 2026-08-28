import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";

import { iconNames } from "../../../design-system/icons";
import { ModelCapabilityBadges } from "./model-capability-badges";

test("model capabilities use compact semantic icon chips", () => {
  const markup = renderToStaticMarkup(
    <ModelCapabilityBadges
      entry={{
        id: "vision-model",
        input_modalities: ["text", "image"],
        output_modalities: ["text", "image"],
      }}
    />,
  );

  assert.ok(iconNames.includes("eye"), "the shared icon set should expose the vision glyph");
  assert.match(markup, /data-capability="text"/);
  assert.match(markup, /data-capability="image-input"/);
  assert.match(markup, /data-capability="image-output"/);
  assert.match(markup, /aria-label="Text"[^>]*title="Text"/);
  assert.match(markup, /aria-label="Image input"[^>]*title="Image input"/);
  assert.match(markup, /aria-label="Image output"[^>]*title="Image output"/);
  assert.doesNotMatch(markup, />Text</);
  assert.doesNotMatch(markup, />Image input</);
  assert.doesNotMatch(markup, />Image output</);
  assert.equal(markup.match(/<svg/g)?.length, 3);
  assert.doesNotMatch(markup, /uppercase/);
});
