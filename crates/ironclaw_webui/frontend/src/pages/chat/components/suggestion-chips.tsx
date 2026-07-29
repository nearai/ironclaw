import { SuggestionChip, SuggestionChipRow } from "@ironclaw/design-system";

/* Composer-adjacent placement of the design-system SuggestionChips:
   page padding + max width here, chip styling in the package. */
export function SuggestionChips({ suggestions, onSelect, disabled = false }) {
  if (!suggestions || suggestions.length === 0) return null;

  return (
    <div className="px-4 pb-3 sm:px-5 lg:px-8">
      <SuggestionChipRow className="mx-auto max-w-5xl">
        {suggestions.map((text) => (
          <SuggestionChip
            key={text}
            disabled={disabled}
            onClick={() => {
              if (!disabled) onSelect(text);
            }}
          >
            {text}
          </SuggestionChip>
        ))}
      </SuggestionChipRow>
    </div>
  );
}
