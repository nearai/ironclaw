// IronClaw Web Gateway - Client
//
// This file is the slim orchestrator. All logic has been split into
// focused modules under /modules/*.js which are loaded via <script>
// tags before this file. Functions remain at global scope for
// backward compatibility — no bundler is used.
//
// Module load order (defined in index.html):
//   1. state.js         — global variables, constants, Activity classes
//   2. ui-utils.js      — escapeHtml, showToast, showConfirmModal, etc.
//   3. rendering.js     — renderMarkdown, sanitizeRenderedHtml, structured data
//   4. theme.js         — dark/light/system theme management
//   5. reasoning.js     — reasoning visibility state
//   6. hash-nav.js      — URL hash navigation
//   7. api.js           — apiFetch wrapper
//   8. auth.js          — authenticate, OAuth, token lifecycle
//   9. restart.js       — restart confirmation & progress modal
//  10. tool-activity.js — tool call cards & controller
//  11. approval.js      — approval card rendering
//  12. images.js        — image upload & generated images
//  13. slash.js         — slash command autocomplete
//  14. chat.js          — sendMessage, message creation, history
//  15. gates.js         — auth/gate card handling, extension setup
//  16. threads.js       — thread list, switching, creation
//  17. tabs.js          — tab switching & indicator
//  18. sse.js           — SSE connection & event dispatch
//  19. memory.js        — workspace/memory tab
//  20. logs.js          — logs tab & server log SSE
//  21. extensions.js    — extension management
//  22. pairing.js       — pairing requests
//  23. jobs.js          — jobs tab
//  24. routines.js      — routines tab
//  25. projects.js      — projects/control room tab
//  26. users.js         — user management (admin)
//  27. gateway-status.js — status polling
//  28. tee.js           — TEE attestation
//  29. skills.js        — skills tab
//  30. tools-permissions.js — tool permissions
//  31. keyboard.js      — keyboard shortcuts
//  32. settings.js      — settings tab & subtabs
//  33. config.js        — LLM provider config, IronClaw widget API
//  34. init.js          — event listeners, delegated handlers, layout
