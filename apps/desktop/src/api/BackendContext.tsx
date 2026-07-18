import { createContext, useContext, type ReactNode } from "react";
import type { Backend } from "./backend";
import { tauriBackend } from "./tauriBackend";

const BackendContext = createContext<Backend>(tauriBackend);

export function BackendProvider({
  backend,
  children,
}: {
  backend: Backend;
  children: ReactNode;
}) {
  return (
    <BackendContext.Provider value={backend}>
      {children}
    </BackendContext.Provider>
  );
}

export function useBackend(): Backend {
  return useContext(BackendContext);
}
