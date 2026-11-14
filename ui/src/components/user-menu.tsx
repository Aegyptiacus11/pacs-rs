// At the right of the top bar: who is signed in and with which role, their password, the
// users for an administrator, and the way out. Nothing when the server asks for no login.

import { useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { KeyRound, Loader2, LogOut, Users } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { changePassword, may, signOut, useSession } from "@/lib/session";

export const MIN_PASSWORD = 12;

function PasswordDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const close = () => {
    setCurrent("");
    setNext("");
    setAgain("");
    setError(null);
    onClose();
  };

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (new TextEncoder().encode(next).length < MIN_PASSWORD) return setError(`A password is at least ${MIN_PASSWORD} characters.`);
    if (next !== again) return setError("The two new passwords differ.");
    setBusy(true);
    setError(null);
    try {
      await changePassword(current, next);
      toast.success("Password changed. Sign in with the new one.");
    } catch (e) {
      setError(e instanceof Error ? e.message : "The password was not changed.");
      setBusy(false);
    }
  }

  const field = (label: string, value: string, set: (v: string) => void, autoComplete: string) => (
    <label className="grid gap-1.5 text-sm font-medium">
      {label}
      <Input type="password" autoComplete={autoComplete} value={value} onChange={(e) => (set(e.target.value), setError(null))} />
    </label>
  );
  return (
    <Dialog open={open} onOpenChange={(o) => !o && !busy && close()}>
      <DialogContent className="sm:max-w-sm">
        <form onSubmit={submit} className="grid gap-4" noValidate>
          <DialogHeader>
            <DialogTitle>Change your password</DialogTitle>
            <DialogDescription>Every session of yours ends, this one included: you sign in again with the new password.</DialogDescription>
          </DialogHeader>
          {field("Current password", current, setCurrent, "current-password")}
          {field("New password", next, setNext, "new-password")}
          {field("New password, again", again, setAgain, "new-password")}
          <p role="alert" className="min-h-5 text-sm text-destructive">
            {error}
          </p>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy && <Loader2 className="animate-spin" />}
              Change password
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function UserMenu() {
  const { user } = useSession();
  const [password, setPassword] = useState(false);
  const queries = useQueryClient();
  if (!user) return null;
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="outline"
            size="icon"
            aria-label={`Account: ${user.login}`}
            className="rounded-full text-xs font-semibold uppercase"
          >
            {user.login.slice(0, 2)}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuLabel className="flex items-center justify-between gap-3 font-normal">
            <span className="truncate font-medium">{user.login}</span>
            <Badge variant="secondary" className="capitalize">
              {user.role}
            </Badge>
          </DropdownMenuLabel>
          <DropdownMenuSeparator />
          {may("admin", user) && (
            <DropdownMenuItem asChild>
              <a href="#/users">
                <Users /> Users
              </a>
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={() => setPassword(true)}>
            <KeyRound /> Change password
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onSelect={async () => {
              await signOut().catch(() => {});
              // nothing of the archive stays in this page's memory
              queries.clear();
            }}
          >
            <LogOut /> Sign out
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <PasswordDialog open={password} onClose={() => setPassword(false)} />
    </>
  );
}
