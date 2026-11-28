// The study's key images: those saved (the study's Key Object Selections) and those marked here
// and not saved yet, each shown with a click; Save makes the new ones a Key Object Selection of
// their study, with a description if one is given.

import { useState } from "react";
import { Loader2, Star, X } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { may } from "@/lib/session";
import type { KeyState, Viewer } from "@/viewer/controller";

export function KeyImagesPanel({ viewer, keys, onClose }: { viewer: Viewer; keys: KeyState[]; onClose: () => void }) {
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const pending = keys.filter((k) => !k.saved).length;
  async function save() {
    setBusy(true);
    try {
      const done = await viewer.saveKeys(description);
      if ("why" in done) toast.error("The key images were not saved", { description: done.why });
      else if (done.saved === 0) toast.info("Already saved", { description: "The study's key image series holds them already." });
      else {
        toast.success(`${done.saved === 1 ? "1 key image" : `${done.saved} key images`} saved`, { description: "In a key image series of the study, for any viewer to show." });
        setDescription("");
      }
    } finally {
      setBusy(false);
    }
  }
  return (
    <section aria-label="Key images" className="absolute top-2 right-2 z-20 flex max-h-[calc(100%-1rem)] w-72 flex-col rounded-lg border bg-popover/95 text-sm shadow-lg backdrop-blur">
      <div className="flex items-center justify-between border-b py-1.5 pr-1.5 pl-3">
        <h2 className="font-medium">Key images</h2>
        <Button variant="ghost" size="icon-sm" aria-label="Close the key images" onClick={onClose}>
          <X />
        </Button>
      </div>
      <ul aria-label="Key images of the study" className="min-h-0 overflow-y-auto p-1.5">
        {keys.map((k) => (
          <li key={k.id} className="flex items-center gap-1">
            <button className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 py-1 text-left hover:bg-accent" onClick={() => viewer.goToKey(k.id)} title="Show it">
              <Star className={cn("size-3.5 shrink-0", k.saved ? "fill-current text-amber-400" : "text-amber-400")} />
              <span className="truncate">{k.label}</span>
            </button>
            {k.saved ? (
              <span className="px-1 text-[10px] text-muted-foreground">saved</span>
            ) : (
              <Button variant="ghost" size="icon-xs" aria-label={`Unmark ${k.label}`} onClick={() => viewer.unmarkKey(k.id)}>
                <X />
              </Button>
            )}
          </li>
        ))}
      </ul>
      {pending > 0 && may("write") && (
        <div className="flex flex-col gap-2 border-t p-2">
          <Input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What they show (optional)" aria-label="Key images description" className="h-8" />
          <Button size="sm" onClick={save} disabled={busy}>
            {busy && <Loader2 className="animate-spin" />}
            Save {pending === 1 ? "1 key image" : `${pending} key images`}
          </Button>
        </div>
      )}
    </section>
  );
}
