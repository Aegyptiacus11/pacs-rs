// The tools, in the settings, for who manages users: the programs that connect without a
// person (a viewer, a pipeline). Each has an ID and a secret, which it exchanges for a short
// access token (POST /auth/token); its role bounds what that token allows.

import { useEffect, useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, Ellipsis, Loader2, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
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
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { api, url } from "@/lib/api";
import { summary, useRoles, type Permission, type RoleInfo } from "@/lib/roles";
import { Field, RoleChoice } from "@/pages/users-page";

type Tool = { id: string; name: string; role: string; created_by: string; created_ms: number; last_token_ms: number | null };
type NewTool = Tool & { secret: string };

const day = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
const moment = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
const json = (body: unknown): RequestInit => ({ headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });

/** A role as a tool has it: a tool never manages users, whatever its role says. */
const forTool = (role: RoleInfo): RoleInfo => ({ ...role, permissions: role.permissions.filter((p: Permission) => p !== "users") });

function Secret({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="grid gap-1.5 text-sm font-medium">
      {label}
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 rounded-md border bg-muted px-2.5 py-2 font-mono text-xs break-all select-all">{value}</code>
        <Button
          type="button"
          variant="outline"
          size="icon-sm"
          aria-label={`Copy the ${label.toLowerCase()}`}
          onClick={() =>
            navigator.clipboard.writeText(value).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            })
          }
        >
          {copied ? <Check /> : <Copy />}
        </Button>
      </div>
    </div>
  );
}

function AddTool({ open, roles, onClose }: { open: boolean; roles: RoleInfo[]; onClose: () => void }) {
  const queries = useQueryClient();
  const [name, setName] = useState("");
  const [chosen, setRole] = useState<string | null>(null);
  const role = chosen ?? roles[0]?.id ?? "";
  const [error, setError] = useState<string | null>(null);
  const [made, setMade] = useState<NewTool | null>(null);
  // emptied when it opens, not when it closes: the secret stays put while the dialog fades out
  useEffect(() => {
    if (!open) return;
    setName("");
    setRole(null);
    setError(null);
    setMade(null);
  }, [open]);
  const close = onClose;
  const add = useMutation({
    mutationFn: () => api<NewTool>("clients", { method: "POST", ...json({ name: name.trim(), role }) }),
    onSuccess: (tool) => {
      queries.invalidateQueries({ queryKey: ["clients"] });
      setMade(tool);
    },
    onError: (e) => setError(e instanceof Error ? e.message : "The tool was not added."),
  });
  function submit(e: FormEvent) {
    e.preventDefault();
    if (!name.trim()) return setError("Enter a name.");
    setError(null);
    add.mutate();
  }
  // the secret is on screen once: closing by a click outside would lose it
  const keep = (e: Event) => made && e.preventDefault();
  return (
    <Dialog open={open} onOpenChange={(o) => !o && !add.isPending && close()}>
      <DialogContent className="sm:max-w-md" onInteractOutside={keep} onEscapeKeyDown={keep}>
        {made ? (
          <div className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{made.name} added</DialogTitle>
              <DialogDescription>Copy the secret now: it is shown this once, and the server does not keep it.</DialogDescription>
            </DialogHeader>
            <Secret label="ID" value={made.id} />
            <Secret label="Secret" value={made.secret} />
            <Secret label="Token address" value={url("auth/token")} />
            <p className="text-sm text-muted-foreground">
              The tool posts <code className="font-mono text-xs">grant_type=client_credentials</code> to the token address, with the ID and the secret as HTTP
              Basic, and sends the access token it gets as <code className="font-mono text-xs">Authorization: Bearer</code>. A token lasts five minutes.
            </p>
            <DialogFooter>
              <Button type="button" onClick={close}>
                I copied the secret
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={submit} className="grid gap-4" noValidate>
            <DialogHeader>
              <DialogTitle>Add a tool</DialogTitle>
              <DialogDescription>A viewer, a script or a pipeline that connects without a person signing in.</DialogDescription>
            </DialogHeader>
            <Field label="Name">
              <Input autoComplete="off" autoFocus maxLength={60} placeholder="Nightly pipeline" value={name} onChange={(e) => (setName(e.target.value), setError(null))} />
            </Field>
            <div className="grid gap-1.5 text-sm font-medium">
              Role
              <RoleChoice roles={roles} value={role} onChange={setRole} />
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
                Add tool
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}

export function ToolsSection() {
  const queries = useQueryClient();
  const tools = useQuery({ queryKey: ["clients"], queryFn: () => api<Tool[]>("clients") });
  const roles = (useRoles().data ?? []).map(forTool);
  const roleName = (id: string) => roles.find((r) => r.id === id)?.name ?? id;
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<Tool | null>(null);

  const done = () => queries.invalidateQueries({ queryKey: ["clients"] });
  const failed = (e: unknown) => toast.error(e instanceof Error ? e.message : "The change was not made.");
  const change = useMutation({
    mutationFn: ({ id, role }: { id: string; role: string }) => api(`clients/${id}`, { method: "PUT", ...json({ role }) }),
    onSuccess: done,
    onError: failed,
  });
  const remove = useMutation({
    mutationFn: (id: string) => api(`clients/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      done();
      toast.success(`${removing?.name} removed`);
      setRemoving(null);
    },
    onError: (e) => {
      failed(e);
      setRemoving(null);
    },
  });

  return (
    <div>
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3">
        <div>
          <h2 className="text-base font-semibold tracking-tight">Tools</h2>
          <p className="mt-1 max-w-xl text-sm text-muted-foreground">
            Programs that connect without a person signing in: a viewer, a script, a pipeline. Each one has its own secret and a role; it never manages users.
          </p>
        </div>
        <Button size="sm" onClick={() => setAdding(true)}>
          <Plus /> Add tool
        </Button>
      </div>
      <div className="mt-5 rounded-lg border bg-card">
        {tools.isError ? (
          <p className="py-10 text-center text-sm text-destructive">{tools.error instanceof Error ? tools.error.message : "The tools could not be read."}</p>
        ) : tools.data?.length === 0 ? (
          <p className="px-4 py-10 text-center text-sm text-muted-foreground">No tool yet. Add one to let a program read or write this archive.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead className="w-28 md:w-36">Role</TableHead>
                <TableHead className="hidden w-40 sm:table-cell">Last token</TableHead>
                <TableHead className="hidden w-44 md:table-cell">Added</TableHead>
                <TableHead className="w-12" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {tools.isPending &&
                [0, 1].map((i) => (
                  <TableRow key={i}>
                    <TableCell colSpan={5}>
                      <Skeleton className="h-5 w-full" />
                    </TableCell>
                  </TableRow>
                ))}
              {tools.data?.map((t) => (
                <TableRow key={t.id}>
                  <TableCell className="max-w-40">
                    <div className="truncate font-medium">{t.name}</div>
                    <div className="truncate font-mono text-xs text-muted-foreground">{t.id}</div>
                  </TableCell>
                  <TableCell>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="sm" className="-ml-2.5" aria-label={`Role of ${t.name}: ${roleName(t.role)}`}>
                          {roleName(t.role)}
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="start" className="w-[min(20rem,calc(100vw-2rem))]">
                        <DropdownMenuRadioGroup value={t.role} onValueChange={(role) => role !== t.role && change.mutate({ id: t.id, role })}>
                          {roles.map((role) => (
                            <DropdownMenuRadioItem key={role.id} value={role.id} className="items-baseline">
                              <span className="w-24 shrink-0 truncate">{role.name}</span>
                              <span className="text-xs text-muted-foreground">{summary(role.permissions)}</span>
                            </DropdownMenuRadioItem>
                          ))}
                        </DropdownMenuRadioGroup>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                  <TableCell className="hidden text-muted-foreground tabular-nums sm:table-cell">
                    {t.last_token_ms ? moment.format(new Date(t.last_token_ms)) : "Never"}
                  </TableCell>
                  <TableCell className="hidden text-muted-foreground md:table-cell">
                    <span className="tabular-nums">{day.format(new Date(t.created_ms))}</span> by {t.created_by}
                  </TableCell>
                  <TableCell>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon-sm" aria-label={`Actions for ${t.name}`}>
                          <Ellipsis />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onSelect={() => navigator.clipboard.writeText(t.id).then(() => toast.success("ID copied"))}>
                          <Copy /> Copy the ID
                        </DropdownMenuItem>
                        <DropdownMenuItem variant="destructive" onSelect={() => setRemoving(t)}>
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
      <AddTool open={adding} roles={roles} onClose={() => setAdding(false)} />
      <AlertDialog open={!!removing} onOpenChange={(o) => !o && !remove.isPending && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {removing?.name}?</AlertDialogTitle>
            <AlertDialogDescription>Its secret stops working at once, and so do the access tokens it already has. This cannot be undone.</AlertDialogDescription>
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
