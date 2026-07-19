import { createContext, useContext, useMemo, type ReactNode } from "react";
import { translate, type Translator } from "./messages";

const I18nContext = createContext<Translator>(translate);

export function I18nProvider({ children }: { children: ReactNode }) {
  const value = useMemo<Translator>(() => translate, []);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useT(): Translator {
  return useContext(I18nContext);
}
