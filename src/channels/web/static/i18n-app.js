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
    
    // Close menu
    const menu = document.getElementById('language-menu');
    if (menu) {
      menu.style.display = 'none';
    }
    
    // Show toast notification
    showToast(I18n.t('language.switch') + ': ' + (lang === 'zh-CN' ? '简体中文' : 'English'));
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

// Toast notification helper
function showToast(message) {
  const toasts = document.getElementById('toasts');
  if (!toasts) return;
  
  const toast = document.createElement('div');
  toast.className = 'toast';
  toast.textContent = message;
  toasts.appendChild(toast);
  
  setTimeout(() => {
    toast.remove();
  }, 3000);
}

// Override authenticate function to use i18n
const originalAuthenticate = authenticate;
authenticate = function() {
  token = document.getElementById('token-input').value.trim();
  if (!token) {
    document.getElementById('auth-error').textContent = I18n.t('auth.errorRequired');
    return;
  }

  apiFetch('/api/chat/threads')
    .then(() => {
      sessionStorage.setItem('ironclaw_token', token);
      document.getElementById('auth-screen').style.display = 'none';
      document.getElementById('app').style.display = 'flex';
      const cleaned = new URL(window.location);
      const urlLogLevel = cleaned.searchParams.get('log_level');
      cleaned.searchParams.delete('token');
      cleaned.searchParams.delete('log_level');
      window.history.replaceState({}, '', cleaned.pathname + cleaned.search);
      connectSSE();
      connectLogSSE();
      startGatewayStatusPolling();
      checkTeeStatus();
      loadThreads();
      loadMemoryTree();
      loadJobs();
      if (urlLogLevel) {
        setServerLogLevel(urlLogLevel);
      } else {
        loadServerLogLevel();
      }
    })
    .catch(() => {
      sessionStorage.removeItem('ironclaw_token');
      document.getElementById('auth-screen').style.display = '';
      document.getElementById('app').style.display = 'none';
      document.getElementById('auth-error').textContent = I18n.t('auth.errorInvalid');
    });
};

// Override triggerRestart to use i18n
const originalTriggerRestart = triggerRestart;
triggerRestart = function() {
  if (!currentThreadId) {
    alert(I18n.t('error.startConversation'));
    return;
  }
  const confirmModal = document.getElementById('restart-confirm-modal');
  confirmModal.style.display = 'flex';
};

// Override confirmRestart to use i18n
const originalConfirmRestart = confirmRestart;
confirmRestart = function() {
  if (!currentThreadId) {
    alert(I18n.t('error.startConversation'));
    return;
  }

  const confirmModal = document.getElementById('restart-confirm-modal');
  confirmModal.style.display = 'none';

  const restartBtn = document.getElementById('restart-btn');
  const restartIcon = document.getElementById('restart-icon');

  isRestarting = true;
  restartBtn.disabled = true;
  if (restartIcon) restartIcon.classList.add('spinning');

  const loaderEl = document.getElementById('restart-loader');
  loaderEl.style.display = 'flex';

  apiFetch('/api/chat/send', {
    method: 'POST',
    body: {
      content: '/restart',
      thread_id: currentThreadId,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    },
  })
    .then((response) => {
      console.log('[confirmRestart] API call succeeded, response:', response);
    })
    .catch((err) => {
      console.error('[confirmRestart] Restart request failed:', err);
      addMessage('system', I18n.t('error.restartFailed', { message: err.message }));
      isRestarting = false;
      restartBtn.disabled = false;
      if (restartIcon) restartIcon.classList.remove('spinning');
      loaderEl.style.display = 'none';
    });
};
