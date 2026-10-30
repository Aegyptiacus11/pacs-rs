// Patients: one row per Patient ID; a row shows that patient's studies; ⋯ (on hover) or a right
// click has the rest.

import { useMemo, useState } from "react";
import { Download, Ellipsis, FileStack, Search, Trash2, Users } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { ConfirmDelete, type PendingDelete } from "@/components/confirm-delete";
import { api, formatDate, initials, personName, plural, sexAge, url } from "@/lib/api";
import { go, href } from "@/lib/route";
import { usePatients, useRefreshStudies, type Patient } from "@/lib/studies";

export function PatientsPage() {
  const patients = usePatients();
  const refresh = useRefreshStudies();
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState<PendingDelete | null>(null);
  const rows = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return (patients.data ?? [])
      .filter((p) => words.every((w) => `${p.name} ${personName(p.name)} ${p.patientId}`.toLowerCase().includes(w)))
      .sort((a, b) => personName(a.name).localeCompare(personName(b.name)));
  }, [patients.data, query]);

  return (
    <div className="flex h-full flex-col">
      <div className="px-6 pt-5 pb-3">
        <h1 className="text-lg font-semibold tracking-tight">Patients</h1>
        <p className="text-sm text-muted-foreground">{patients.isPending ? "Loading…" : plural(rows.length, "patient")}</p>
      </div>
      <div className="relative mx-6 mb-3 w-full sm:w-72">
        <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Name or patient ID…" className="h-8 bg-card pl-8" />
      </div>
      <div className="min-h-0 flex-1 overflow-auto border-t">
        <Table>
          <TableHeader className="sticky top-0 z-10 bg-background shadow-[inset_0_-1px_0_var(--border)]">
            <TableRow className="border-0 hover:bg-transparent">
              <TableHead className="h-9 pl-6 text-xs font-medium text-muted-foreground">Patient</TableHead>
              <TableHead className="h-9 text-xs font-medium text-muted-foreground">Born</TableHead>
              <TableHead className="h-9 text-xs font-medium text-muted-foreground">Modalities</TableHead>
              <TableHead className="h-9 text-xs font-medium text-muted-foreground">Last study</TableHead>
              <TableHead className="h-9 text-right text-xs font-medium text-muted-foreground">Studies</TableHead>
              <TableHead className="h-9 text-right text-xs font-medium text-muted-foreground">Images</TableHead>
              <TableHead className="h-9 w-14 pr-6"><span className="sr-only">Actions</span></TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {patients.isPending &&
              Array.from({ length: 4 }, (_, i) => (
                <TableRow key={i} className="h-12">
                  <TableCell colSpan={7} className="px-6">
                    <Skeleton className="h-3 w-64" />
                  </TableCell>
                </TableRow>
              ))}
            {rows.map((p) => (
              <ContextMenu key={p.id}>
                <ContextMenuTrigger asChild>
                  <TableRow className="group h-12 cursor-pointer" tabIndex={0} onClick={() => go(href.studies(p.id))} onKeyDown={(e) => e.key === "Enter" && go(href.studies(p.id))}>
                    <TableCell className="py-1.5 pl-6">
                      <div className="flex items-center gap-3">
                        <span className="grid size-8 shrink-0 place-items-center rounded-full bg-secondary text-xs font-medium text-muted-foreground ring-1 ring-border">{initials(p.name)}</span>
                        <div className="min-w-0">
                          <div className="truncate font-medium">{personName(p.name)}</div>
                          <div className="truncate text-xs text-muted-foreground">{[p.patientId, sexAge(p.sex, p.birth)].filter(Boolean).join(" · ")}</div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="tabular">{formatDate(p.birth)}</TableCell>
                    <TableCell>
                      <div className="flex gap-1">
                        {p.modalities.sort().map((m) => (
                          <Badge key={m} variant="outline" className="h-5 rounded px-1.5 font-mono text-[11px] text-muted-foreground">
                            {m}
                          </Badge>
                        ))}
                      </div>
                    </TableCell>
                    <TableCell className="tabular">{formatDate(p.last)}</TableCell>
                    <TableCell className="tabular text-right">{p.studies.length}</TableCell>
                    <TableCell className="tabular text-right">{p.instances.toLocaleString()}</TableCell>
                    <TableCell className="pr-6 text-right" onClick={(e) => e.stopPropagation()}>
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button variant="ghost" size="icon-sm" aria-label="Patient actions" className="text-muted-foreground opacity-0 group-hover:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100 [@media(hover:none)]:opacity-100">
                            <Ellipsis />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="w-52">
                          <PatientMenu patient={p} kind="dropdown" onDelete={setPending} />
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </TableCell>
                  </TableRow>
                </ContextMenuTrigger>
                <ContextMenuContent className="w-52">
                  <PatientMenu patient={p} kind="context" onDelete={setPending} />
                </ContextMenuContent>
              </ContextMenu>
            ))}
          </TableBody>
        </Table>
        {!patients.isPending && rows.length === 0 && (
          <div className="flex flex-col items-center px-6 py-20 text-center">
            <div className="mb-4 grid size-11 place-items-center rounded-full border bg-background">
              <Users className="size-5 text-muted-foreground" strokeWidth={1.75} />
            </div>
            <div className="font-medium">{query ? "No patient matches" : "No patients yet"}</div>
          </div>
        )}
      </div>
      <ConfirmDelete
        pending={pending}
        onCancel={() => setPending(null)}
        onConfirm={async () => {
          try {
            await api(`patients/${pending!.ids[0]}`, { method: "DELETE" });
            toast.success("Patient deleted");
          } catch (e: any) {
            toast.error("Couldn't delete the patient", { description: e.message });
          }
          setPending(null);
          await refresh();
        }}
      />
    </div>
  );
}

function PatientMenu({ patient: p, kind, onDelete }: { patient: Patient; kind: "dropdown" | "context"; onDelete: (d: PendingDelete) => void }) {
  const [Item, Separator] = kind === "dropdown" ? [DropdownMenuItem, DropdownMenuSeparator] : [ContextMenuItem, ContextMenuSeparator];
  return (
    <>
      <Item onSelect={() => go(href.studies(p.id))}>
        <FileStack /> Studies
      </Item>
      <Item asChild>
        <a href={url(`patients/${p.id}/archive`)} download>
          <Download /> Download ZIP
        </a>
      </Item>
      <Separator />
      <Item
        variant="destructive"
        onSelect={() =>
          onDelete({
            ids: [p.id],
            title: "Delete this patient?",
            description: `${personName(p.name)} (${p.patientId || "no ID"}) and all ${plural(p.studies.length, "study", "studies")}, ${plural(p.instances, "image")}, will be removed from storage. This can't be undone.`,
          })
        }
      >
        <Trash2 /> Delete patient…
      </Item>
    </>
  );
}
