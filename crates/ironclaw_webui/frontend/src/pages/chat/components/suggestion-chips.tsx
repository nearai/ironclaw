
export function SuggestionChips({ suggestions, onSelect, disabled = false }) {
  if (!suggestions || suggestions.length === 0) return null;

  return (
    <div className="px-4 pb-3 sm:px-5 lg:px-8">
      <div className="mx-auto flex max-w-5xl flex-wrap gap-2">
        {suggestions.map(
          (text) => (
            <button
              key={text}
              onClick={() => {
                if (!disabled) onSelect(text);
              }}
              disabled={disabled}
              className="v2-button rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-1.5 text-xs text-[var(--v2-text-strong)] hover:border-[var(--v2-accent)]/40 hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              {text}
            </button>
          )
        )}
      </div>
    </div>
  );
}
