// Modifying a patient (POST /patients/{id}/modify) and merging one into another (/merge):
// every file of theirs is written again, the files replaced wait in the trash, and the toast's
// Undo restores that entry. For a role with `modify`.

import { useState, type FormEvent } from "react";
import { toast } from "sonner";
import { GitMerge, Loader2, Search } from "lucide-react";
import { api, initials, isoDate, personName, plural } from "@/lib/api";
import { config } from "@/lib/config";
import { href } from "@/lib/route";
import { usePatientSuggestions, useRefreshStudies, type Patient } from "@/lib/studies";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";

type Who = { name: string; patientId: string; sex: string; birth: string };
type Modification = { ID: string; Instances: number; Trash: string | null };

const kept = config.trash_days && config.trash_days > 0 ? `for ${plural(config.trash_days, "day")}` : null;

/** What the toast says after a modification or a merge, with its Undo. */
function done(what: string, answer: Modification, refresh: () => void) {
  refresh();
  location.hash = href.patients(answer.ID);
  toast.success(what, {
    description: `${plural(answer.Instances, "file")} written again.`,
    action: answer.Trash
      ? {
          label: "Undo",
          onClick: async () => {
            try {
              await api(`trash/${answer.Trash}/restore`, { method: "POST" });
              refresh();
              toast("Undone", { description: "The previous files are back." });
            } catch (e) {
              toast.error("Not undone", { description: (e as Error).message });
            }
          },
        }
      : undefined,
  });
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5 text-sm font-medium">
      {label}
      {children}
      {hint && <span className="text-xs font-normal text-muted-foreground">{hint}</span>}
    </label>
  );
}

export function ModifyPatientDialog({ id, who, open, onOpenChange }: { id: string; who: Who; open: boolean; onOpenChange: (open: boolean) => void }) {
  const [family = "", given = "", ...rest] = who.name.split("^");
  const [form, setForm] = useState({ family, given, patientId: who.patientId, birth: isoDate(who.birth), sex: who.sex });
  const [busy, setBusy] = useState(false);
  const refresh = useRefreshStudies();
  const set = (key: keyof typeof form) => (e: { target: { value: string } }) => setForm((f) => ({ ...f, [key]: e.target.value }));

  // only what changed is sent; the name keeps its other components
  const name = [form.family.trim(), form.given.trim(), ...rest].join("^").replace(/\^+$/, "");
  const changes: Record<string, string> = {};
  if (name !== who.name) changes.PatientName = name;
  if (form.patientId.trim() !== who.patientId) changes.PatientID = form.patientId.trim();
  if (form.birth.replaceAll("-", "") !== who.birth) changes.PatientBirthDate = form.birth.replaceAll("-", "");
  if (form.sex !== who.sex) changes.PatientSex = form.sex;
  const count = Object.keys(changes).length;

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const answer = await api<Modification>(`patients/${id}/modify`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(changes),
      });
      onOpenChange(false);
      done("Patient modified", answer, refresh);
    } catch (e) {
      toast.error("The patient was not modified", { description: (e as Error).message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <form onSubmit={submit} className="flex flex-col gap-5">
          <DialogHeader>
            <DialogTitle>Modify patient</DialogTitle>
            <DialogDescription>
              The new values are written into every file of {personName(who.name)}.
              {kept ? ` The previous files stay in the trash ${kept}, and Undo puts them back.` : " The previous files are removed: this cannot be undone."}
            </DialogDescription>
          </DialogHeader>
          <div className="grid grid-cols-2 gap-4">
            <Field label="Family name">
              <Input value={form.family} onChange={set("family")} autoFocus />
            </Field>
            <Field label="Given name">
              <Input value={form.given} onChange={set("given")} />
            </Field>
            <Field label="Patient ID" hint={changes.PatientID ? "A new ID changes the patient's links." : undefined}>
              <Input value={form.patientId} onChange={set("patientId")} required />
            </Field>
            <Field label="Birth date">
              <Input type="date" value={form.birth} onChange={set("birth")} />
            </Field>
            <Field label="Sex">
              <select value={form.sex} onChange={set("sex")} className="h-9 rounded-md border bg-background px-2.5 text-sm font-normal shadow-xs dark:bg-input/30">
                <option value="">Not given</option>
                <option value="F">Female</option>
                <option value="M">Male</option>
                <option value="O">Other</option>
              </select>
            </Field>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!count || busy}>
              {busy && <Loader2 className="animate-spin" />}
              {count ? `Write ${plural(count, "change")}` : "No change"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function MergePatientDialog({ id, who, studies, open, onOpenChange }: { id: string; who: Who; studies: number; open: boolean; onOpenChange: (open: boolean) => void }) {
  const [q, setQ] = useState("");
  const [into, setInto] = useState<Patient | null>(null);
  const [busy, setBusy] = useState(false);
  const refresh = useRefreshStudies();
  const found = usePatientSuggestions(q);
  const others = (found.data ?? []).filter((p) => p.id !== id);

  async function merge() {
    if (!into) return;
    setBusy(true);
    try {
      const answer = await api<Modification>(`patients/${id}/merge`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ Into: into.id }),
      });
      onOpenChange(false);
      done(`Merged into ${personName(into.name)}`, answer, refresh);
    } catch (e) {
      toast.error("The patients were not merged", { description: (e as Error).message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Merge into another patient</DialogTitle>
          <DialogDescription>
            {personName(who.name)}'s {plural(studies, "study", "studies")} take the other patient's name, ID, birth date and sex, and become theirs.
            {kept ? ` The previous files stay in the trash ${kept}, and Undo puts them back.` : " This cannot be undone."}
          </DialogDescription>
        </DialogHeader>
        <label className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="The patient to keep: name or ID" className="pl-8" aria-label="The patient to keep" autoFocus />
        </label>
        <ul aria-label="Patients" className="max-h-56 overflow-y-auto">
          {others.map((p) => (
            <li key={p.id}>
              <button
                type="button"
                aria-pressed={into?.id === p.id}
                onClick={() => setInto(p)}
                className={cn("flex w-full items-center gap-3 rounded-md px-2 py-2 text-left hover:bg-accent", into?.id === p.id && "bg-selection-soft hover:bg-selection-soft")}
              >
                <span className="grid size-8 shrink-0 place-items-center rounded-full bg-secondary text-xs text-muted-foreground">{initials(p.name)}</span>
                <span className="min-w-0">
                  <span className="block truncate font-medium">{personName(p.name)}</span>
                  <span className="block truncate text-xs text-muted-foreground">
                    {[p.patientId, plural(p.studies, "study", "studies")].join(" · ")}
                  </span>
                </span>
              </button>
            </li>
          ))}
          {q.trim() && found.data && !others.length && <li className="px-2 py-3 text-sm text-muted-foreground">No other patient matches.</li>}
        </ul>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={!into || busy} onClick={merge}>
            {busy ? <Loader2 className="animate-spin" /> : <GitMerge />}
            {into ? `Merge into ${personName(into.name)}` : "Merge"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
