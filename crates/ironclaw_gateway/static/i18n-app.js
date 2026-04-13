// i18n Integration for IronClaw App
// This file contains i18n-related functions that extend app.js

// Initialize i18n when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
  // Initialize i18n
  I18n.init();
  I18n.updatePageContent();
  updateSlashCommands();
  updateLanguageMenu();
});

// Update slash commands with current language
function updateSlashCommands() {
  // Update SLASH_COMMANDS descriptions
  SLASH_COMMANDS.forEach(cmd => {
    const key = 'cmd.' + cmd.cmd.replace(/\s+/g, '').replace(/\//g, '') + '.desc';
    const translated = I18n.t(key);
    if (translated !== key) {
      cmd.desc = translated;
    }
  });
}

// Toggle language menu
function toggleLanguageMenu() {
  const menu = document.getElementById('language-menu');
  if (menu) {
    menu.style.display = menu.style.display === 'none' ? 'block' : 'none';
  }
}

// Switch language
function switchLanguage(lang) {
  if (I18n.setLanguage(lang)) {
    // Update slash commands
    updateSlashCommands();

    // Update language menu active state
    updateLanguageMenu();

    // Re-render dynamically built sections that use I18n.t()
    if (typeof renderProviders === 'function' && typeof _configLoaded !== 'undefined' && _configLoaded) {
      renderProviders();
    }
    if (typeof loadInferenceSettings === 'function') {
      var inferencePanel = document.getElementById('settings-inference');
      if (inferencePanel && inferencePanel.classList.contains('active')) {
        loadInferenceSettings();
      }
    }

    // Close menu
    const menu = document.getElementById('language-menu');
    if (menu) {
      menu.style.display = 'none';
    }

    // Show toast notification. Both halves of the message read from the
    // current language pack — which has just been switched to `lang` — so
    // the toast appears entirely in the language the user just selected.
    // Each language file owns its own native name under `language.name`,
    // so adding a new language here is a single key addition with no
    // changes to this file.
    showToast(I18n.t('language.switch') + ': ' + I18n.t('language.name'));
  }
}

// Update language menu active state
function updateLanguageMenu() {
  const currentLang = I18n.getCurrentLang();
  document.querySelectorAll('.language-option').forEach(option => {
    if (option.getAttribute('data-lang') === currentLang) {
      option.classList.add('active');
    } else {
      option.classList.remove('active');
    }
  });
}

// Close language menu when clicking outside
document.addEventListener('click', (e) => {
  if (!e.target.closest('.language-switcher')) {
    const menu = document.getElementById('language-menu');
    if (menu) {
      menu.style.display = 'none';
    }
  }
});

