import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { useState } from "react"
import { AppErrorBoundary } from "@/components/shell/AppErrorBoundary"
import { AppShell } from "@/components/shell/AppShell"
import { UpdateManager } from "@/components/update/UpdateManager"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { ApiProvider, createDefaultApi, type UiApi } from "@/lib/api"

interface Props {
  /** 테스트·스토리용 주입. 생략하면 런타임에 맞는 구현을 고른다 (D2). */
  api?: UiApi
  queryClient?: QueryClient
  /** 시작 시 업데이트 자동 확인까지의 지연 (첫 화면 렌더와 경쟁하지 않도록). 테스트에서 주입 */
  updateCheckDelayMs?: number
}

export const UPDATE_CHECK_DELAY_MS = 5000

function App({ api, queryClient, updateCheckDelayMs = UPDATE_CHECK_DELAY_MS }: Props) {
  const [defaultApi] = useState<UiApi>(() => api ?? createDefaultApi())
  const [defaultClient] = useState(() => queryClient ?? new QueryClient({ defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } } }))
  return (
    <AppErrorBoundary>
      <QueryClientProvider client={defaultClient}>
        <ApiProvider api={api ?? defaultApi}>
          <TooltipProvider>
            <AppShell />
            <UpdateManager startupDelayMs={updateCheckDelayMs} />
            <Toaster />
          </TooltipProvider>
        </ApiProvider>
      </QueryClientProvider>
    </AppErrorBoundary>
  )
}

export default App
