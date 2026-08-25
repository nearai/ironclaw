// @ts-nocheck
import React from "react";
import { Button } from "../../../design-system/button";
import { Input } from "../../../design-system/input";
import { InlineNotice } from "../../../design-system/inline-notice";
import { Panel } from "../../../design-system/primitives";
import { Skeleton } from "../../../design-system/skeleton";
import { clientActionId } from "../../../lib/api";
import { useT } from "../../../lib/i18n";
import { useAdminConfiguration } from "../hooks/useAdminConfiguration";

export function AdminConfigurationTab() {
  const t = useT();
  const state = useAdminConfiguration();
  if (state.query.isLoading) {
    return <Skeleton className="h-48 rounded-xl" aria-label={t("admin.configuration.loading")} />;
  }
  if (state.query.error) {
    return (
      <InlineNotice tone="danger" role="alert">
        {t("admin.configuration.loadFailed")}
      </InlineNotice>
    );
  }
  return (
    <section className="space-y-5" data-testid="admin-configuration-page">
      <header>
        <p className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("nav.admin")}</p>
        <h1 className="mt-2 text-xl font-semibold text-iron-50">{t("admin.configuration.title")}</h1>
        <p className="mt-2 max-w-3xl text-sm text-iron-300">
          {t("admin.configuration.description")}
        </p>
      </header>
      {state.groups.length === 0 ? (
        <Panel className="p-6 text-sm text-iron-300">{t("admin.configuration.empty")}</Panel>
      ) : state.groups.map((group) => (
        <ConfigurationGroup key={group.group_id} group={group} state={state} />
      ))}
    </section>
  );
}

export function buildConfigurationSaveMutation(group, values, idempotencyKey) {
  return {
    groupId: group.group_id,
    expectedRevision: group.revision,
    idempotencyKey,
    values: group.fields.map((field) => ({
      handle: field.handle,
      value: values[field.handle] || "",
    })),
  };
}

export function configurationSaveErrorMessageKey(error) {
  return error?.status === 409
    ? "admin.configuration.saveConflict"
    : "admin.configuration.saveFailed";
}

function configurationValuesFromFields(fields) {
  return Object.fromEntries(fields.map((field) => [
    field.handle,
    field.secret ? "" : field.value || "",
  ]));
}

function mergeRefetchedConfigurationValues(fields, current, dirtyHandles) {
  return Object.fromEntries(fields.map((field) => [
    field.handle,
    dirtyHandles.has(field.handle)
      ? current[field.handle] || ""
      : field.secret ? "" : field.value || "",
  ]));
}

export function ConfigurationGroup({ group, state }) {
  const t = useT();
  const initialValues = React.useMemo(
    () => configurationValuesFromFields(group.fields),
    [group.fields],
  );
  const [values, setValues] = React.useState(initialValues);
  const [saved, setSaved] = React.useState(false);
  const dirtyHandlesRef = React.useRef(new Set());
  React.useEffect(() => {
    setValues((current) => mergeRefetchedConfigurationValues(
      group.fields,
      current,
      dirtyHandlesRef.current,
    ));
  }, [group.fields]);
  const isSaving = state.isSaving && state.savingGroupId === group.group_id;

  const submit = async (event) => {
    event.preventDefault();
    setSaved(false);
    state.resetSave?.();
    const mutation = buildConfigurationSaveMutation(
      group,
      values,
      clientActionId(),
    );
    try {
      const savedGroup = await state.save(mutation);
      dirtyHandlesRef.current.clear();
      setSaved(true);
      setValues(configurationValuesFromFields(savedGroup?.fields || group.fields));
    } catch (_) {
      // The mutation exposes a sanitized error below.
    }
  };

  return (
    <Panel className="p-5 sm:p-6" data-testid="admin-configuration-group">
      <form onSubmit={submit}>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-base font-semibold text-iron-50">{group.display_name}</h2>
              <span className={group.complete ? "text-xs text-signal" : "text-xs text-amber-200"}>
                {group.complete
                  ? t("admin.configuration.statusConfigured")
                  : t("admin.configuration.statusRequired")}
              </span>
            </div>
            {group.description && <p className="mt-1 text-sm text-iron-300">{group.description}</p>}
            <p className="mt-2 text-xs text-iron-400">
              {t("admin.configuration.usedBy")} {group.used_by.map((extension) => (
                <span key={extension.package_id} className="mr-2 inline-block">
                  {extension.display_name}{extension.installed
                    ? ` · ${t("admin.configuration.installed")}`
                    : ""}
                </span>
              ))}
            </p>
          </div>
          <code className="text-[11px] text-iron-400">{group.group_id}</code>
        </div>

        <div className="mt-5 grid gap-4 md:grid-cols-2">
          {group.fields.map((field) => {
            const hint = field.secret && field.provided
              ? t("admin.configuration.secretHint")
              : null;
            const descriptionId = field.description
              ? `${group.group_id}-${field.handle}-description`
              : null;
            return (
              <div key={field.handle}>
                <label htmlFor={`${group.group_id}-${field.handle}`} className="mb-1 block text-xs text-iron-300">
                  {field.label}{field.required ? " *" : ""}
                </label>
                <Input
                  id={`${group.group_id}-${field.handle}`}
                  size="sm"
                  type={field.secret ? "password" : "text"}
                  value={values[field.handle] || ""}
                  disabled={isSaving}
                  autoComplete={field.secret ? "new-password" : "off"}
                  spellCheck={false}
                  aria-describedby={descriptionId || undefined}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    dirtyHandlesRef.current.add(field.handle);
                    setSaved(false);
                    setValues((current) => ({ ...current, [field.handle]: value }));
                  }}
                />
                {field.description && (
                  <p id={descriptionId} className="mt-1 text-[11px] text-iron-400">{field.description}</p>
                )}
                {hint && <p className="mt-1 text-[11px] text-iron-400">{hint}</p>}
              </div>
            );
          })}
        </div>
        <div className="mt-5 space-y-3">
          <Button type="submit" size="sm" loading={isSaving} disabled={state.isSaving}>
            {t("admin.configuration.save")}
          </Button>
          {saved && (
            <InlineNotice tone="success" role="status">
              {t("admin.configuration.saved")}
            </InlineNotice>
          )}
          {state.saveError && state.savingGroupId === group.group_id && (
            <InlineNotice tone="danger" role="alert">
              {t(configurationSaveErrorMessageKey(state.saveError))}
            </InlineNotice>
          )}
        </div>
      </form>
    </Panel>
  );
}
