export function normalizeSearchQuery(query) {
  return String(query || "").trim().toLowerCase();
}

function stringifySearchValue(value) {
  if (value === null || value === undefined) return "";
  if (Array.isArray(value)) return value.map(stringifySearchValue).join(" ");
  if (typeof value === "object") {
    try {
      return JSON.stringify(value);
    } catch (_) {
      return "";
    }
  }
  return String(value);
}

export function matchesSearch(query, values) {
  const normalized = normalizeSearchQuery(query);
  if (!normalized) return true;
  return values
    .map(stringifySearchValue)
    .join(" ")
    .toLowerCase()
    .includes(normalized);
}

export function filterSettingsSections(sections, settings, searchQuery, t) {
  const normalized = normalizeSearchQuery(searchQuery);
  if (!normalized) return sections;

  return sections
    .map((section) => {
      const groupLabel = section.groupKey ? t(section.groupKey) : "";
      const fields = section.fields.filter((field) =>
        matchesSearch(normalized, [
          groupLabel,
          field.key,
          field.labelKey ? t(field.labelKey) : field.label,
          field.descKey ? t(field.descKey) : field.description,
          settings[field.key],
        ])
      );
      return { ...section, fields };
    })
    .filter((section) => section.fields.length > 0);
}
