import { Button } from "@ironclaw/design-system";

export function SuggestionChips({ suggestions, onSelect, disabled = false }) {
  if (!suggestions || suggestions.length === 0) return null;

  return (
    <div className="px-4 pb-3 sm:px-5 lg:px-8">
      <div className="mx-auto flex max-w-5xl flex-wrap gap-2">
        {suggestions.map(
          (text) => (
            <Button
              key={text}
              type="button"
              variant="secondary"
              size="sm"
              disabled={disabled}
              onClick={() => onSelect(text)}
              className="rounded-full text-xs"
            >
              {text}
            </Button>
          )
        )}
      </div>
    </div>
  );
}
