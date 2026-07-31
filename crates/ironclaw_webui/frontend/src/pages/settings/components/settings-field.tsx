import React from "react";
import { useT } from "../../../lib/i18n";
import { Card } from "../../../design-system/card";
import { Switch } from "../../../design-system/switch";

function SavedIndicator({ visible }) {
  const t = useT();
  if (!visible) return null;
  return (
    <span
      className="font-mono text-[11px] text-mint"
      role="status"
    >
      {t("tools.saved")}
    </span>
  );
}

export function SettingsField({ field, value, onSave, isSaved }) {
  const t = useT();
  const [localValue, setLocalValue] = React.useState("");
  const label = field.labelKey ? t(field.labelKey) : field.label || "";
  const description = field.descKey ? t(field.descKey) : field.description || "";

  React.useEffect(() => {
    if (field.type !== "boolean") {
      setLocalValue(value !== null && value !== undefined ? String(value) : "");
    }
  }, [value, field.type]);

  const handleCommit = React.useCallback(
    (val) => {
      if (val === "") {
        onSave(field.key, null);
      } else if (field.type === "number") {
        const parsed = parseInt(val, 10);
        if (!isNaN(parsed)) onSave(field.key, parsed);
      } else if (field.type === "float") {
        const parsed = parseFloat(val);
        if (!isNaN(parsed)) onSave(field.key, parsed);
      } else {
        onSave(field.key, val);
      }
    },
    [field.key, field.type, onSave]
  );

  return (
    <div className="flex items-start justify-between gap-6 border-t border-white/[0.06] py-4 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-iron-200">{label}</div>
        {description &&
        (<div className="mt-1 text-xs leading-5 text-iron-300">{description}</div>)}
      </div>

      <div className="flex shrink-0 items-center gap-3">
        {field.type === "boolean"
          ? (
              <Switch
                checked={value === true || value === "true"}
                onChange={(v) => onSave(field.key, v ? "true" : "false")}
                aria-label={label}
                size="sm"
              />
            )
          : field.type === "select"
          ? (
              <select
                value={localValue}
                onChange={(e) => {
                  setLocalValue(e.currentTarget.value);
                  handleCommit(e.currentTarget.value);
                }}
                aria-label={label}
                className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
              >
                <option value="">{t("tools.default")}</option>
                {field.options.map(
                  (opt) => (<option key={opt} value={opt}>{opt}</option>)
                )}
              </select>
            )
          : (
              <input
                type={field.type === "float" || field.type === "number" ? "number" : "text"}
                value={localValue}
                onChange={(e) => setLocalValue(e.currentTarget.value)}
                onBlur={(e) => handleCommit(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && handleCommit(e.currentTarget.value)}
                step={field.step !== undefined ? String(field.step) : field.type === "float" ? "any" : "1"}
                min={field.min !== undefined ? String(field.min) : undefined}
                max={field.max !== undefined ? String(field.max) : undefined}
                placeholder={t("tools.default")}
                aria-label={label}
                className="h-9 w-36 rounded-md border border-white/12 bg-white/[0.04] px-3 text-right font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
            )}
        <SavedIndicator visible={isSaved} />
      </div>
    </div>
  );
}

export function SettingsGroup({ group = "", groupKey, fields, settings, onSave, savedKeys }) {
  const t = useT();
  const groupLabel = groupKey ? t(groupKey) : group || "";
  return (
    <Card className="p-4 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{groupLabel}</h3>
      <div>
        {fields.map(
          (field) =>
            (
              <SettingsField
                key={field.key}
                field={field}
                value={settings[field.key]}
                onSave={onSave}
                isSaved={savedKeys[field.key]}
              />
            )
        )}
      </div>
    </Card>
  );
}
