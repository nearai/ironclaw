export function interpolateParams(text, params = {}) {
  if (!params || typeof text !== "string") return text;
  return text.replace(/\{(\w+)\}/g, (match, name) =>
    params[name] !== undefined ? params[name] : match,
  );
}
