// The users, for an administrator: who may sign in, with which role, and whether they still
// may. Roles: a reader reads, an editor also writes, a manager also deletes, an administrator
// also manages users.

import { useState, type FormEvent, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Ellipsis, KeyRound, Loader2, Plus, Trash2, UserCheck, UserX } from "lucide-react";
import { toast } from "sonner";
import { MIN_PASSWORD } from "@/components/user-menu";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { api } from "@/lib/api";
import { useSession, type Role } from "@/lib/session";

type Account = { id: string; login: string; role: Role; active: boolean; created_ms: number };

const ROLES: { role: Role; may: string }[] = [
  { role: "reader", may: "Reads studies and images" },
  { role: "editor", may: "Also uploads, measures, and pulls from remote archives" },
  { role: "manager", may: "Also deletes" },
  { role: "admin", may: "Also manages users" },
];
const created = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
const json = (body: unknown): RequestInit => ({ headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
const tooShort = (password: string) => new TextEncoder().encode(password).length < MIN_PASSWORD;

function RoleChoice({ value, onChange }: { value: Role; onChange: (role: Role) => void }) {
  return (
    <div role="radiogroup" aria-label="Role" className="grid gap-1.5">
      {ROLES.map(({ role, may }) => (
        <button
          key={role}
          type="button"
          role="radio"
          aria-checked={value === role}
          onClick={() => onChange(role)}
          className="flex items-baseline gap-3 rounded-md border px-3 py-2 text-left text-sm outline-none transition-colors hover:bg-accent focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 aria-checked:border-selection aria-checked:bg-selection-soft"
        >
          <span className="w-16 shrink-0 font-medium capitalize">{role}</span>
          <span className="text-muted-foreground">{may}</span>
        </button>
      ))}
    </div>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="grid gap-1.5 text-sm font-medium">
      {label}
      {children}
    </label>
  );
}

function AddUser({ open, onClose }: { open: boolean; onClose: () => void }) {
  const queries = useQueryClient();
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>("reader");
  const [error, setError] = useState<string | null>(null);
  const close = () => {
    setLogin("");
    setPassword("");
    setRole("reader");
    setError(null);
    onClose();
  };
  const add = useMutation({
    mutationFn: () => api<Account>("users", { method: "POST", ...json({ login: login.trim(), password, role }) }),
    onSuccess: (user) => {
      queries.invalidateQueries({ queryKey: ["users"] });
      toast.success(`${user.login} added`);
      close();
    },
    onError: (e) => setError(e instanceof Error ? e.message : "The user was not added."),
  });
  function submit(e: FormEvent) {
    e.preventDefault();
    if (!login.trim() || /\s/.test(login.trim())) return setError("Enter a login without spaces.");
    if (tooShort(password)) return setError(`A password is at least ${MIN_PASSWORD} characters.`);
    setError(null);
    add.mutate();
  }
  return (
    <Dialog open={open} onOpenChange={(o) => !o && !add.isPending && close()}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={submit} className="grid gap-4" noValidate>
          <DialogHeader>
            <DialogTitle>Add a user</DialogTitle>
            <DialogDescription>Give them the login and the password; they can change the password once signed in.</DialogDescription>
          </DialogHeader>
          <Field label="Login">
            <Input autoComplete="off" autoCapitalize="none" spellCheck={false} placeholder="j.doe" value={login} onChange={(e) => (setLogin(e.target.value), setError(null))} />
          </Field>
          <Field label="Password">
            <Input type="password" autoComplete="new-password" value={password} onChange={(e) => (setPassword(e.target.value), setError(null))} />
          </Field>
          <div className="grid gap-1.5 text-sm font-medium">
            Role
            <RoleChoice value={role} onChange={setRole} />
          </div>
          <p role="alert" className="min-h-5 text-sm text-destructive">
            {error}
          </p>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close} disabled={add.isPending}>
              Cancel
            </Button>
            <Button type="submit" disabled={add.isPending}>
              {add.isPending && <Loader2 className="animate-spin" />}
              Add user
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ResetPassword({ user, onClose }: { user: Account | null; onClose: () => void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const close = () => {
    setPassword("");
    setError(null);
    onClose();
  };
  const reset = useMutation({
    mutationFn: () => api(`users/${user!.id}`, { method: "PUT", ...json({ password }) }),
    onSuccess: () => {
      toast.success(`Password of ${user!.login} changed`);
      close();
    },
    onError: (e) => setError(e instanceof Error ? e.message : "The password was not changed."),
  });
  return (
    <Dialog open={!!user} onOpenChange={(o) => !o && !reset.isPending && close()}>
      <DialogContent className="sm:max-w-sm">
        <form
          className="grid gap-4"
          noValidate
          onSubmit={(e) => {
            e.preventDefault();
            if (tooShort(password)) return setError(`A password is at least ${MIN_PASSWORD} characters.`);
            reset.mutate();
          }}
        >
          <DialogHeader>
            <DialogTitle>New password for {user?.login}</DialogTitle>
            <DialogDescription>Their sessions end at once: they sign in again with this password.</DialogDescription>
          </DialogHeader>
          <Field label="New password">
            <Input type="password" autoComplete="new-password" autoFocus value={password} onChange={(e) => (setPassword(e.target.value), setError(null))} />
          </Field>
          <p role="alert" className="min-h-5 text-sm text-destructive">
            {error}
          </p>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close} disabled={reset.isPending}>
              Cancel
            </Button>
            <Button type="submit" disabled={reset.isPending}>
              {reset.isPending && <Loader2 className="animate-spin" />}
              Set password
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function UsersPage() {
  const { user: me } = useSession();
  const queries = useQueryClient();
  const users = useQuery({ queryKey: ["users"], queryFn: () => api<Account[]>("users") });
  const [adding, setAdding] = useState(false);
  const [resetting, setResetting] = useState<Account | null>(null);
  const [removing, setRemoving] = useState<Account | null>(null);

  const done = () => queries.invalidateQueries({ queryKey: ["users"] });
  const failed = (e: unknown) => toast.error(e instanceof Error ? e.message : "The change was not made.");
  const change = useMutation({
    mutationFn: ({ id, ...body }: { id: string; role?: Role; active?: boolean }) => api(`users/${id}`, { method: "PUT", ...json(body) }),
    onSuccess: done,
    onError: failed,
  });
  const remove = useMutation({
    mutationFn: (id: string) => api(`users/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      done();
      toast.success(`${removing?.login} removed`);
      setRemoving(null);
    },
    onError: (e) => {
      failed(e);
      setRemoving(null);
    },
  });

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 shrink-0 items-center justify-between gap-4 border-b px-5">
        <div className="flex items-baseline gap-3">
          <h1 className="text-base font-semibold tracking-tight">Users</h1>
          {users.data && <span className="text-sm text-muted-foreground">{users.data.length}</span>}
        </div>
        <Button size="sm" onClick={() => setAdding(true)}>
          <Plus /> Add user
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-5 py-2">
        {users.isError ? (
          <p className="py-10 text-center text-sm text-destructive">{users.error instanceof Error ? users.error.message : "The users could not be read."}</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Login</TableHead>
                <TableHead className="w-40">Role</TableHead>
                <TableHead className="w-32">Status</TableHead>
                <TableHead className="w-36">Added</TableHead>
                <TableHead className="w-12" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {users.isPending &&
                [0, 1, 2].map((i) => (
                  <TableRow key={i}>
                    <TableCell colSpan={5}>
                      <Skeleton className="h-5 w-full" />
                    </TableCell>
                  </TableRow>
                ))}
              {users.data?.map((u) => (
                <TableRow key={u.id} className={u.active ? undefined : "text-muted-foreground"}>
                  <TableCell className="font-medium">
                    {u.login}
                    {u.id === me?.id && <span className="ml-2 text-xs font-normal text-muted-foreground">you</span>}
                  </TableCell>
                  <TableCell>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="sm" className="-ml-2.5 capitalize" aria-label={`Role of ${u.login}: ${u.role}`}>
                          {u.role}
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="start" className="w-80">
                        <DropdownMenuRadioGroup value={u.role} onValueChange={(role) => role !== u.role && change.mutate({ id: u.id, role: role as Role })}>
                          {ROLES.map(({ role, may }) => (
                            <DropdownMenuRadioItem key={role} value={role} className="items-baseline">
                              <span className="w-16 shrink-0 capitalize">{role}</span>
                              <span className="text-xs text-muted-foreground">{may}</span>
                            </DropdownMenuRadioItem>
                          ))}
                        </DropdownMenuRadioGroup>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                  <TableCell>{u.active ? <Badge variant="secondary">Active</Badge> : <Badge variant="outline">Deactivated</Badge>}</TableCell>
                  <TableCell className="text-muted-foreground tabular-nums">{created.format(new Date(u.created_ms))}</TableCell>
                  <TableCell>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon-sm" aria-label={`Actions for ${u.login}`}>
                          <Ellipsis />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onSelect={() => setResetting(u)}>
                          <KeyRound /> Set a new password
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => change.mutate({ id: u.id, active: !u.active })}>
                          {u.active ? <UserX /> : <UserCheck />} {u.active ? "Deactivate" : "Activate"}
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem variant="destructive" onSelect={() => setRemoving(u)}>
                          <Trash2 /> Remove
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </div>
      <AddUser open={adding} onClose={() => setAdding(false)} />
      <ResetPassword user={resetting} onClose={() => setResetting(null)} />
      <AlertDialog open={!!removing} onOpenChange={(o) => !o && !remove.isPending && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {removing?.login}?</AlertDialogTitle>
            <AlertDialogDescription>
              Their sessions end at once and the login is free again. To keep the account and stop them signing in, deactivate it.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={remove.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-white hover:bg-destructive/90"
              disabled={remove.isPending}
              onClick={(e) => {
                e.preventDefault();
                if (removing) remove.mutate(removing.id);
              }}
            >
              {remove.isPending && <Loader2 className="animate-spin" />}
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
