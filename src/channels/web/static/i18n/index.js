// IronClaw i18n — lightweight translation framework

const I18n = (() => {
  const langs = {};      // { 'en': {...}, 'zh-CN': {...}, 'zh-TW': {...} }
  let currentLang = 'en';

  return {
    register(lang, translations) {
      langs[lang] = translations;
    },

    init() {
      // 1. Check localStorage first
      const saved = localStorage.getItem('ironclaw_language');
      if (saved && langs[saved]) {
        currentLang = saved;
        return;
      }
      // 2. Browser language detection
      const browserLang = navigator.language || navigator.userLanguage || 'en';
      if (browserLang === 'zh-TW' || browserLang === 'zh-Hant' || browserLang.startsWith('zh-Hant')) {
        currentLang = langs['zh-TW'] ? 'zh-TW' : 'en';
      } else if (browserLang.startsWith('zh')) {
        currentLang = langs['zh-CN'] ? 'zh-CN' : 'en';
      } else {
        currentLang = 'en';
      }
    },

    setLanguage(lang) {
      if (langs[lang]) {
        currentLang = lang;
        localStorage.setItem('ironclaw_language', lang);
        this.updatePageContent();
      }
    },

    getCurrentLang() {
      return currentLang;
    },

    getAvailableLanguages() {
      return Object.keys(langs);
    },

    t(key, params) {
      const val = (langs[currentLang] && langs[currentLang][key])
                || (langs['en'] && langs['en'][key])
                || key;
      if (!params) return val;
      return val.replace(/\{(\w+)\}/g, (_, k) => (params[k] !== undefined ? params[k] : `{${k}}`));
    },

    updatePageContent() {
      // Update text content
      document.querySelectorAll('[data-i18n]').forEach(el => {
        const key = el.getAttribute('data-i18n');
        el.textContent = this.t(key);
      });
      // Update placeholders
      document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
        el.placeholder = this.t(el.getAttribute('data-i18n-placeholder'));
      });
      // Update titles
      document.querySelectorAll('[data-i18n-title]').forEach(el => {
        el.title = this.t(el.getAttribute('data-i18n-title'));
      });
    }
  };
})();
