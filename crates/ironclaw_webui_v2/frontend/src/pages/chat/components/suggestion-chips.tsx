
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
              className="v2-button rounded-full border border-white/10 bg-white/[0.035] px-3 py-1.5 text-xs text-iron-100 hover:border-signal/40 hover:text-signal disabled:cursor-not-allowed disabled:opacity-50"
            >
              {text}
            </button>
          )
        )}
      </div>
    </div>
  );
}
