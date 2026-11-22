// The sign-in page: one card on the paper background, nothing else. Shown whenever the server
// asks for a login and this browser has no session. With an OpenID provider (config.provider)
// the card sends the browser there instead of asking for a password; the provider sends it
// back through /auth/oidc/callback, which lands here signed in, or with ?signin_error=<reason>.

import { useState, type FormEvent } from "react";
import { Loader2 } from "lucide-react";
import { Logo } from "@/components/app-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { config } from "@/lib/api";
import { ROOT } from "@/lib/config";
import { signIn, throughProvider } from "@/lib/session";

/** What the server's ?signin_error= reasons mean to the person signing in. */
const REASONS: Record<string, string> = {
  expired: "That sign-in took too long or was started in another tab. Try again.",
  refused: "Your organisation's sign-in did not let you in.",
  provider: "Your organisation's sign-in could not be reached. Try again, or tell your administrator.",
  token: "The answer of your organisation's sign-in could not be trusted. Tell your administrator.",
  account: "Your account on this server is turned off or cannot be made. Ask an administrator.",
  internal: "Something went wrong on this server. Try again.",
};

/** Why a sign-in through the provider failed, read once and taken out of the address. */
const failed = (() => {
  const params = new URLSearchParams(location.search);
  const reason = params.get("signin_error");
  if (reason === null) return null;
  params.delete("signin_error");
  const search = params.size ? `?${params}` : "";
  history.replaceState(history.state, "", `${location.pathname}${search}${location.hash}`);
  return REASONS[reason] ?? REASONS.internal;
})();

export function SignInPage() {
  return (
    <div className="grid min-h-dvh place-items-center bg-background p-6">
      <div className="w-full max-w-sm">
        {throughProvider ? <ProviderCard /> : <PasswordCard />}
        <p className="mt-4 text-center text-xs text-muted-foreground">pacs-rs {config.version}</p>
      </div>
    </div>
  );
}

/** To the provider, with the page to come back to. */
function ProviderCard() {
  const start = new URL(`auth/oidc/start?next=${encodeURIComponent(location.hash)}`, ROOT);
  return (
    <div className="rounded-xl border bg-card p-7">
      <Logo className="size-7" />
      <h1 className="mt-5 text-lg font-semibold tracking-tight">Sign in to pacs-rs</h1>
      <p className="mt-2 text-sm text-muted-foreground">You sign in with your organisation's account.</p>
      <p role="alert" className="mt-4 min-h-5 text-sm text-destructive">
        {failed}
      </p>
      <Button asChild className="mt-2 w-full">
        <a href={start.href}>Continue to sign in</a>
      </Button>
    </div>
  );
}

function PasswordCard() {
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!login.trim() || !password) {
      setError("Enter your login and password.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await signIn(login.trim(), password);
    } catch (e) {
      setError(e instanceof Error ? e.message : "The server did not answer. Try again.");
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="rounded-xl border bg-card p-7" noValidate>
      <Logo className="size-7" />
      <h1 className="mt-5 text-lg font-semibold tracking-tight">Sign in to pacs-rs</h1>
      <div className="mt-6 grid gap-4">
        <label className="grid gap-1.5 text-sm font-medium">
          Login
          <Input
            name="username"
            autoComplete="username"
            autoCapitalize="none"
            spellCheck={false}
            autoFocus
            value={login}
            onChange={(e) => (setLogin(e.target.value), setError(null))}
            aria-invalid={!!error}
          />
        </label>
        <label className="grid gap-1.5 text-sm font-medium">
          Password
          <Input
            name="password"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => (setPassword(e.target.value), setError(null))}
            aria-invalid={!!error}
          />
        </label>
      </div>
      <p role="alert" className="mt-3 min-h-5 text-sm text-destructive">
        {error}
      </p>
      <Button type="submit" className="mt-2 w-full" disabled={busy}>
        {busy && <Loader2 className="animate-spin" />}
        Sign in
      </Button>
    </form>
  );
}
