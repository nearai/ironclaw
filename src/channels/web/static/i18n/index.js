// Lightweight internationalization implementation with dynamic language switching

const I18n = {
  currentLang: 'en',
  fallbackLang: 'en',
  translations: {},
  
  // Initialize i18n
  init() {
    // Read user preference from localStorage
    const savedLang = localStorage.getItem('ironclaw_language');
    if (savedLang && this.translations[savedLang]) {
      this.currentLang = savedLang;
    } else {
      // Detect browser language
      const browserLang = navigator.language || navigator.userLanguage;
      this.currentLang = browserLang.startsWith('zh') ? 'zh-CN' : 'en';
    }
    this.updateHtmlLang();
  },
  
  // Register language pack
  register(lang, translations) {
    this.translations[lang] = translations;
  },
  
  // Switch language
  setLanguage(lang) {
    if (this.translations[lang]) {
      this.currentLang = lang;
      localStorage.setItem('ironclaw_language', lang);
      this.updateHtmlLang();
      this.updatePageContent();
      return true;
    }
    return false;
  },
  
  // Get current language
  getCurrentLang() {
    return this.currentLang;
  },
  
  // Translate function
  t(key, params = {}) {
    const translation = this.translations[this.currentLang]?.[key] 
      || this.translations[this.fallbackLang]?.[key] 
      || key;
    
    // Support placeholder replacement: {name}
    return translation.replace(/\{(\w+)\}/g, (match, key) => {
      return params[key] !== undefined ? params[key] : match;
    });
  },
  
  // Update HTML lang attribute
  updateHtmlLang() {
    document.documentElement.lang = this.currentLang;
  },
  
  // Update page content (traverse all data-i18n elements)
  updatePageContent() {
    // Update text content
    document.querySelectorAll('[data-i18n]').forEach(el => {
      const key = el.getAttribute('data-i18n');
      const attr = el.getAttribute('data-i18n-attr');
      if (attr) {
        el.setAttribute(attr, this.t(key));
      } else {
        el.textContent = this.t(key);
      }
    });
    
    // Update placeholder attributes
    document.querySelectorAll('[data-i18n-placeholder]').forEach(el => {
      const key = el.getAttribute('data-i18n-placeholder');
      el.placeholder = this.t(key);
    });
    
    // Update title attributes
    document.querySelectorAll('[data-i18n-title]').forEach(el => {
      const key = el.getAttribute('data-i18n-title');
      el.title = this.t(key);
    });
  }
};

// Global access
window.I18n = I18n;
