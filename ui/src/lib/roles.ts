// Roles and what they permit, as the server keeps them (/roles). The five permissions are the
// server's; a role has the ones it is given, and none includes another.

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { authOn } from "@/lib/session";

export type Permission = "read" | "write" | "remote" | "delete" | "users";
export type RoleInfo = { id: string; name: string; permissions: Permission[]; locked: boolean };

export const PERMISSIONS: { id: Permission; what: string; detail: string }[] = [
  { id: "read", what: "See studies and images", detail: "The lists, the viewer, searches, ZIP archives and DICOM files" },
  { id: "write", what: "Upload and annotate", detail: "Uploads, measurements and hanging protocols" },
  { id: "remote", what: "Use remote archives", detail: "Search another PACS and pull studies from it" },
  { id: "delete", what: "Delete", detail: "Patients, studies, series and instances" },
  { id: "users", what: "Manage users and roles", detail: "Users, their roles, and what each role permits" },
];

/** "See studies and images, delete", for a line that sums a role up. */
export function summary(permissions: Permission[]) {
  const parts = PERMISSIONS.filter((p) => permissions.includes(p.id)).map((p) => p.what.toLowerCase());
  if (!parts.length) return "Nothing but their own account";
  return parts.join(", ").replace(/^./, (c) => c.toUpperCase());
}

export function useRoles() {
  return useQuery({ queryKey: ["roles"], queryFn: () => api<RoleInfo[]>("roles"), enabled: authOn });
}
