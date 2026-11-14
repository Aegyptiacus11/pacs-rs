// The settings, from the foot of the rail: the users for an administrator, the roles and what
// each one permits, one's own account when the server asks for a login, and what this server is. A section the user may not see
// is not listed, and asking for it by address shows the first one they may.

import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { CircleUser, Copy, Info, KeyRound, LogOut, ShieldCheck, Users } from "lucide-react";
import { toast } from "sonner";
import { PasswordDialog } from "@/components/user-menu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { config, ROOT, WEB } from "@/lib/api";
import { href, type SettingsSection } from "@/lib/route";
import { summary, useRoles, type Permission } from "@/lib/roles";
import { authOn, may, signOut, useSession } from "@/lib/session";
import { cn } from "@/lib/utils";
import { RolesSection } from "@/pages/roles-section";
import { UsersSection } from "@/pages/users-page";

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-1.5 px-4 py-3">
      <span className="shrink-0 text-sm text-muted-foreground">{label}</span>
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
  const roles = useRoles();
  if (!user) return null;
  const role = roles.data?.find((r) => r.id === user.role);
  return (
    <>
      <Section title="Your account" about="How you sign in to this server.">
        <Row label="Login">
          <span className="font-medium">{user.login}</span>
        </Row>
        <Row label="Role">
          <span className="hidden text-muted-foreground sm:inline">{summary(user.permissions as Permission[])}</span>
          <Badge variant="secondary">{role?.name ?? user.role}</Badge>
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
      <div className="shrink-0 border-b bg-card px-4 pt-4 md:px-8 md:pt-5">
        <h1 className="text-lg font-semibold tracking-tight">Settings</h1>
        {/* on a narrow screen the tabs scroll sideways, each on one line */}
        <nav aria-label="Settings" className="-mx-4 mt-3 flex gap-1 overflow-x-auto px-4 md:mx-0 md:px-0">
          {sections.map(({ id, label, Icon }) => (
            <a
              key={id}
              href={href.settings(id)}
              aria-current={id === current.id ? "page" : undefined}
              className={cn(
                "-mb-px flex h-10 shrink-0 items-center gap-2 border-b-2 border-transparent px-3 text-sm whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground",
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
        <div className="max-w-4xl px-4 py-5 md:px-8 md:py-6">
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
