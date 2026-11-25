import { useQuery } from "@tanstack/react-query"
import { useApi } from "@/lib/api"
import { qk } from "./keys"

/** 언어 폴더의 언어 파일 목록. 폴더는 앱 밖에서 바뀌므로 자동 재조회하지 않고 `refetch`(새로고침 버튼)로만 갱신한다. */
export function useLocales() {
  const api = useApi()
  return useQuery({
    queryKey: qk.locales,
    queryFn: () => api.listLocales(),
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  })
}
