import { React, html } from "./html.js";

const STORAGE_KEY = "ironclaw_language";

function detectLanguage() {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return saved;
  } catch (_) {}
  const nav = navigator.language || "";
  if (nav.startsWith("es")) return "es";
  if (nav.startsWith("fr")) return "fr";
  if (nav.startsWith("de")) return "de";
  if (nav.startsWith("pt")) return "pt-BR";
  if (nav.startsWith("ja")) return "ja";
  if (nav.startsWith("ar")) return "ar";
  if (nav.startsWith("hi")) return "hi";
  if (nav.startsWith("uk")) return "uk";
  if (nav.startsWith("zh")) return "zh-CN";
  if (nav.startsWith("ko")) return "ko";
  return "en";
}

const packs = {};

export function registerPack(lang, translations) {
  packs[lang] = translations;
}

function translate(lang, key, params = {}) {
  const text = packs[lang]?.[key] || packs["en"]?.[key] || key;
  if (!params || typeof text !== "string") return text;
  return text.replace(/\{(\w+)\}/g, (match, k) => (params[k] !== undefined ? params[k] : match));
}

const I18nContext = React.createContext({
  lang: "en",
  setLang: () => {},
  t: (key, params) => translate("en", key, params),
});

export function I18nProvider({ children }) {
  const [lang, setLangState] = React.useState(detectLanguage);

  const setLang = React.useCallback((next) => {
    if (!packs[next]) return;
    setLangState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch (_) {}
    document.documentElement.lang = next;
  }, []);

  React.useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  const t = React.useCallback(
    (key, params) => translate(lang, key, params),
    [lang]
  );

  const ctx = React.useMemo(() => ({ lang, setLang, t }), [lang, setLang, t]);

  return html`<${I18nContext.Provider} value=${ctx}>${children}<//>`;
}

export function useI18n() {
  return React.useContext(I18nContext);
}

export function useT() {
  return React.useContext(I18nContext).t;
}

export const AVAILABLE_LANGUAGES = [
  { code: "en", name: "English", native: "English" },
  { code: "es", name: "Spanish", native: "Español" },
  { code: "fr", name: "French", native: "Français" },
  { code: "de", name: "German", native: "Deutsch" },
  { code: "pt-BR", name: "Portuguese (Brazil)", native: "Português (Brasil)" },
  { code: "ja", name: "Japanese", native: "日本語" },
  { code: "ar", name: "Arabic", native: "العربية" },
  { code: "hi", name: "Hindi", native: "हिन्दी" },
  { code: "uk", name: "Ukrainian", native: "Українська" },
  { code: "zh-CN", name: "Chinese (Simplified)", native: "简体中文" },
  { code: "ko", name: "Korean", native: "한국어" },
];
