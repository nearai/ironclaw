import React from "react";
import { interpolateParams } from "./i18n-format";

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
  if (packs[lang]) {
    Object.assign(packs[lang], translations);
  } else {
    packs[lang] = translations;
  }
}

// Lazy loaders for every non-default locale. `en` is bundled eagerly in
// main.tsx as the fallback (see `translate`), so it has no loader. Each
// module calls `registerPack` as an import side effect, populating
// `packs[lang]`. Literal import paths keep the assets statically
// discoverable. Loaded packs are cached in `packs`, so each fires once.
const loaders = {
  es: () => import("../i18n/es"),
  fr: () => import("../i18n/fr"),
  de: () => import("../i18n/de"),
  "pt-BR": () => import("../i18n/pt-BR"),
  ja: () => import("../i18n/ja"),
  ar: () => import("../i18n/ar"),
  hi: () => import("../i18n/hi"),
  uk: () => import("../i18n/uk"),
  "zh-CN": () => import("../i18n/zh-CN"),
  ko: () => import("../i18n/ko"),
};

const pending = {};

// Loads and returns the translations pack for `lang` on demand.
// Resolves the pack object once available, or null for a locale with no
// loader and no prior registration. The in-flight promise is cached in
// `pending` and cleared once it settles either way: on success
// `packs[lang]` short-circuits above, and on failure dropping it lets a
// later attempt retry instead of caching a permanent miss. Private —
// the provider owns the only language transition that consumes it.
function ensurePack(lang) {
  if (packs[lang]) return Promise.resolve(packs[lang]);
  const loader = loaders[lang];
  if (!loader) return Promise.resolve(null);
  if (!pending[lang]) {
    pending[lang] = loader()
      .then(() => {
        delete pending[lang];
        return packs[lang] || null;
      })
      .catch(() => {
        delete pending[lang];
        return null;
      });
  }
  return pending[lang];
}

// Resolves a key against the active pack, falling back to the eagerly
// bundled English pack and finally the raw key.
function translate(pack, key, params = {}) {
  const text = pack?.[key] || packs["en"]?.[key] || key;
  return interpolateParams(text, params);
}

const I18nContext = React.createContext({
  lang: "en",
  setLang: (_next = "en") => {},
  t: (key, params = {}) => translate(packs["en"], key, params),
});

export function I18nProvider({ children }) {
  const [lang, setLangState] = React.useState(detectLanguage);
  // The translations committed for the active language. Held in state
  // (rather than read live from the module `packs` registry) so an async
  // pack load re-renders consumers naturally — no separate version
  // counter. `null` means "fall back to English" until the pack lands.
  const [pack, setPack] = React.useState(() => packs[lang] || null);
  // The most recently requested language. A pack load that resolves
  // after a newer request is discarded, so out-of-order imports never
  // commit a stale language.
  const activeLangRef = React.useRef(lang);

  // The single language transition: stamp the request, ensure its pack
  // is loaded, then commit (state + persistence) only if the pack is
  // available AND it is still the latest request when the load resolves.
  // Deferring the commit until the pack lands avoids flashing the English
  // fallback for an already-chosen language; the staleness guard makes
  // out-of-order resolution a no-op; an unknown/failed locale (`loaded`
  // null) never commits, leaving the current language in place.
  const setLang = React.useCallback((next) => {
    activeLangRef.current = next;
    ensurePack(next).then((loaded) => {
      if (!loaded || activeLangRef.current !== next) return;
      setLangState(next);
      setPack(loaded);
      try {
        localStorage.setItem(STORAGE_KEY, next);
      } catch (_) {}
    });
  }, []);

  // The initially-detected language may not be bundled (only `en` is);
  // load its pack on mount and commit it once available, without
  // re-persisting the auto-detected default. `document.documentElement.lang`
  // tracks the committed language on every change.
  React.useEffect(() => {
    let cancelled = false;
    if (!packs[lang]) {
      ensurePack(lang).then((loaded) => {
        if (!cancelled && loaded && activeLangRef.current === lang) {
          setPack(loaded);
        }
      });
    }
    document.documentElement.lang = lang;
    return () => {
      cancelled = true;
    };
  }, [lang]);

  const t = React.useCallback((key, params = {}) => translate(pack, key, params), [pack]);

  const ctx = React.useMemo(() => ({ lang, setLang, t }), [lang, setLang, t]);

  return (<I18nContext.Provider value={ctx}>{children}</I18nContext.Provider>);
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
