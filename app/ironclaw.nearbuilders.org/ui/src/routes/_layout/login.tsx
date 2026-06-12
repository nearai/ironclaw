import { useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Navigate, redirect, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { toast } from "sonner";
import { sessionQueryOptions, useAuthClient } from "@/app";
import { UnderConstruction } from "@/components/under-construction";

type SearchParams = {
  redirect?: string;
};

export const Route = createFileRoute("/_layout/login")({
  ssr: false,
  validateSearch: (search: Record<string, unknown>): SearchParams => ({
    redirect: typeof search.redirect === "string" ? search.redirect : undefined,
  }),
  beforeLoad: ({ context, search }) => {
    const { queryClient, authClient } = context;
    const initialSession = context.session;
    const session =
      initialSession ??
      queryClient.getQueryData(sessionQueryOptions(authClient, initialSession).queryKey);

    if (session?.user) {
      const redirectTo = search.redirect?.startsWith("/") ? search.redirect : "/home";
      throw redirect({ to: redirectTo, search: {} });
    }
  },
  loader: ({ context }) => {
    const initialSession = context.session;
    void context.queryClient.prefetchQuery(sessionQueryOptions(context.authClient, initialSession));
  },
  component: LoginPage,
});

function LoginPage() {
  const navigate = useNavigate();
  const auth = useAuthClient();
  const { data: session } = useQuery(sessionQueryOptions(auth, undefined));
  const { redirect } = Route.useSearch();
  const { runtimeConfig } = Route.useRouteContext();
  const queryClient = useQueryClient();

  const [nearPending, setNearPending] = useState(false);
  const [anonPending, setAnonPending] = useState(false);

  const handleSuccess = async (message: string) => {
    const redirectTo = redirect?.startsWith("/") ? redirect : "/home";
    toast.success(message);
    const { data: freshSession } = await auth.getSession();
    if (freshSession) {
      queryClient.setQueryData(["session"], freshSession);
    }
    await queryClient.invalidateQueries({ queryKey: ["session"] });
    navigate({ to: redirectTo, replace: true, search: {} });
  };

  const handleError = (error: { code?: string; message?: string } | Error) => {
    const code = "code" in error ? error.code : undefined;
    const message = "message" in error ? error.message : "Failed to sign in";
    if (code === "UNAUTHORIZED_NONCE_REPLAY") toast.error("Sign-in already used");
    else if (code === "UNAUTHORIZED_INVALID_SIGNATURE") toast.error("Invalid signature");
    else if (code === "SIGNER_NOT_AVAILABLE") toast.error("NEAR wallet not available");
    else toast.error(message || "Failed to sign in");
  };

  const handleNear = async () => {
    setNearPending(true);
    await auth.signIn.near({
      onSuccess: async () => {
        setNearPending(false);
        await handleSuccess("Signed in with NEAR");
      },
      onError: (error: { code?: string; message?: string }) => {
        setNearPending(false);
        handleError(error);
      },
    });
  };

  const handleAnonymous = async () => {
    setAnonPending(true);
    try {
      await auth.signIn.anonymous({
        fetchOptions: {
          onSuccess: async () => {
            setAnonPending(false);
            await handleSuccess("Started anonymous session");
          },
          onError: (ctx: { error?: { message?: string } }) => {
            setAnonPending(false);
            handleError(new Error(ctx.error?.message || "Anonymous sign in failed"));
          },
        },
      });
    } catch {
      setAnonPending(false);
    }
  };

  if (session?.user) {
    const redirectTo = redirect?.startsWith("/") ? redirect : "/home";
    return <Navigate to={redirectTo} replace search={{}} />;
  }

  const isPending = nearPending || anonPending;

  return (
    <div className="min-h-[70vh] w-full flex items-start justify-center px-6 pt-[30vh] animate-fade-in">
      <div className="w-full max-w-sm space-y-8">
        <div className="flex justify-center">
          <UnderConstruction
            sourceFile="ui/src/routes/_layout/login.tsx"
            runtimeConfig={runtimeConfig}
          />
        </div>

        <div className="space-y-3 animate-fade-in-up">
          <button
            type="button"
            onClick={handleNear}
            disabled={isPending}
            className="w-full h-10 px-4 inline-flex items-center justify-center gap-2 text-sm font-medium border-2 border-outset border-border-strong bg-card text-foreground shadow-sm hover:shadow-md hover:bg-muted active:border-inset active:shadow-none transition-all duration-200 ease-out disabled:pointer-events-none disabled:opacity-50 cursor-pointer"
          >
            {nearPending ? "connecting..." : "connect to everything"}
          </button>

          <div className="flex items-center gap-3">
            <div className="flex-1 h-px bg-border" />
            <span className="text-xs text-muted-foreground">or</span>
            <div className="flex-1 h-px bg-border" />
          </div>

          <button
            type="button"
            onClick={handleAnonymous}
            disabled={isPending}
            className="w-full h-10 px-4 inline-flex items-center justify-center gap-2 text-sm font-medium border-2 border-transparent bg-transparent text-muted-foreground hover:bg-muted hover:text-foreground hover:shadow-sm active:shadow-none transition-all duration-200 ease-out disabled:pointer-events-none disabled:opacity-50 cursor-pointer"
          >
            {anonPending ? "starting..." : "continue anonymously"}
          </button>
        </div>

        <div className="pt-2 border-t border-border">
          <p className="text-xs text-muted-foreground text-center leading-relaxed">
            Anonymous sessions don't persist after sign out
          </p>
        </div>
      </div>
    </div>
  );
}
