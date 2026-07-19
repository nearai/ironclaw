import { useForm } from "react-hook-form";
import { Button } from "../../design-system/button";
import { Card } from "../../design-system/card";
import { Input, FormField } from "../../design-system/input";
import { Icon } from "../../design-system/icons";
import { useInterfaceTheme } from "../../design-system/theme";
import { useT } from "../../lib/i18n";
import { cn } from "../../utils/cn";
import { OAuthProviderButtons } from "./components/oauth-provider-buttons";
import { useOAuthProviders } from "./hooks/useOAuthProviders";

export function LoginPage({ initialToken, error, oauthRedirectAfter = "/", onSubmit }) {
  const t = useT();
  const { theme, toggleTheme } = useInterfaceTheme();
  const oauthProviders = useOAuthProviders();
  const {
    formState: { errors, isSubmitting },
    handleSubmit,
    register,
  } = useForm({
    defaultValues: { token: initialToken || "" },
  });

  return (
    <main
      className="relative flex min-h-[100dvh] items-center justify-center bg-[var(--v2-canvas)] px-4 py-8 sm:px-6 lg:px-12"
    >
      {/* Theme toggle */}
      <Button
        variant="secondary"
        size="icon"
        onClick={toggleTheme}
        aria-label={theme === "dark" ? t("theme.switchToLight") : t("theme.switchToDark")}
        title={theme === "dark" ? t("theme.light") : t("theme.dark")}
        className="absolute right-4 top-4 z-10 sm:right-6 sm:top-6"
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} className="h-4 w-4" />
      </Button>

      {/* Login form (centered) */}
      <Card
        as="section"
        radius="lg"
        padding="md"
        className="w-full max-w-md p-6 shadow-none sm:p-8"
      >
        <div className="mb-8">
          <p className="mb-3 font-mono text-xs uppercase tracking-[0.2em] text-[var(--v2-accent-text)]">
            {t("login.tagline")}
          </p>
          <h1
            className="text-5xl font-semibold leading-none tracking-[-0.04em] text-[var(--v2-text-strong)]"
          >
            {t("login.console")}
          </h1>
          <p className="mt-4 text-sm leading-6 text-[var(--v2-text-muted)]">
            {t("login.secureSub")}
          </p>
        </div>

        <form
          className="space-y-4"
          onSubmit={handleSubmit(({ token }) => onSubmit(token))}
        >
          <FormField
            label={t("login.tokenLabel")}
            htmlFor="v2-token"
            error={String(errors.token?.message ?? "")}
            hint={t("login.tokenHint")}
          >
            <Input
              id="v2-token"
              type="password"
              error={!!errors.token}
              {...register("token", {
                required: t("login.tokenRequired"),
                setValueAs: (v) => v.trim(),
              })}
              placeholder={t("login.tokenPlaceholder")}
              autoComplete="current-password"
            />
          </FormField>

          {error &&
            (<p
              className={cn(
                "rounded-[10px] border px-3 py-2 text-sm",
                "border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]",
                "bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]"
              )}
            >{error}</p>)}

          <Button
            type="submit"
            variant="primary"
            fullWidth
            disabled={isSubmitting}
          >
            {t("login.connect")}
          </Button>
        </form>

        <OAuthProviderButtons
          providers={oauthProviders}
          redirectAfter={oauthRedirectAfter}
        />
      </Card>
    </main>
  );
}
