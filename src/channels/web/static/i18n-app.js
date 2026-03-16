// IronClaw i18n — app integration (language switcher + slash command translation)

document.addEventListener('DOMContentLoaded', () => {
  I18n.init();
  I18n.updatePageContent();
  updateSlashCommands();
  injectLanguageSwitcher();
});

// Map slash command names to i18n keys and update descriptions
function updateSlashCommands() {
  if (typeof SLASH_COMMANDS === 'undefined') return;
  const cmdMap = {
    '/status': 'cmd.status.desc',
    '/list': 'cmd.list.desc',
    '/cancel': 'cmd.cancel.desc',
    '/undo': 'cmd.undo.desc',
    '/redo': 'cmd.redo.desc',
    '/compact': 'cmd.compact.desc',
    '/clear': 'cmd.clear.desc',
    '/interrupt': 'cmd.interrupt.desc',
    '/heartbeat': 'cmd.heartbeat.desc',
    '/summarize': 'cmd.summarize.desc',
    '/suggest': 'cmd.suggest.desc',
    '/help': 'cmd.help.desc',
    '/version': 'cmd.version.desc',
    '/tools': 'cmd.tools.desc',
    '/skills': 'cmd.skills.desc',
    '/model': 'cmd.model.desc',
    '/thread new': 'cmd.threadNew.desc',
  };
  SLASH_COMMANDS.forEach(entry => {
    const key = cmdMap[entry.cmd];
    if (key) entry.desc = I18n.t(key);
  });
}

// Inject language switcher into the status bar area
function injectLanguageSwitcher() {
  const statusBar = document.querySelector('.status-bar');
  if (!statusBar) return;

  const wrapper = document.createElement('div');
  wrapper.className = 'language-switcher';
  wrapper.innerHTML = `
    <button class="language-btn" onclick="toggleLanguageMenu()" title="Language">
      <span class="language-icon">🌐</span>
    </button>
    <div class="language-menu" id="language-menu" style="display:none;">
      <button class="language-option" data-lang="en" onclick="switchLanguage('en')">English</button>
      <button class="language-option" data-lang="zh-CN" onclick="switchLanguage('zh-CN')">简体中文</button>
      <button class="language-option" data-lang="zh-TW" onclick="switchLanguage('zh-TW')">繁體中文</button>
    </div>
  `;
  statusBar.appendChild(wrapper);
  updateLanguageMenu();
}

function toggleLanguageMenu() {
  const menu = document.getElementById('language-menu');
  if (!menu) return;
  menu.style.display = menu.style.display === 'none' ? 'flex' : 'none';
}

function switchLanguage(lang) {
  I18n.setLanguage(lang);
  updateSlashCommands();
  updateLanguageMenu();
  const menu = document.getElementById('language-menu');
  if (menu) menu.style.display = 'none';
}

function updateLanguageMenu() {
  const current = I18n.getCurrentLang();
  document.querySelectorAll('.language-option').forEach(btn => {
    btn.classList.toggle('active', btn.getAttribute('data-lang') === current);
  });
}

// Close language menu on outside click
document.addEventListener('click', (e) => {
  if (!e.target.closest('.language-switcher')) {
    const menu = document.getElementById('language-menu');
    if (menu) menu.style.display = 'none';
  }
});
