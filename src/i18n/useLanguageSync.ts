import { useEffect } from "react"
import { useSettings } from "@/hooks/useSettings"
import { useApi } from "@/lib/api"
import { applyLanguage } from "./applyLanguage"

/**
 * 앱 셸에서 한 번 호출 — 설정이 로드되면 저장된 표시 언어를 한 번 적용한다(`useThemeSync` 와 같은 자리).
 * 실패하면 영어(기본)로 남는다. 이후의 언어 변경은 설정 화면(`LanguageSection`)이 직접 적용·저장한다.
 */
export function useLanguageSync(): void {
  const api = useApi()
  const saved = useSettings().data?.language
  useEffect(() => {
    if (saved === undefined) return
    void applyLanguage(saved, api.readLocale)
    // 시작 시 한 번만 — 설정 화면에서 바꾼 값은 거기서 이미 적용했다
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [saved === undefined])
}
