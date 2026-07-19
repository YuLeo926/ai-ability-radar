import { useEffect, useState } from "react";
import { useT } from "../i18n/I18nContext";

export const THEME_STORAGE_KEY = "ai-ability-radar.theme";
export type Theme = "system" | "light" | "dark";

export function isTheme(value: unknown): value is Theme {
  return value === "system" || value === "light" || value === "dark";
}

export function readStoredTheme(
  storage?: Pick<Storage, "getItem">,
): Theme {
  try {
    const stored = (storage ?? globalThis.localStorage).getItem(
      THEME_STORAGE_KEY,
    );
    return isTheme(stored) ? stored : "system";
  } catch {
    return "system";
  }
}

export function applyTheme(
  theme: Theme,
  root: HTMLElement = document.documentElement,
  storage?: Pick<Storage, "setItem" | "removeItem">,
): void {
  if (theme === "system") {
    root.removeAttribute("data-theme");
    try {
      (storage ?? globalThis.localStorage).removeItem(THEME_STORAGE_KEY);
    } catch {
      // A blocked storage implementation must not prevent applying the theme.
    }
    return;
  }

  root.dataset.theme = theme;
  try {
    (storage ?? globalThis.localStorage).setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // The explicit in-memory choice remains usable for this session.
  }
}

export function ThemeToggle() {
  const t = useT();
  const [theme, setTheme] = useState<Theme>(readStoredTheme);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  return (
    <label className="theme-control">
      <span>{t("theme.label")}</span>
      <select
        aria-label={t("theme.label")}
        onChange={(event) => {
          const value = event.target.value;
          setTheme(isTheme(value) ? value : "system");
        }}
        value={theme}
      >
        <option value="system">{t("theme.system")}</option>
        <option value="light">{t("theme.light")}</option>
        <option value="dark">{t("theme.dark")}</option>
      </select>
    </label>
  );
}
