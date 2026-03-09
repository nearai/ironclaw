// IronClaw i18n engine
// Supports: i18n.t('key'), data-i18n attributes, data-i18n-placeholder, data-i18n-title

const i18n = {
  locale: localStorage.getItem('locale') || (function() {
    const lang = navigator.language || '';
    if (lang.startsWith('zh-TW') || lang.startsWith('zh-HK') || lang.startsWith('zh-MO')) return 'zh-TW';
    if (lang.startsWith('zh')) return 'zh';
    return 'en';
  }()),
  messages: {},
  _ready: false,
  _readyCallbacks: [],

  t(key, replacements) {
    const keys = key.split('.');
    let value = this.messages[this.locale];
    for (const k of keys) {
      value = value?.[k];
      if (value === undefined) break;
    }
    if (value === undefined) {
      // Fallback to English
      let fallback = this.messages['en'];
      for (const k of keys) {
        fallback = fallback?.[k];
        if (fallback === undefined) break;
      }
      value = fallback;
    }
    if (value === undefined) return key;
    // Simple placeholder replacement: {0}, {1}, or {name}
    if (replacements) {
      if (Array.isArray(replacements)) {
        replacements.forEach((val, idx) => {
          value = value.replaceAll('{' + idx + '}', val);
        });
      } else if (typeof replacements === 'object') {
        Object.keys(replacements).forEach(k => {
          value = value.replaceAll('{' + k + '}', replacements[k]);
        });
      }
    }
    return value;
  },

  async load(locale) {
    if (this.messages[locale]) {
      this.locale = locale;
      localStorage.setItem('locale', locale);
      document.documentElement.lang = locale;
      this.applyTranslations();
      return;
    }
    try {
      const res = await fetch('/i18n/locales/' + locale + '.json');
      this.messages[locale] = await res.json();
      this.locale = locale;
      localStorage.setItem('locale', locale);
      document.documentElement.lang = locale;
      this.applyTranslations();
    } catch (e) {
      console.error('Failed to load locale:', locale, e);
    }
  },

  async init() {
    // Load English as fallback first, then the selected locale
    try {
      const enRes = await fetch('/i18n/locales/en.json');
      this.messages['en'] = await enRes.json();
    } catch (e) {
      console.warn('Failed to load English fallback:', e);
    }
    if (this.locale !== 'en') {
      try {
        const locRes = await fetch('/i18n/locales/' + this.locale + '.json');
        this.messages[this.locale] = await locRes.json();
      } catch (e) {
        console.warn('Failed to load locale:', this.locale, e);
      }
    }
    document.documentElement.lang = this.locale;
    this.applyTranslations();
    this._ready = true;
    for (const cb of this._readyCallbacks) cb();
    this._readyCallbacks = [];
  },

  onReady(cb) {
    if (this._ready) cb();
    else this._readyCallbacks.push(cb);
  },

  applyTranslations() {
    // data-i18n: set textContent
    document.querySelectorAll('[data-i18n]').forEach(el => {
      const key = el.getAttribute('data-i18n');
      if (key) el.textContent = this.t(key);
    });
    // data-i18n-placeholder: set placeholder
    document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
      const key = el.getAttribute('data-i18n-placeholder');
      if (key) el.placeholder = this.t(key);
    });
    // data-i18n-title: set title
    document.querySelectorAll('[data-i18n-title]').forEach(el => {
      const key = el.getAttribute('data-i18n-title');
      if (key) el.title = this.t(key);
    });
    // data-i18n-html: set innerHTML
    document.querySelectorAll('[data-i18n-html]').forEach(el => {
      const key = el.getAttribute('data-i18n-html');
      if (key) el.innerHTML = this.t(key);
    });
  }
};

// Auto-init when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => i18n.init());
} else {
  i18n.init();
}
