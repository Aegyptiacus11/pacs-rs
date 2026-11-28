// Rights per project, in the settings: which studies a user or a tool sees. Everything, or
// only the studies of some of the server's projects (those of PACS_PROJECTS). A study that
// is in no project is seen only by an account that sees everything.

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api } from "@/lib/api";

/** The projects an account is limited to; null when it sees every study. */
export type Projects = string[] | null;

/** The server's projects, for who manages users. Empty on a server without projects. */
export const useProjects = (enabled = true) => useQuery({ queryKey: ["projects"], queryFn: () => api<string[]>("projects"), staleTime: 60_000, enabled });

/** A limit in a few words, for a table row; nothing for an account that sees everything. */
export function limitLabel(projects: Projects): string | null {
  if (projects === null) return null;
  return projects.length ? `Only ${projects.join(", ")}` : "No study";
}

const choice =
  "flex items-baseline gap-3 rounded-md border px-3 py-2 text-left text-sm outline-none transition-colors hover:bg-accent focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 aria-checked:border-selection aria-checked:bg-selection-soft";

export function ProjectsChoice({ projects, value, onChange }: { projects: string[]; value: Projects; onChange: (projects: Projects) => void }) {
  const toggle = (name: string) => {
    const now = value ?? [];
    onChange(now.includes(name) ? now.filter((p) => p !== name) : [...now, name].sort());
  };
  return (
    <div className="grid gap-1.5">
      <div role="radiogroup" aria-label="Studies" className="grid gap-1.5">
        <button type="button" role="radio" aria-checked={value === null} onClick={() => onChange(null)} className={choice}>
          <span className="w-24 shrink-0 font-medium">Every study</span>
          <span className="text-muted-foreground">all projects, and studies in none</span>
        </button>
        <button type="button" role="radio" aria-checked={value !== null} onClick={() => value === null && onChange([])} className={choice}>
          <span className="w-24 shrink-0 font-medium">Only projects</span>
          <span className="text-muted-foreground">the studies of the projects ticked below</span>
        </button>
      </div>
      {value !== null && (
        <div role="group" aria-label="Projects" className="ml-4 grid max-h-40 gap-1 overflow-auto border-l pl-3">
          {projects.map((name) => (
            <button
              key={name}
              type="button"
              role="checkbox"
              aria-checked={value.includes(name)}
              onClick={() => toggle(name)}
              className="group flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50"
            >
              <span className="flex size-4 items-center justify-center rounded-sm border group-aria-checked:border-primary group-aria-checked:bg-primary group-aria-checked:text-primary-foreground">
                <Check className="size-3 opacity-0 group-aria-checked:opacity-100" />
              </span>
              <span className="truncate font-mono text-xs">{name}</span>
            </button>
          ))}
          {value.length === 0 && <p className="px-2 text-xs text-muted-foreground">With none ticked, this account sees no study.</p>}
        </div>
      )}
    </div>
  );
}

/** Changes the projects of one user (`users`) or tool (`clients`). */
export function ProjectsDialog({
  kind,
  account,
  projects,
  onClose,
}: {
  kind: "users" | "clients";
  account: { id: string; name: string; projects: Projects } | null;
  projects: string[];
  onClose: () => void;
}) {
  return (
    <Dialog open={!!account} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">{account && <Form key={account.id} kind={kind} account={account} projects={projects} onClose={onClose} />}</DialogContent>
    </Dialog>
  );
}

function Form({ kind, account, projects, onClose }: { kind: "users" | "clients"; account: { id: string; name: string; projects: Projects }; projects: string[]; onClose: () => void }) {
  const queries = useQueryClient();
  const [value, setValue] = useState<Projects>(account.projects);
  const save = useMutation({
    mutationFn: () =>
      api(`${kind}/${account.id}`, { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ projects: value }) }),
    onSuccess: () => {
      queries.invalidateQueries({ queryKey: [kind] });
      toast.success(`Studies of ${account.name} changed`);
      onClose();
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "The change was not made."),
  });
  return (
    <form
      className="grid gap-4"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
    >
      <DialogHeader>
        <DialogTitle>Studies {account.name} sees</DialogTitle>
        <DialogDescription>The role says what they may do; this says on which studies. A change holds from their next request.</DialogDescription>
      </DialogHeader>
      <ProjectsChoice projects={projects} value={value} onChange={setValue} />
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose} disabled={save.isPending}>
          Cancel
        </Button>
        <Button type="submit" disabled={save.isPending}>
          {save.isPending && <Loader2 className="animate-spin" />}
          Save
        </Button>
      </DialogFooter>
    </form>
  );
}
