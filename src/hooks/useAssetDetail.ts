import { useQuery } from "@tanstack/react-query"
import { useApi } from "@/lib/api"
import { qk } from "./keys"

export function useAssetDetail(assetId: number | null) {
  const api = useApi()
  return useQuery({
    queryKey: qk.asset(assetId),
    queryFn: () => api.getAssetDetail(assetId as number),
    enabled: assetId !== null,
  })
}
