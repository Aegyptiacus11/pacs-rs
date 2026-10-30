// ⌘K: find a patient or a study by any of its values, or run an action, from anywhere. The
// words go to the server as they are typed; the patients are those of the studies found.

import { useEffect, useState } from "react";
import { FileStack, FileUp, FolderUp, Loader2, User } from "lucide-react";
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator, CommandShortcut } from "@/components/ui/command";
import { useUpload } from "@/components/upload";
import { formatDate, personName, plural } from "@/lib/api";
import { go, href } from "@/lib/route";
import { useStudySearch, type Study } from "@/lib/studies";

export function CommandMenu({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const upload = useUpload();
  const [typed, setTyped] = useState("");
  const [q, setQ] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setQ(typed), 200);
    return () => clearTimeout(t);
  }, [typed]);
  useEffect(() => {
    if (!open) setTyped("");
  }, [open]);
  const found = useStudySearch(q);
  const studies = q.trim() ? (found.data ?? []) : [];
  const patients = new Map<string, { study: Study; count: number }>();
  for (const s of studies) {
    const p = patients.get(s.patient);
    patients.set(s.patient, { study: p?.study ?? s, count: (p?.count ?? 0) + 1 });
  }
  const run = (fn: () => void) => () => {
    onOpenChange(false);
    fn();
  };
  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} title="Search" description="Find a patient or a study, or run an action" shouldFilter={false}>
      <CommandInput value={typed} onValueChange={setTyped} placeholder="Name, patient ID, accession, description…" />
      <CommandList>
        {typed.trim() && !found.isFetching && studies.length === 0 && <CommandEmpty>Nothing found.</CommandEmpty>}
        {found.isFetching && typed.trim() && (
          <div className="flex items-center gap-2 px-4 py-3 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> Searching…
          </div>
        )}
        {patients.size > 0 && (
          <CommandGroup heading="Patients">
            {[...patients.values()].map(({ study: s, count }) => (
              <CommandItem key={s.patient} value={`patient ${s.patient}`} onSelect={run(() => go(href.studies(s.patient)))}>
                <User />
                <span>{personName(s.name)}</span>
                <span className="text-muted-foreground">{s.patientId}</span>
                <CommandShortcut>{plural(count, "study", "studies")} found</CommandShortcut>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        {studies.length > 0 && (
          <CommandGroup heading="Studies">
            {studies.map((s) => (
              <CommandItem key={s.id} value={`study ${s.id}`} onSelect={run(() => go(href.studies(null, s.id)))}>
                <FileStack />
                <span>{s.description || "No description"}</span>
                <span className="text-muted-foreground">{personName(s.name)}</span>
                <CommandShortcut>{formatDate(s.date)}</CommandShortcut>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        <CommandSeparator />
        <CommandGroup heading="Actions">
          <CommandItem value="upload files" onSelect={run(() => upload.pick("files"))}>
            <FileUp /> Upload files
          </CommandItem>
          <CommandItem value="upload a folder" onSelect={run(() => upload.pick("folder"))}>
            <FolderUp /> Upload a folder
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
