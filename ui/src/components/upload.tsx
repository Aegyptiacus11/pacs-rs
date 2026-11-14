// Uploads: a dialog with a drop zone, or files dropped anywhere on the page. Each file goes to
// POST /instances, four at a time, and is counted as new, already stored or skipped.

import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, FileUp, FolderUp, Loader2, UploadCloud, XCircle } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api, bytes, plural } from "@/lib/api";
import { may } from "@/lib/session";
import { href } from "@/lib/route";
import { cn } from "@/lib/utils";

type Progress = {
  total: number;
  totalBytes: number;
  sentBytes: number;
  stored: number;
  already: number;
  failed: [string, string][];
  studies: Set<string>;
  lastStudy: string | null;
  running: boolean;
  seconds: number;
};

const UploadContext = createContext<{ open: () => void; pick: (kind: "files" | "folder") => void }>({ open() {}, pick() {} });
export const useUpload = () => useContext(UploadContext);

const ignored = /(^|\/)(DICOMDIR|\.DS_Store|Thumbs\.db|desktop\.ini)$/i;

async function droppedFiles(transfer: DataTransfer) {
  const entries = [...transfer.items].map((i) => i.webkitGetAsEntry?.()).filter(Boolean) as FileSystemEntry[];
  if (!entries.length) return [...transfer.files];
  const files: File[] = [];
  async function walk(entry: FileSystemEntry, path: string) {
    if (entry.isFile) {
      const file = await new Promise<File>((ok, fail) => (entry as FileSystemFileEntry).file(ok, fail));
      Object.defineProperty(file, "webkitRelativePath", { value: path + file.name });
      files.push(file);
    } else if (entry.isDirectory) {
      const reader = (entry as FileSystemDirectoryEntry).createReader();
      for (;;) {
        const batch = await new Promise<FileSystemEntry[]>((ok, fail) => reader.readEntries(ok, fail));
        if (!batch.length) break;
        for (const child of batch) await walk(child, `${path}${entry.name}/`);
      }
    }
  }
  for (const entry of entries) await walk(entry, "");
  return files;
}

export function UploadProvider({ children }: { children: ReactNode }) {
  const client = useQueryClient();
  const [open, setOpen] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const queue = useRef<File[]>([]);
  const state = useRef<Progress | null>(null);
  const filesInput = useRef<HTMLInputElement>(null);
  const folderInput = useRef<HTMLInputElement>(null);

  const start = useCallback(
    (all: File[]) => {
      const files = all.filter((f) => f.size > 0 && !ignored.test(f.webkitRelativePath || f.name));
      if (!files.length) return toast("No files to upload");
      setOpen(true);
      const size = files.reduce((n, f) => n + f.size, 0);
      queue.current.push(...files);
      if (state.current?.running) {
        state.current.total += files.length;
        state.current.totalBytes += size;
        return;
      }
      const s: Progress = { total: files.length, totalBytes: size, sentBytes: 0, stored: 0, already: 0, failed: [], studies: new Set(), lastStudy: null, running: true, seconds: 0 };
      state.current = s;
      const started = performance.now();
      let last = 0;
      const paint = () => setProgress({ ...s, failed: [...s.failed], studies: new Set(s.studies) });
      const worker = async () => {
        while (queue.current.length) {
          const file = queue.current.shift()!;
          try {
            const result = await api("instances", { method: "POST", body: file, headers: { "Content-Type": "application/dicom" } });
            if (result.Status === "AlreadyStored") s.already++;
            else s.stored++;
            s.studies.add(result.ParentStudy);
            s.lastStudy = result.ParentStudy;
          } catch (e: any) {
            s.failed.push([file.webkitRelativePath || file.name, e.status === 400 ? "Not a DICOM file" : e.message]);
          }
          s.sentBytes += file.size;
          if (performance.now() - last > 120) {
            last = performance.now();
            paint();
          }
        }
      };
      paint();
      Promise.all(Array.from({ length: 4 }, worker)).then(() => {
        s.running = false;
        s.seconds = (performance.now() - started) / 1000;
        paint();
        client.invalidateQueries({ queryKey: ["studies"] });
        toast.success(`Upload finished`, { description: `${s.stored} new, ${s.already} already stored${s.failed.length ? `, ${s.failed.length} skipped` : ""}` });
      });
    },
    [client],
  );

  // the whole window takes drops
  useEffect(() => {
    let depth = 0;
    // a role that may not write gets no drop zone: the server would refuse every file
    const hasFiles = (e: DragEvent) => may("write") && [...(e.dataTransfer?.types || [])].includes("Files");
    const enter = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth++;
      setDragging(true);
    };
    const over = (e: DragEvent) => hasFiles(e) && e.preventDefault();
    const leave = () => {
      if (--depth <= 0) {
        depth = 0;
        setDragging(false);
      }
    };
    const drop = async (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth = 0;
      setDragging(false);
      start(await droppedFiles(e.dataTransfer!));
    };
    const unload = (e: BeforeUnloadEvent) => state.current?.running && e.preventDefault();
    addEventListener("dragenter", enter);
    addEventListener("dragover", over);
    addEventListener("dragleave", leave);
    addEventListener("drop", drop);
    addEventListener("beforeunload", unload);
    return () => {
      removeEventListener("dragenter", enter);
      removeEventListener("dragover", over);
      removeEventListener("dragleave", leave);
      removeEventListener("drop", drop);
      removeEventListener("beforeunload", unload);
    };
  }, [start]);

  const pick = (kind: "files" | "folder") => (kind === "folder" ? folderInput : filesInput).current?.click();
  const p = progress;
  const done = p ? p.stored + p.already + p.failed.length : 0;
  const pct = p && p.totalBytes ? Math.round((100 * p.sentBytes) / p.totalBytes) : 0;

  return (
    <UploadContext.Provider value={{ open: () => setOpen(true), pick }}>
      {children}
      <input ref={filesInput} type="file" multiple hidden onChange={(e) => (start([...(e.target.files ?? [])]), (e.target.value = ""))} />
      <input
        ref={folderInput}
        type="file"
        multiple
        hidden
        {...({ webkitdirectory: "" } as any)}
        onChange={(e) => (start([...(e.target.files ?? [])]), (e.target.value = ""))}
      />

      {dragging && (
        <div className="pointer-events-none fixed inset-0 z-[100] grid place-items-center bg-background/80 backdrop-blur-[2px]">
          <div className="flex flex-col items-center gap-3 rounded-xl border-2 border-dashed border-selection bg-card px-16 py-12">
            <UploadCloud className="size-8 text-selection" strokeWidth={1.5} />
            <div className="text-base font-medium">Drop to upload</div>
            <div className="text-sm text-muted-foreground">DICOM files or whole folders</div>
          </div>
        </div>
      )}

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Upload studies</DialogTitle>
            <DialogDescription>Files are stored as they are; anything that isn't DICOM is skipped.</DialogDescription>
          </DialogHeader>

          {!p ? (
            <div
              className="flex flex-col items-center gap-4 rounded-lg border border-dashed bg-muted/40 px-6 py-8 text-center"
              onDragOver={(e) => e.preventDefault()}
            >
              <div className="grid size-11 place-items-center rounded-full border bg-card">
                <UploadCloud className="size-5 text-muted-foreground" strokeWidth={1.75} />
              </div>
              <div>
                <div className="font-medium">Drag files or folders here</div>
                <div className="text-sm text-muted-foreground">or choose them from your computer</div>
              </div>
              <div className="flex gap-2">
                <Button variant="outline" size="sm" onClick={() => pick("files")}>
                  <FileUp /> Choose files
                </Button>
                <Button variant="outline" size="sm" onClick={() => pick("folder")}>
                  <FolderUp /> Choose a folder
                </Button>
              </div>
            </div>
          ) : null}

          {p && (
            <div className="space-y-3">
              <div className="flex items-baseline justify-between text-sm">
                <span className="flex items-center gap-2 font-medium">
                  {p.running ? <Loader2 className="size-4 animate-spin text-muted-foreground" /> : <CheckCircle2 className="size-4 text-emerald-600" />}
                  {p.running ? `Uploading ${done.toLocaleString()} of ${p.total.toLocaleString()}` : `${plural(p.total, "file")} done in ${p.seconds.toFixed(1)} s`}
                </span>
                <span className="tabular text-muted-foreground">
                  {bytes(p.sentBytes)} / {bytes(p.totalBytes)}
                </span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                <div className={cn("h-full rounded-full bg-primary transition-[width] duration-200")} style={{ width: `${p.running ? pct : 100}%` }} />
              </div>
              <div className="grid grid-cols-3 gap-2 text-sm">
                <Stat label="New" value={p.stored} />
                <Stat label="Already stored" value={p.already} />
                <Stat label="Skipped" value={p.failed.length} warn={p.failed.length > 0} />
              </div>
              {p.failed.length > 0 && (
                <details className="rounded-md border text-sm">
                  <summary className="cursor-pointer px-3 py-2 text-muted-foreground">Skipped files</summary>
                  <ul className="max-h-40 overflow-auto border-t">
                    {p.failed.slice(0, 300).map(([name, why]) => (
                      <li key={name} className="flex items-center justify-between gap-3 border-b px-3 py-1.5 last:border-0">
                        <span className="truncate font-mono text-xs">{name}</span>
                        <span className="flex shrink-0 items-center gap-1 text-xs text-destructive">
                          <XCircle className="size-3.5" />
                          {why}
                        </span>
                      </li>
                    ))}
                  </ul>
                </details>
              )}
            </div>
          )}

          {p && !p.running && (
            <DialogFooter>
              <Button variant="outline" onClick={() => setProgress(null)}>
                Upload more
              </Button>
              {p.lastStudy && (
                <Button asChild onClick={() => setOpen(false)}>
                  <a href={href.studies()}>{p.studies.size === 1 ? "Show the study" : `Show ${p.studies.size} studies`}</a>
                </Button>
              )}
            </DialogFooter>
          )}
        </DialogContent>
      </Dialog>
    </UploadContext.Provider>
  );
}

function Stat({ label, value, warn }: { label: string; value: number; warn?: boolean }) {
  return (
    <div className="rounded-md border px-3 py-2">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className={cn("tabular text-lg font-medium", warn && "text-destructive")}>{value.toLocaleString()}</div>
    </div>
  );
}
