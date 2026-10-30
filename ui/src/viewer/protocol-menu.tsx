// The toolbar's hanging protocols: the ones that fit this study, the previous and next one,
// whether one applies by itself when a study opens, and the user's own (save, delete).

import { useState } from "react";
import { BookmarkPlus, Check, ChevronDown, ChevronLeft, ChevronRight, LayoutTemplate, Settings2, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { LAYOUTS, type ViewerState } from "@/viewer/controller";
import { autoApply, fromCurrent, saveUserProtocols, setAutoApply, userProtocols, type Plan, type StudyContext } from "@/viewer/hanging";

function describe(p: Plan) {
  const what = p.mpr ? "MPR" : LAYOUTS[p.layout].name;
  return [what, p.linked ? "linked" : null].filter(Boolean).join(" · ");
}

export function ProtocolMenu({
  plans,
  active,
  onApply,
  state,
  ctx,
  onSaved,
}: {
  plans: Plan[];
  active: string | null;
  onApply: (plan: Plan) => void;
  state: ViewerState;
  ctx: StudyContext | null;
  onSaved: () => void;
}) {
  const [auto, setAuto] = useState(autoApply);
  const [saving, setSaving] = useState(false);
  const [managing, setManaging] = useState(false);
  const [name, setName] = useState("");
  const [modalities, setModalities] = useState<string[]>([]);
  const [mine, setMine] = useState(userProtocols);
  const current = plans.find((p) => p.protocol.id === active);
  const at = plans.findIndex((p) => p.protocol.id === active);

  const openSave = () => {
    setName(ctx ? `${ctx.modalities.join("/")} ${state.cells.length > 1 ? LAYOUTS[state.layout].name : "single"}`.trim() : "My layout");
    setModalities(ctx?.modalities ?? []);
    setSaving(true);
  };
  const save = () => {
    if (!ctx || !name.trim()) return;
    const shown = state.mode === "mpr" ? [state.cells[0]] : state.cells;
    const protocol = fromCurrent(name.trim(), state.mode === "mpr" ? 0 : state.layout, shown, ctx, state.linked, modalities);
    if (state.mode === "mpr") protocol.mpr = true;
    const list = [protocol, ...userProtocols()];
    saveUserProtocols(list);
    setMine(list);
    setSaving(false);
    toast.success("Protocol saved", { description: `${protocol.name} will open ${modalities.length ? modalities.join(", ") : "every"} study this way.` });
    onSaved();
  };
  const remove = (id: string) => {
    const list = userProtocols().filter((p) => p.id !== id);
    saveUserProtocols(list);
    setMine(list);
    onSaved();
  };

  return (
    <>
      <DropdownMenu>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label="Hanging protocols"
                className={cn(
                  "flex h-12 min-w-12 flex-col items-center justify-center gap-0.5 rounded-md px-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
                  current && "text-foreground",
                )}
              >
                <LayoutTemplate className="size-5" strokeWidth={1.6} />
                <span className="flex max-w-32 items-center gap-0.5 truncate text-[10.5px] leading-none">
                  <span className="truncate">{current?.protocol.name ?? "Protocol"}</span> <ChevronDown className="size-2.5 shrink-0" />
                </span>
              </button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent side="bottom" className="flex items-center gap-2">
            Hanging protocol <Kbd>,</Kbd> <Kbd>.</Kbd>
          </TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="start" className="w-72">
          <DropdownMenuLabel>Hanging protocols for this study</DropdownMenuLabel>
          {plans.map((p) => (
            <DropdownMenuItem key={p.protocol.id} onSelect={() => onApply(p)}>
              <Check className={cn(p.protocol.id !== active && "opacity-0")} />
              <div className="flex min-w-0 flex-col">
                <span className="truncate">{p.protocol.name}</span>
                <span className="text-xs text-muted-foreground">
                  {describe(p)}
                  {!p.protocol.builtin && " · yours"}
                </span>
              </div>
            </DropdownMenuItem>
          ))}
          {plans.length === 0 && <div className="px-2 py-1.5 text-sm text-muted-foreground">Loading…</div>}
          <DropdownMenuSeparator />
          <DropdownMenuItem disabled={at <= 0} onSelect={() => onApply(plans[at - 1])}>
            <ChevronLeft /> Previous protocol <DropdownMenuShortcut>,</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem disabled={at < 0 || at >= plans.length - 1} onSelect={() => onApply(plans[at + 1])}>
            <ChevronRight /> Next protocol <DropdownMenuShortcut>.</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuCheckboxItem
            checked={auto}
            onSelect={(e) => e.preventDefault()}
            onCheckedChange={(on) => {
              setAuto(on);
              setAutoApply(on);
            }}
          >
            Apply the best one when a study opens
          </DropdownMenuCheckboxItem>
          <DropdownMenuItem onSelect={openSave} disabled={!ctx}>
            <BookmarkPlus /> Save this layout as a protocol…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => setManaging(true)} disabled={mine.length === 0}>
            <Settings2 /> Your protocols{mine.length ? ` (${mine.length})` : ""}…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <Dialog open={saving} onOpenChange={setSaving}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Save this layout as a protocol</DialogTitle>
            <DialogDescription>
              Studies that match open the same way: the same layout, and in each viewport the same kind of series (current or prior, modality, description), whatever
              the study.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <label className="block space-y-1.5">
              <span className="text-sm font-medium">Name</span>
              <Input value={name} onChange={(e) => setName(e.target.value)} autoFocus onKeyDown={(e) => e.key === "Enter" && save()} />
            </label>
            <div className="space-y-1.5">
              <div className="text-sm font-medium">Use it for</div>
              <div className="flex flex-wrap gap-1.5">
                {(ctx?.modalities ?? []).map((m) => {
                  const on = modalities.includes(m);
                  return (
                    <button
                      key={m}
                      type="button"
                      aria-pressed={on}
                      onClick={() => setModalities((list) => (on ? list.filter((x) => x !== m) : [...list, m]))}
                      className={cn("h-7 rounded-md border px-2.5 font-mono text-xs", on ? "border-selection bg-selection-soft text-selection" : "text-muted-foreground hover:bg-accent")}
                    >
                      {m}
                    </button>
                  );
                })}
              </div>
              <p className="text-xs text-muted-foreground">{modalities.length ? `${modalities.join(", ")} studies` : "Every study, whatever its modality"}</p>
            </div>
            <div className="rounded-md border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
              {state.mode === "mpr" ? "MPR of the active series" : `${LAYOUTS[state.layout].name}, ${state.cells.filter((c) => c.seriesId).length} series${state.linked ? ", scrolling linked" : ""}`}
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setSaving(false)}>
              Cancel
            </Button>
            <Button onClick={save} disabled={!name.trim()}>
              Save protocol
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={managing} onOpenChange={setManaging}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Your protocols</DialogTitle>
            <DialogDescription>Kept in this browser. They come before the built-in ones when they fit a study.</DialogDescription>
          </DialogHeader>
          <ul className="divide-y rounded-md border">
            {mine.map((p) => (
              <li key={p.id} className="flex items-center gap-3 px-3 py-2">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{p.name}</div>
                  <div className="text-xs text-muted-foreground">
                    {p.mpr ? "MPR" : p.layout === "auto" ? "One per series" : LAYOUTS[p.layout].name} · {p.modalities.length ? p.modalities.join(", ") : "any modality"}
                  </div>
                </div>
                <Button variant="ghost" size="icon-sm" aria-label={`Delete ${p.name}`} className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive" onClick={() => remove(p.id)}>
                  <Trash2 />
                </Button>
              </li>
            ))}
            {mine.length === 0 && <li className="px-3 py-6 text-center text-sm text-muted-foreground">None yet.</li>}
          </ul>
        </DialogContent>
      </Dialog>
    </>
  );
}
