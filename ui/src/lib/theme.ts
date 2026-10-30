// Light, dark, or whatever the system says; kept in this browser. The viewer is always dark
// and calls `applyTheme` when it closes, to put the chosen one back.

import { useEffect, useState } from "react";

export type Theme = "light" | "dark" | "system";
const KEY = "pacs-rs-theme";
const media = matchMedia("(prefers-color-scheme: dark)");

export function savedTheme(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    if (t === "light" || t === "dark") return t;
  } catch {}
  return "system";
}

export function applyTheme(theme: Theme = savedTheme()) {
  const dark = theme === "dark" || (theme === "system" && media.matches);
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.style.colorScheme = dark ? "dark" : "light";
}

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(savedTheme);
  useEffect(() => {
    applyTheme(theme);
    if (theme !== "system") return;
    const follow = () => applyTheme("system");
    media.addEventListener("change", follow);
    return () => media.removeEventListener("change", follow);
  }, [theme]);
  return [
    theme,
    (next: Theme) => {
      try {
        if (next === "system") localStorage.removeItem(KEY);
        else localStorage.setItem(KEY, next);
      } catch {}
      setTheme(next);
    },
  ] as const;
}
