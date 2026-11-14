// The sign-in page: one card on the paper background, nothing else. Shown whenever the server
// asks for a login and this browser has no session.

import { useState, type FormEvent } from "react";
import { Loader2 } from "lucide-react";
import { Logo } from "@/components/app-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { config } from "@/lib/api";
import { signIn } from "@/lib/session";

export function SignInPage() {
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
    <div className="grid min-h-dvh place-items-center bg-background p-6">
      <div className="w-full max-w-sm">
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
        <p className="mt-4 text-center text-xs text-muted-foreground">pacs-rs {config.version}</p>
      </div>
    </div>
  );
}
