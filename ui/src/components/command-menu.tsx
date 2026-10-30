// ⌘K: find a patient or a study by any of its values, or run an action, from anywhere.

import { FileStack, FileUp, FolderUp, User, Users } from "lucide-react";
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator, CommandShortcut } from "@/components/ui/command";
import { useUpload } from "@/components/upload";
import { formatDate, personName } from "@/lib/api";
import { go, href } from "@/lib/route";
import { usePatients, useStudies } from "@/lib/studies";

export function CommandMenu({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const studies = useStudies();
  const patients = usePatients();
  const upload = useUpload();
  const run = (fn: () => void) => () => {
    onOpenChange(false);
    fn();
  };
  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} title="Search" description="Find a patient or a study, or run an action">
      <CommandInput placeholder="Name, patient ID, accession, description…" />
      <CommandList>
        <CommandEmpty>Nothing found.</CommandEmpty>
        <CommandGroup heading="Patients">
          {(patients.data ?? []).slice(0, 50).map((p) => (
            <CommandItem key={p.id} value={`patient ${p.name} ${personName(p.name)} ${p.patientId}`} onSelect={run(() => go(href.studies(p.id)))}>
              <User />
              <span>{personName(p.name)}</span>
              <span className="text-muted-foreground">{p.patientId}</span>
              <CommandShortcut>{p.studies.length} studies</CommandShortcut>
            </CommandItem>
          ))}
        </CommandGroup>
        <CommandGroup heading="Studies">
          {(studies.data ?? []).slice(0, 100).map((s) => (
            <CommandItem
              key={s.id}
              value={`study ${s.description} ${s.name} ${personName(s.name)} ${s.patientId} ${s.accession} ${s.modalities.join(" ")} ${s.date}`}
              onSelect={run(() => go(href.studies(null, s.id)))}
            >
              <FileStack />
              <span>{s.description || "No description"}</span>
              <span className="text-muted-foreground">{personName(s.name)}</span>
              <CommandShortcut>{formatDate(s.date)}</CommandShortcut>
            </CommandItem>
          ))}
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Actions">
          <CommandItem onSelect={run(() => upload.pick("files"))}>
            <FileUp /> Upload files
          </CommandItem>
          <CommandItem onSelect={run(() => upload.pick("folder"))}>
            <FolderUp /> Upload a folder
          </CommandItem>
          <CommandItem onSelect={run(() => go(href.patients()))}>
            <Users /> Go to patients
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
