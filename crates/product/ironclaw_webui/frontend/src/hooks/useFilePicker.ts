import { useRef, type ChangeEventHandler } from "react";

export interface UseFilePickerOptions {
  accept?: string;
  multiple?: boolean;
  disabled?: boolean;
  onSelect: (files: File[]) => void;
}

export function useFilePicker({
  accept,
  multiple = false,
  disabled = false,
  onSelect,
}: UseFilePickerOptions) {
  const inputRef = useRef<HTMLInputElement>(null);

  const openFilePicker = () => {
    if (!disabled) {
      inputRef.current?.click();
    }
  };

  const handleChange: ChangeEventHandler<HTMLInputElement> = (event) => {
    const input = event.currentTarget;
    const files = Array.from(input.files ?? []);

    // Browsers do not emit another change event for an unchanged file input.
    // Clear it before handing control back so the same file can be selected again.
    input.value = "";

    if (files.length > 0) {
      onSelect(files);
    }
  };

  return [
    openFilePicker,
    {
      ref: inputRef,
      type: "file" as const,
      accept,
      multiple,
      disabled,
      hidden: true,
      onChange: handleChange,
    },
  ] as const;
}
