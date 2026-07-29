// @ts-nocheck
import React from "react";
import { Button, Input, Panel, Text } from "@ironclaw/design-system";
import { clientActionId } from "../../../lib/api";
import { useAdminConfiguration } from "../hooks/useAdminConfiguration";

export function AdminConfigurationTab() {
  const state = useAdminConfiguration();
  if (state.query.isLoading) {
    return <div className="v2-skeleton h-48 rounded-xl" aria-label="Loading configuration" />;
  }
  if (state.query.error) {
    return <Text variant="body" tone="danger" role="alert">Unable to load extension configuration.</Text>;
  }
  return (
    <section className="space-y-5" data-testid="admin-configuration-page">
      <header>
        <Text as="p" variant="eyebrow" tone="accent">Admin</Text>
        <Text as="h1" variant="title" tone="strong" className="mt-2">Extension configuration</Text>
        <Text variant="body" tone="muted" className="mt-2 max-w-3xl">
          Configure deployment-owned values declared by extensions. Saving values does not install,
          connect, activate, or remove an extension.
        </Text>
      </header>
      {state.groups.length === 0 ? (
        <Panel className="p-6 text-sm text-[var(--v2-text-muted)]">No extensions require deployment configuration.</Panel>
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
              <Text as="h2" variant="body-lg" tone="strong" weight="medium">{group.display_name}</Text>
              <Text variant="caption" tone={group.complete ? "positive" : "warning"}>
                {group.complete ? "Configured" : "Configuration required"}
              </Text>
            </div>
            {group.description && <Text variant="body" tone="muted" className="mt-1">{group.description}</Text>}
            <Text as="p" variant="caption" tone="faint" className="mt-2">
              Used by {group.used_by.map((extension) => (
                <span key={extension.package_id} className="mr-2 inline-block">
                  {extension.display_name}{extension.installed ? " · installed" : ""}
                </span>
              ))}
            </Text>
          </div>
          <code className="text-[11px] text-[var(--v2-text-faint)]">{group.group_id}</code>
        </div>

        <div className="mt-5 grid gap-4 md:grid-cols-2">
          {group.fields.map((field) => {
            const hint = field.secret && field.provided
              ? "Configured. Leave blank to keep the stored value."
              : null;
            return (
              <div key={field.handle}>
                <Text
                  as="label"
                  variant="caption"
                  tone="muted"
                  htmlFor={`${group.group_id}-${field.handle}`}
                  className="mb-1 block"
                >
                  {field.label}{field.required ? " *" : ""}
                </Text>
                <Input
                  id={`${group.group_id}-${field.handle}`}
                  size="sm"
                  type={field.secret ? "password" : "text"}
                  value={values[field.handle] || ""}
                  disabled={isSaving}
                  autoComplete={field.secret ? "new-password" : "off"}
                  spellCheck={false}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    dirtyHandlesRef.current.add(field.handle);
                    setSaved(false);
                    setValues((current) => ({ ...current, [field.handle]: value }));
                  }}
                />
                {hint && <p className="mt-1 text-[11px] text-[var(--v2-text-faint)]">{hint}</p>}
              </div>
            );
          })}
        </div>
        <div className="mt-5 flex items-center gap-3">
          <Button type="submit" size="sm" loading={isSaving} disabled={state.isSaving}>
            Save configuration
          </Button>
          {saved && <Text as="span" variant="body" tone="positive" role="status">Configuration saved.</Text>}
          {state.saveError && state.savingGroupId === group.group_id && (
            <Text as="span" variant="body" tone="danger" role="alert">Unable to save configuration.</Text>
          )}
        </div>
      </form>
    </Panel>
  );
}
