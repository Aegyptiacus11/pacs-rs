// A link the browser downloads by itself. With a login, the browser cannot send the session's
// token on a link it follows, so a click first asks the server for a download token good for
// a minute and for this path alone, then follows the link with it.

import type { ComponentProps, MouseEvent } from "react";
import { toast } from "sonner";
import { url } from "@/lib/api";
import { authOn, downloadUrl } from "@/lib/session";

/** `path` is relative to the server's root, e.g. `studies/<id>/archive`. */
export function DownloadLink({ path, onClick, ...props }: { path: string | null } & Omit<ComponentProps<"a">, "href" | "download">) {
  async function click(e: MouseEvent<HTMLAnchorElement>) {
    onClick?.(e);
    if (!authOn || !path) return;
    e.preventDefault();
    try {
      const a = document.createElement("a");
      a.href = await downloadUrl(path);
      a.download = "";
      a.click();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "The download could not be started.");
    }
  }
  return <a href={path ? url(path) : undefined} download onClick={click} {...props} />;
}
