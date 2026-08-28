import assert from "node:assert/strict";
import { test } from "vitest";
import {
  capabilityLabels,
  modelEntryFor,
  normalizeModelCatalog,
} from "./model-capabilities";

test("legacy model arrays normalize to entries with no capabilities", () => {
  const catalog = normalizeModelCatalog({ models: ["model-a", "model-a", " model-b "] });

  assert.deepEqual(catalog.models, ["model-a", "model-b"]);
  assert.deepEqual(catalog.modelEntries, [
    { id: "model-a", input_modalities: [], output_modalities: [] },
    { id: "model-b", input_modalities: [], output_modalities: [] },
  ]);
});
test("detailed entries preserve known directional modalities and ignore unknown values", () => {
  const catalog = normalizeModelCatalog({
    models: ["vision-model"],
    model_entries: [
      {
        id: "vision-model",
        input_modalities: ["text", "image", "image", "future-input"],
        output_modalities: ["text", "image", "future-output"],
      },
    ],
  });

  assert.deepEqual(catalog.modelEntries, [
    {
      id: "vision-model",
      input_modalities: ["text", "image"],
      output_modalities: ["text", "image"],
    },
  ]);
  assert.deepEqual(capabilityLabels(catalog.modelEntries[0]), [
    "Text",
    "Image input",
    "Image output",
  ]);
  assert.equal(modelEntryFor(catalog.modelEntries, "vision-model")?.id, "vision-model");
});
