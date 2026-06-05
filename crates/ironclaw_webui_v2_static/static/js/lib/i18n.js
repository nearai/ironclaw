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

// Lazy loaders for every non-default locale. `en` is bundled eagerly in
// main.js as the fallback (see `translate`), so it has no loader. Each
// module calls `registerPack` as an import side effect, populating
// `packs[lang]`. Literal import paths keep the assets statically
// discoverable. Loaded packs are cached in `packs`, so each fires once.
const loaders = {
  es: () => import("../i18n/es.js"),
  fr: () => import("../i18n/fr.js"),
  de: () => import("../i18n/de.js"),
  "pt-BR": () => import("../i18n/pt-BR.js"),
  ja: () => import("../i18n/ja.js"),
  ar: () => import("../i18n/ar.js"),
  hi: () => import("../i18n/hi.js"),
  uk: () => import("../i18n/uk.js"),
  "zh-CN": () => import("../i18n/zh-CN.js"),
  ko: () => import("../i18n/ko.js"),
};

const pending = {};

// Resolves true once `packs[lang]` is available. Returns false for an
// unknown locale (no loader, not already registered).
export function loadPack(lang) {
  if (packs[lang]) return Promise.resolve(true);
  const loader = loaders[lang];
  if (!loader) return Promise.resolve(false);
  if (!pending[lang]) {
    pending[lang] = loader()
      .then(() => !!packs[lang])
      .catch(() => false);
  }
  return pending[lang];
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
  // Bumped when an async pack finishes loading so `t` (and thus the
  // context value) gets a fresh identity, re-rendering consumers with
  // the now-available translations.
  const [version, setVersion] = React.useState(0);

  const applyLang = React.useCallback((next) => {
    setLangState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch (_) {}
    document.documentElement.lang = next;
  }, []);

  const setLang = React.useCallback(
    (next) => {
      if (packs[next]) {
        applyLang(next);
        return;
      }
      // Defer the switch until the pack is loaded so we never flash the
      // English fallback for an already-selected language.
      loadPack(next).then((ok) => {
        if (ok) {
          setVersion((v) => v + 1);
          applyLang(next);
        }
      });
    },
    [applyLang]
  );

  // The initially-detected language may not be bundled (only `en` is);
  // load it on mount and re-render once its strings are available.
  React.useEffect(() => {
    let cancelled = false;
    if (!packs[lang]) {
      loadPack(lang).then((ok) => {
        if (!cancelled && ok) setVersion((v) => v + 1);
      });
    }
    return () => {
      cancelled = true;
    };
  }, [lang]);

  React.useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  const t = React.useCallback(
    (key, params) => translate(lang, key, params),
    [lang, version]
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
