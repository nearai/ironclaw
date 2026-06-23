import { useNavigate } from "react-router";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { Panel } from "../../../design-system/primitives.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";

// Example prompts shown in the empty state. The text is localized through
// `t`, so this list only holds the i18n keys; non-English packs fall back to
// the en.js copy automatically.
const EXAMPLE_PROMPT_KEYS = [
  "automations.empty.example1",
  "automations.empty.example2",
  "automations.empty.example3",
];

// A single example prompt with a copy-to-clipboard button. The icon flips to a
// checkmark briefly after a successful copy so the click has visible feedback.
function ExamplePrompt({ promptKey }) {
  const t = useT();
  const text = t(promptKey);
  const [copied, setCopied] = React.useState(false);
  const timerRef = React.useRef(null);

  React.useEffect(() => () => clearTimeout(timerRef.current), []);

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setCopied(false), 1500);
    } catch (_) {
      // Clipboard can be blocked (insecure context / denied permission);
      // leave the prompt visible to copy manually rather than crash.
    }
  };

  return html`
    <li
      className="flex items-center gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
    >
      <span className="min-w-0 flex-1 text-sm leading-6 text-iron-200">${text}</span>
      <button
        type="button"
        onClick=${onCopy}
        aria-label=${copied ? t("automations.empty.copied") : t("automations.empty.copyPrompt")}
        title=${copied ? t("automations.empty.copied") : t("automations.empty.copyPrompt")}
        className=${cn(
          "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-[var(--v2-panel-border)] text-iron-300 hover:text-iron-100 hover:border-white/20",
          "focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--v2-accent)]",
          copied && "text-emerald-300"
        )}
      >
        <${Icon} name=${copied ? "check" : "copy"} className="h-4 w-4" />
      </button>
    </li>
  `;
}

// Onboarding empty state shown when the agent has no scheduled automations at
// all. Automations are created by chatting with the agent (there is no "New
// automation" form), so the empty state must say so and offer a shortcut to
// chat plus a few example prompts to copy.
export function AutomationsEmptyState() {
  const t = useT();
  const navigate = useNavigate();

  return html`
    <${Panel} className="p-6 sm:p-8">
      <div className="max-w-2xl">
        <h2 className="mt-4 text-2xl font-semibold tracking-tight text-iron-100 flex items-center gap-3">
          ${t("automations.empty.onboardingTitle")}
        </h2>
        <p className="mt-3 text-sm leading-6 text-iron-300">
          ${t("automations.empty.onboardingDescription")}
        </p>

        <div className="mt-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-400">
            ${t("automations.empty.examplesTitle")}
          </div>
          <ul className="mt-3 space-y-2">
            ${EXAMPLE_PROMPT_KEYS.map(
              (key) => html`<${ExamplePrompt} key=${key} promptKey=${key} />`
            )}
          </ul>
        </div>

        <div className="mt-6">
          <${Button} variant="primary" size="sm" onClick=${() => navigate("/chat")}>
            <${Icon} name="chat" className="mr-1.5 h-4 w-4" />
            ${t("automations.empty.startInChat")}
          <//>
        </div>
      </div>
    <//>
  `;
}
