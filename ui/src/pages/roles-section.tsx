// Roles and permissions, in the settings: one column a role, one row a permission. Who
// manages users ticks what each role permits, adds roles, renames and removes them; everyone
// else sees the same table, read-only. The admin role is locked: it always permits everything,
// so there is always a role that can manage the rest.

import { useState, type FormEvent } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Ellipsis, Loader2, Lock, Pencil, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { api } from "@/lib/api";
import { PERMISSIONS, useRoles, type Permission, type RoleInfo } from "@/lib/roles";
import { may, useSession } from "@/lib/session";
import { cn } from "@/lib/utils";

const json = (body: unknown): RequestInit => ({ headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : fallback);

/** Add a role, or rename `role`. */
function NameDialog({ open, role, onClose }: { open: boolean; role: RoleInfo | null; onClose: () => void }) {
  const queries = useQueryClient();
  const [name, setName] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const value = name ?? role?.name ?? "";
  const close = () => {
    setName(null);
    setError(null);
    onClose();
  };
  const save = useMutation({
    mutationFn: () =>
      role
        ? api<RoleInfo>(`roles/${role.id}`, { method: "PUT", ...json({ name: value.trim() }) })
        : // a new role starts with the least: reading
          api<RoleInfo>("roles", { method: "POST", ...json({ name: value.trim(), permissions: ["read"] }) }),
    onSuccess: (saved) => {
      queries.invalidateQueries({ queryKey: ["roles"] });
      toast.success(role ? `Renamed to ${saved.name}` : `${saved.name} added`);
      close();
    },
    onError: (e) => setError(message(e, "The role was not saved.")),
  });
  function submit(e: FormEvent) {
    e.preventDefault();
    if (!value.trim()) return setError("Enter a name.");
    save.mutate();
  }
  return (
    <Dialog open={open} onOpenChange={(o) => !o && !save.isPending && close()}>
      <DialogContent className="sm:max-w-sm">
        <form onSubmit={submit} className="grid gap-4" noValidate>
          <DialogHeader>
            <DialogTitle>{role ? `Rename ${role.name}` : "Add a role"}</DialogTitle>
            <DialogDescription>
              {role ? "The users who have this role keep it." : "It starts with seeing studies and images; tick the rest in the table."}
            </DialogDescription>
          </DialogHeader>
          <label className="grid gap-1.5 text-sm font-medium">
            Name
            <Input autoFocus maxLength={40} placeholder="Resident" value={value} onChange={(e) => (setName(e.target.value), setError(null))} />
          </label>
          <p role="alert" className="min-h-5 text-sm text-destructive">
            {error}
          </p>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close} disabled={save.isPending}>
              Cancel
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {save.isPending && <Loader2 className="animate-spin" />}
              {role ? "Rename" : "Add role"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function RolesSection() {
  const { user } = useSession();
  const queries = useQueryClient();
  const roles = useRoles();
  const editable = may("admin", user);
  const [adding, setAdding] = useState(false);
  const [renaming, setRenaming] = useState<RoleInfo | null>(null);

  // the tick moves at once and goes back if the server refuses
  const toggle = useMutation({
    mutationFn: ({ role, permissions }: { role: RoleInfo; permissions: Permission[] }) =>
      api<RoleInfo>(`roles/${role.id}`, { method: "PUT", ...json({ permissions }) }),
    onMutate: async ({ role, permissions }) => {
      await queries.cancelQueries({ queryKey: ["roles"] });
      const before = queries.getQueryData<RoleInfo[]>(["roles"]);
      queries.setQueryData<RoleInfo[]>(["roles"], (list) => list?.map((r) => (r.id === role.id ? { ...r, permissions } : r)));
      return { before };
    },
    onError: (e, _change, context) => {
      queries.setQueryData(["roles"], context?.before);
      toast.error(message(e, "The role was not changed."));
    },
    onSettled: () => queries.invalidateQueries({ queryKey: ["roles"] }),
  });
  const remove = useMutation({
    mutationFn: (role: RoleInfo) => api(`roles/${role.id}`, { method: "DELETE" }),
    onSuccess: (_done, role) => {
      queries.invalidateQueries({ queryKey: ["roles"] });
      toast.success(`${role.name} removed`);
    },
    onError: (e) => toast.error(message(e, "The role was not removed.")),
  });

  return (
    <div>
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3">
        <div className="max-w-xl">
          <h2 className="text-base font-semibold tracking-tight">Roles and permissions</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Every user has one role, and a role permits what is ticked for it, nothing more.
            {editable ? " A change reaches the users who have the role within 15 minutes." : ""}
          </p>
        </div>
        {editable && (
          <Button size="sm" onClick={() => setAdding(true)}>
            <Plus /> Add role
          </Button>
        )}
      </div>
      <div className="mt-5 rounded-lg border bg-card">
        {roles.isPending ? (
          <div className="p-4">
            <Skeleton className="h-40 w-full" />
          </div>
        ) : roles.isError ? (
          <p className="p-6 text-center text-sm text-destructive">{message(roles.error, "The roles could not be read.")}</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="min-w-44">Permission</TableHead>
                {roles.data.map((role) => (
                  <TableHead key={role.id} className={cn("min-w-24 px-2 text-center", role.id === user?.role && "text-selection")}>
                    <span className="inline-flex items-center gap-1">
                      {role.name}
                      {role.locked && <Lock className="size-3 text-muted-foreground" aria-label="Always permits everything" />}
                      {editable && !role.locked && (
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button variant="ghost" size="icon-xs" aria-label={`Actions for the role ${role.name}`}>
                              <Ellipsis />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onSelect={() => setRenaming(role)}>
                              <Pencil /> Rename
                            </DropdownMenuItem>
                            <DropdownMenuItem variant="destructive" onSelect={() => remove.mutate(role)}>
                              <Trash2 /> Remove
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      )}
                    </span>
                    {role.id === user?.role && <span className="block text-[10.5px] font-normal">your role</span>}
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {PERMISSIONS.map(({ id, what, detail }) => (
                <TableRow key={id}>
                  <TableCell className="whitespace-normal">
                    <div className="font-medium">{what}</div>
                    <div className="hidden text-xs text-muted-foreground sm:block">{detail}</div>
                  </TableCell>
                  {roles.data.map((role) => {
                    const has = role.permissions.includes(id);
                    return (
                      <TableCell key={role.id} className={cn("px-2 text-center", role.id === user?.role && "bg-selection-soft/50")}>
                        <Checkbox
                          className="mx-auto"
                          checked={has}
                          disabled={!editable || role.locked}
                          aria-label={`${role.name}: ${what}`}
                          onCheckedChange={(on) =>
                            toggle.mutate({
                              role,
                              permissions: PERMISSIONS.map((p) => p.id).filter((p) => (p === id ? on === true : role.permissions.includes(p))),
                            })
                          }
                        />
                      </TableCell>
                    );
                  })}
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </div>
      {editable && (
        <p className="mt-3 text-xs text-muted-foreground">
          A role that users still have cannot be removed: give them another one first, in Users. The admin role always permits everything.
        </p>
      )}
      <NameDialog open={adding} role={null} onClose={() => setAdding(false)} />
      <NameDialog open={!!renaming} role={renaming} onClose={() => setRenaming(null)} />
    </div>
  );
}
