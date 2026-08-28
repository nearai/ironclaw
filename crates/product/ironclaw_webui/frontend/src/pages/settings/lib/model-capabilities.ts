const KNOWN_MODALITIES = new Set(["text", "image", "audio", "video", "embedding"]);

function normalizedModelIds(values: unknown): string[] {
  if (!Array.isArray(values)) return [];
  const seen = new Set<string>();
  const models: string[] = [];
  for (const value of values) {
    const model = String(value ?? "").trim();
    if (!model || seen.has(model)) continue;
    seen.add(model);
    models.push(model);
  }
  return models;
}

function normalizedModalities(values: unknown): string[] {
  if (!Array.isArray(values)) return [];
  const seen = new Set<string>();
  const modalities: string[] = [];
  for (const value of values) {
    const modality = String(value ?? "").trim().toLowerCase();
    if (!KNOWN_MODALITIES.has(modality) || seen.has(modality)) continue;
    seen.add(modality);
    modalities.push(modality);
  }
  return modalities;
}

export type ModelCatalogEntry = {
  id: string;
  input_modalities: string[];
  output_modalities: string[];
};

export function normalizeModelCatalog(source: unknown): {
  models: string[];
  modelEntries: ModelCatalogEntry[];
} {
  const record = source && typeof source === "object" ? source as Record<string, unknown> : {};
  const sourceEntries = Array.isArray(record.model_entries) ? record.model_entries : [];
  const entriesById = new Map<string, ModelCatalogEntry>();
  for (const value of sourceEntries) {
    if (!value || typeof value !== "object") continue;
    const entry = value as Record<string, unknown>;
    const id = String(entry.id ?? "").trim();
    if (!id || entriesById.has(id)) continue;
    entriesById.set(id, {
      id,
      input_modalities: normalizedModalities(entry.input_modalities),
      output_modalities: normalizedModalities(entry.output_modalities),
    });
  }

  const explicitModels = normalizedModelIds(record.models ?? record.allowed_models);
  const models = explicitModels.length > 0
    ? explicitModels
    : Array.from(entriesById.keys());
  return {
    models,
    modelEntries: models.map((id) => entriesById.get(id) ?? {
      id,
      input_modalities: [],
      output_modalities: [],
    }),
  };
}

export function modelEntryFor(
  entries: readonly ModelCatalogEntry[] | null | undefined,
  model: string | null | undefined,
): ModelCatalogEntry | null {
  const id = String(model ?? "").trim();
  return entries?.find((entry) => entry.id === id) ?? null;
}

export function modelEntriesForIds(
  entries: readonly ModelCatalogEntry[] | null | undefined,
  models: readonly string[],
): ModelCatalogEntry[] {
  const byId = new Map((entries ?? []).map((entry) => [entry.id, entry]));
  return models.flatMap((model) => {
    const entry = byId.get(model);
    return entry &&
      (entry.input_modalities.length > 0 || entry.output_modalities.length > 0)
      ? [entry]
      : [];
  });
}

export function mergeModelEntries(
  ...collections: Array<readonly ModelCatalogEntry[] | null | undefined>
): ModelCatalogEntry[] {
  const byId = new Map<string, ModelCatalogEntry>();
  for (const entries of collections) {
    for (const entry of entries ?? []) {
      const current = byId.get(entry.id);
      const hasCapabilities =
        entry.input_modalities.length > 0 || entry.output_modalities.length > 0;
      if (!current || hasCapabilities) byId.set(entry.id, entry);
    }
  }
  return Array.from(byId.values());
}

export function capabilityLabels(entry: ModelCatalogEntry | null | undefined): string[] {
  if (!entry) return [];
  const labels: string[] = [];
  if (entry.input_modalities.includes("text") || entry.output_modalities.includes("text")) {
    labels.push("Text");
  }
  if (entry.input_modalities.includes("image")) labels.push("Image input");
  if (entry.output_modalities.includes("image")) labels.push("Image output");
  return labels;
}
