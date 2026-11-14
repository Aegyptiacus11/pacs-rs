// The settings, from the foot of the rail: the users for an administrator, the roles and what
// each one permits, one's own account when the server asks for a login, and what this server is. A section the user may not see
// is not listed, and asking for it by address shows the first one they may.

import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Check, CircleUser, Copy, Info, KeyRound, LogOut, Minus, ShieldCheck, Users } from "lucide-react";
import { toast } from "sonner";
import { PasswordDialog } from "@/components/user-menu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { config, ROOT, WEB } from "@/lib/api";
import { href, type SettingsSection } from "@/lib/route";
import { authOn, may, signOut, useSession, type Role } from "@/lib/session";
import { cn } from "@/lib/utils";
import { UsersSection } from "@/pages/users-page";

const ROLE_MAY: Record<string, string> = {
  reader: "You read studies and images.",
  editor: "You read, upload, measure, and pull studies from remote archives.",
  manager: "You read, upload, measure, pull studies from remote archives, and delete.",
  admin: "You do everything, managing users included.",
};

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 px-4 py-3">
      <span className="text-sm text-muted-foreground">{label}</span>
      <span className="flex min-w-0 items-center gap-2 text-sm">{children}</span>
    </div>
  );
}

function Section({ title, about, children }: { title: string; about: string; children: ReactNode }) {
  return (
    <div>
      <h2 className="text-base font-semibold tracking-tight">{title}</h2>
      <p className="mt-1 text-sm text-muted-foreground">{about}</p>
      <div className="mt-5 divide-y rounded-lg border bg-card">{children}</div>
    </div>
  );
}

function AccountSection() {
  const { user } = useSession();
  const queries = useQueryClient();
  const [password, setPassword] = useState(false);
  if (!user) return null;
  return (
    <>
      <Section title="Your account" about="How you sign in to this server.">
        <Row label="Login">
          <span className="font-medium">{user.login}</span>
        </Row>
        <Row label="Role">
          <span className="text-muted-foreground">{ROLE_MAY[user.role]}</span>
          <Badge variant="secondary" className="capitalize">
            {user.role}
          </Badge>
        </Row>
        <Row label="Password">
          <Button variant="outline" size="sm" onClick={() => setPassword(true)}>
            <KeyRound /> Change password
          </Button>
        </Row>
        <Row label="This browser">
          <Button
            variant="outline"
            size="sm"
            onClick={async () => {
              await signOut().catch(() => {});
              queries.clear();
            }}
          >
            <LogOut /> Sign out
          </Button>
        </Row>
      </Section>
      <PasswordDialog open={password} onClose={() => setPassword(false)} />
    </>
  );
}

const ROLES: Role[] = ["reader", "editor", "manager", "admin"];
/** What the server lets each role do (`auth::right_for`): the lowest role that has it. */
const PERMISSIONS: { what: string; detail: string; from: Role }[] = [
  { what: "See studies and images", detail: "The lists, the viewer, metadata", from: "reader" },
  { what: "Search and download", detail: "Find in this archive, ZIP archives, DICOM files", from: "reader" },
  { what: "Upload studies", detail: "From the interface, STOW-RS or POST /instances", from: "editor" },
  { what: "Save measurements and hanging protocols", detail: "Shared with everyone on this server", from: "editor" },
  { what: "Use remote archives", detail: "Search another PACS and pull studies from it", from: "editor" },
  { what: "Delete", detail: "Patients, studies, series and instances", from: "manager" },
  { what: "Manage users", detail: "Add, change role, set passwords, deactivate, remove", from: "admin" },
];

function RolesSection() {
  const { user } = useSession();
  return (
    <div>
      <h2 className="text-base font-semibold tracking-tight">Roles and permissions</h2>
      <p className="mt-1 text-sm text-muted-foreground">
        Every user has one role. Each role permits what the one before it does, and more. The roles are fixed by the server.
      </p>
      <div className="mt-5 rounded-lg border bg-card">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Permission</TableHead>
              {ROLES.map((role) => (
                <TableHead key={role} className={cn("w-24 text-center capitalize", role === user?.role && "text-selection")}>
                  {role}
                  {role === user?.role && <span className="block text-[10.5px] font-normal normal-case">your role</span>}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {PERMISSIONS.map(({ what, detail, from }) => (
              <TableRow key={what}>
                <TableCell>
                  <div className="font-medium">{what}</div>
                  <div className="text-xs text-muted-foreground">{detail}</div>
                </TableCell>
                {ROLES.map((role) => {
                  const allowed = ROLES.indexOf(role) >= ROLES.indexOf(from);
                  return (
                    <TableCell key={role} className={cn("text-center", role === user?.role && "bg-selection-soft/50")}>
                      {allowed ? (
                        <Check className="mx-auto size-4" aria-label="Allowed" />
                      ) : (
                        <Minus className="mx-auto size-4 text-muted-foreground/50" aria-label="Not allowed" />
                      )}
                    </TableCell>
                  );
                })}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function Address({ value }: { value: string }) {
  return (
    <>
      <code className="truncate font-mono text-xs">{value}</code>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label="Copy the address"
        onClick={() => navigator.clipboard.writeText(value).then(() => toast.success("Address copied"))}
      >
        <Copy />
      </Button>
    </>
  );
}

function AboutSection() {
  return (
    <Section title="About this server" about="What a viewer or a script needs to connect to it.">
      <Row label="Version">
        <span className="font-medium tabular-nums">pacs-rs {config.version}</span>
      </Row>
      <Row label="DICOMweb">
        <Address value={WEB.href} />
      </Row>
      <Row label="REST API">
        <Address value={ROOT.href} />
      </Row>
      <Row label="Login">{authOn ? "Asked for on every request" : "Off: anyone who reaches this server can read and delete"}</Row>
    </Section>
  );
}

export function SettingsPage({ section }: { section: SettingsSection | null }) {
  const { user } = useSession();
  const sections = [
    { id: "users" as const, label: "Users", Icon: Users, shown: authOn && may("admin", user) },
    { id: "roles" as const, label: "Roles and permissions", Icon: ShieldCheck, shown: authOn && !!user },
    { id: "account" as const, label: "Your account", Icon: CircleUser, shown: authOn && !!user },
    { id: "about" as const, label: "About this server", Icon: Info, shown: true },
  ].filter((s) => s.shown);
  const current = sections.find((s) => s.id === section) ?? sections[0];
  return (
    <div className="flex h-full flex-col">
      <div className="shrink-0 border-b bg-card px-8 pt-5">
        <h1 className="text-lg font-semibold tracking-tight">Settings</h1>
        <nav aria-label="Settings" className="mt-3 flex gap-1">
          {sections.map(({ id, label, Icon }) => (
            <a
              key={id}
              href={href.settings(id)}
              aria-current={id === current.id ? "page" : undefined}
              className={cn(
                "-mb-px flex h-10 items-center gap-2 border-b-2 border-transparent px-3 text-sm text-muted-foreground transition-colors hover:text-foreground",
                id === current.id && "border-selection font-medium text-selection hover:text-selection",
              )}
            >
              <Icon className="size-4" strokeWidth={1.75} />
              {label}
            </a>
          ))}
        </nav>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <div className="max-w-4xl px-8 py-6">
          {current.id === "users" ? (
            <UsersSection />
          ) : current.id === "roles" ? (
            <RolesSection />
          ) : current.id === "account" ? (
            <AccountSection />
          ) : (
            <AboutSection />
          )}
        </div>
      </div>
    </div>
  );
}
