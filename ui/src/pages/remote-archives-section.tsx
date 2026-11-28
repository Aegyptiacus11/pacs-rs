// The remote archives, in the settings, for an administrator: the other DICOM archives and
// workstations this server searches, pulls studies from and sends them to. Those of
// PACS_DIMSE_REMOTES are shown, locked; the others are added, changed and removed here
// (PUT and DELETE /modalities/{id}), used by the server at once, with no restart.

import { useEffect, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Activity, Ellipsis, Loader2, Lock, Pencil, Plus, ShieldCheck, Trash2 } from "lucide-react";
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
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { api } from "@/lib/api";
import { badAeTitle, badRemoteId, deleteRemote, putRemote, useRemotes, type RemoteAe } from "@/lib/remote";

type Draft = { id: string; aet: string; host: string; port: string; tls: boolean; mayReceive: boolean };
const blank: Draft = { id: "", aet: "", host: "", port: "104", tls: false, mayReceive: false };

function Field({ label, hint, error, children }: { label: string; hint?: string; error?: string | null; children: React.ReactNode }) {
  return (
    <div className="grid gap-1.5">
      <label className="grid gap-1.5 text-sm font-medium">
        {label}
        {children}
      </label>
      {/* outside the label: the field's name stays its name */}
      {error ? <span className="text-xs text-destructive">{error}</span> : hint && <span className="text-xs text-muted-foreground">{hint}</span>}
    </div>
  );
}

/** Adding one (no `editing`), or changing one added before. */
function RemoteDialog({ open, editing, onOpenChange }: { open: boolean; editing: RemoteAe | null; onOpenChange: (open: boolean) => void }) {
  const queries = useQueryClient();
  const [draft, setDraft] = useState<Draft>(blank);
  const [tried, setTried] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!open) return;
    setTried(false);
    setDraft(editing ? { id: editing.id, aet: editing.aet, host: editing.host, port: String(editing.port), tls: editing.tls, mayReceive: editing.mayReceive } : blank);
  }, [open, editing]);
  const set = (key: keyof Draft) => (e: { target: { value: string } }) => setDraft((d) => ({ ...d, [key]: e.target.value }));
  const port = Number(draft.port);
  const errors = {
    id: editing ? null : badRemoteId(draft.id.trim()),
    aet: badAeTitle(draft.aet),
    host: draft.host.trim() ? (/\s/.test(draft.host.trim()) ? "No spaces" : null) : "Required",
    port: Number.isInteger(port) && port >= 1 && port <= 65535 ? null : "1 to 65535",
  };
  const valid = Object.values(errors).every((e) => !e);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setTried(true);
    if (!valid) return;
    setBusy(true);
    try {
      await putRemote(draft.id.trim(), { aet: draft.aet, host: draft.host, port, tls: draft.tls, mayReceive: draft.mayReceive });
      await queries.invalidateQueries({ queryKey: ["remotes"] });
      toast.success(editing ? `${draft.id} changed` : `${draft.id.trim()} added`, { description: "The server uses it from now on." });
      onOpenChange(false);
    } catch (e) {
      toast.error(editing ? `${draft.id} was not changed` : "The remote archive was not added", { description: (e as Error).message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <form onSubmit={submit} className="flex flex-col gap-5" noValidate>
          <DialogHeader>
            <DialogTitle>{editing ? `Change ${editing.id}` : "Add a remote archive"}</DialogTitle>
            <DialogDescription>
              Another DICOM archive or workstation. Its administrator registers this server too: its AE title, host and port.
            </DialogDescription>
          </DialogHeader>
          <div className="grid grid-cols-2 gap-4">
            <Field label="Name" hint="How this server names it" error={tried ? errors.id : null}>
              <Input value={draft.id} onChange={set("id")} disabled={!!editing} placeholder="hospital-pacs" autoFocus={!editing} />
            </Field>
            <Field label="AE title" error={tried ? errors.aet : null}>
              <Input value={draft.aet} onChange={set("aet")} placeholder="HOSP_PACS" className="font-mono" autoFocus={!!editing} />
            </Field>
            <Field label="Host" hint="Its name or IP address" error={tried ? errors.host : null}>
              <Input value={draft.host} onChange={set("host")} placeholder="10.0.0.5" />
            </Field>
            <Field label="Port" error={tried ? errors.port : null}>
              <Input value={draft.port} onChange={set("port")} inputMode="numeric" />
            </Field>
          </div>
          <div className="grid gap-3 text-sm">
            <label className="flex items-start gap-2.5">
              <Checkbox aria-label="Studies may be sent to it" checked={draft.mayReceive} onCheckedChange={(v) => setDraft((d) => ({ ...d, mayReceive: !!v }))} className="mt-0.5" />
              <span>
                Studies may be sent to it
                <span className="block text-xs text-muted-foreground">Send to, and the destination of a C-MOVE asked of this server. Leave it off for an archive only read from.</span>
              </span>
            </label>
            <label className="flex items-start gap-2.5">
              <Checkbox aria-label="Reached over TLS" checked={draft.tls} onCheckedChange={(v) => setDraft((d) => ({ ...d, tls: !!v }))} className="mt-0.5" />
              <span>
                Reached over TLS
                <span className="block text-xs text-muted-foreground">Needs this server's DIMSE TLS (PACS_DIMSE_TLS_CA_FILE).</span>
              </span>
            </label>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy && <Loader2 className="animate-spin" />}
              {editing ? "Save" : "Add"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** A C-ECHO on demand: how long it took, or why it failed. */
function Test({ id }: { id: string }) {
  const [state, setState] = useState<{ busy?: boolean; ms?: number; error?: string }>({});
  async function run() {
    setState({ busy: true });
    try {
      const echo = await api<{ ElapsedMs: number }>(`modalities/${encodeURIComponent(id)}/echo`, { method: "POST" });
      setState({ ms: echo.ElapsedMs });
    } catch (e) {
      setState({ error: (e as Error).message });
    }
  }
  return (
    <div className="flex items-center justify-end gap-2">
      {state.ms !== undefined && <span className="text-xs text-emerald-600 dark:text-emerald-400">Answered in {state.ms} ms</span>}
      {state.error && (
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="max-w-40 truncate text-xs text-destructive">{state.error}</span>
          </TooltipTrigger>
          <TooltipContent>{state.error}</TooltipContent>
        </Tooltip>
      )}
      <Button variant="ghost" size="sm" className="h-7" onClick={run} disabled={state.busy} aria-label={`Test the link to ${id}`}>
        {state.busy ? <Loader2 className="animate-spin" /> : <Activity />} Test
      </Button>
    </div>
  );
}

export function RemoteArchivesSection() {
  const remotes = useRemotes();
  const queries = useQueryClient();
  const [dialog, setDialog] = useState<{ editing: RemoteAe | null } | null>(null);
  const [removing, setRemoving] = useState<RemoteAe | null>(null);

  async function remove() {
    if (!removing) return;
    try {
      await deleteRemote(removing.id);
      await queries.invalidateQueries({ queryKey: ["remotes"] });
      toast.success(`${removing.id} removed`);
    } catch (e) {
      toast.error(`${removing.id} was not removed`, { description: (e as Error).message });
    } finally {
      setRemoving(null);
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold">Remote archives</h2>
          <p className="text-sm text-muted-foreground">
            The other DICOM archives and workstations this server searches, pulls studies from and sends them to. A change is used at once.
          </p>
        </div>
        <Button onClick={() => setDialog({ editing: null })}>
          <Plus /> Add
        </Button>
      </div>
      {remotes.isError ? (
        <p role="alert" className="text-sm text-destructive">
          The remote archives could not be listed: {remotes.error.message}
        </p>
      ) : (
        <div className="overflow-x-auto rounded-lg border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>AE title</TableHead>
                <TableHead>Address</TableHead>
                <TableHead>Studies sent to it</TableHead>
                <TableHead className="w-0" />
                <TableHead className="w-0" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {remotes.isPending &&
                Array.from({ length: 2 }, (_, i) => (
                  <TableRow key={i}>
                    <TableCell colSpan={6}>
                      <Skeleton className="h-4 w-full" />
                    </TableCell>
                  </TableRow>
                ))}
              {remotes.data?.length === 0 && (
                <TableRow>
                  <TableCell colSpan={6} className="py-8 text-center text-sm text-muted-foreground">
                    No remote archive yet.
                  </TableCell>
                </TableRow>
              )}
              {remotes.data?.map((r) => (
                <TableRow key={r.id}>
                  <TableCell className="font-medium">
                    <span className="flex items-center gap-1.5">
                      {r.id}
                      {r.fixed && (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <Lock className="size-3.5 text-muted-foreground" aria-label="Set in the server's settings" />
                          </TooltipTrigger>
                          <TooltipContent>Set in PACS_DIMSE_REMOTES: changed there</TooltipContent>
                        </Tooltip>
                      )}
                    </span>
                  </TableCell>
                  <TableCell className="font-mono text-xs">{r.aet}</TableCell>
                  <TableCell className="font-mono text-xs">
                    <span className="flex items-center gap-1.5">
                      {r.host}:{r.port}
                      {r.tls && <ShieldCheck className="size-3.5 text-muted-foreground" aria-label="over TLS" />}
                    </span>
                  </TableCell>
                  <TableCell className="text-sm">{r.mayReceive ? "Yes" : <span className="text-muted-foreground">No</span>}</TableCell>
                  <TableCell>
                    <Test id={r.id} />
                  </TableCell>
                  <TableCell>
                    {!r.fixed && (
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button variant="ghost" size="icon-sm" aria-label={`Actions on ${r.id}`}>
                            <Ellipsis />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onSelect={() => setDialog({ editing: r })}>
                            <Pencil /> Change…
                          </DropdownMenuItem>
                          <DropdownMenuItem variant="destructive" onSelect={() => setRemoving(r)}>
                            <Trash2 /> Remove…
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <RemoteDialog open={!!dialog} editing={dialog?.editing ?? null} onOpenChange={(open) => !open && setDialog(null)} />
      <AlertDialog open={!!removing} onOpenChange={(open) => !open && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {removing?.id}?</AlertDialogTitle>
            <AlertDialogDescription>
              This server stops searching it, pulling from it and sending to it, and no longer answers it if only known callers are let in. Its studies already here stay.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={remove} className="bg-destructive text-white hover:bg-destructive/90">
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
