import { createContext, useContext, type ReactNode } from "react"
import type { UiApi } from "./types"

const ApiContext = createContext<UiApi | null>(null)

export function ApiProvider({ api, children }: { api: UiApi; children: ReactNode }) {
  return <ApiContext.Provider value={api}>{children}</ApiContext.Provider>
}

export function useApi(): UiApi {
  const api = useContext(ApiContext)
  if (!api) throw new Error("useApi must be used within ApiProvider")
  return api
}
