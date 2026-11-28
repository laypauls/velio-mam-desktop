import { useQuery } from "@tanstack/react-query"
import { useCallback, useEffect, useRef, useState } from "react"
import { qk } from "@/hooks/keys"
import { useSettings, useUpdateSettings } from "@/hooks/useSettings"
import { useApi } from "@/lib/api"
import type { SubtitleTrack } from "@/lib/ipc-contract"
import { useSubtitleStore } from "@/stores"
import type { GeneratedSubtitleUi } from "./useGeneratedSubtitle"
import { pickTrack } from "./subtitlePick"

/** 자막 메뉴와 재생 경로(`<video>`/libmpv)가 공유하는 자막 상태 */
export interface SubtitleUi {
  tracks: SubtitleTrack[]
  /** 사용자가 고르거나 자동으로 정해진 트랙 (설정이 꺼져 있어도 유지) */
  selectedId: string | null
  /** 실제로 표시할 트랙 — 준비 전·표시 꺼짐이면 null */
  effectiveId: string | null
  /** `null` = 자막 끄기. 끄면 설정 `subtitleVisible` 을 false 로, 트랙을 고르면 true 로 저장하고 그 언어를 이번 실행의 우선 언어로 기억한다 */
  select: (id: string | null) => void
  /** 트랙 목록을 받는 중 */
  loading: boolean
  /** 생성된 자막(음성 인식 결과) — 플레이어가 `useGeneratedSubtitle` 결과를 합쳐 넘긴다. 없으면 자막 파일 메뉴만 */
  generated?: GeneratedSubtitleUi
}

const NO_TRACKS: SubtitleTrack[] = []

/**
 * 영상 한 편의 자막 상태. 트랙은 열 때마다 다시 찾고(폴더가 바뀌었을 수 있다), 오류(오프라인 포함)는 "자막 없음" 으로 본다.
 * 자동 선택(`pickTrack`)은 트랙·설정이 준비된 뒤 자산마다 1회 — 이후 사용자가 고른 값을 다시 계산하지 않는다.
 * 설정 "자막 표시" 가 바뀌면(꺼지거나, 꺼진 채 시작한 것이 켜지면) 다시 계산한다.
 * `enabled=false` 면 조회하지 않는다 — 플레이어가 영상을 성공적으로 열기 전(또는 음성·열기 실패)에는 원본 폴더를 읽지 않는다.
 * `waiting=true` 는 아직 열리는 중이라 조회를 기다린다는 뜻으로, 자막 버튼을 "찾는 중"(비활성) 상태로 둔다.
 */
export function useSubtitles(assetId: number, enabled = true, waiting = false): SubtitleUi {
  const api = useApi()
  const settings = useSettings()
  const { mutate: saveSettings } = useUpdateSettings({ silent: true })
  const sessionLanguage = useSubtitleStore((s) => s.sessionLanguage)
  const query = useQuery({
    queryKey: qk.subtitleTracks(assetId),
    queryFn: async () => {
      try {
        return await api.subtitleTracks(assetId)
      } catch {
        return NO_TRACKS
      }
    },
    enabled,
    staleTime: 0,
    gcTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
  })
  const tracks = enabled && query.data ? query.data : NO_TRACKS
  const visible = settings.data?.subtitleVisible ?? true
  const defaultLanguage = settings.data ? settings.data.subtitleDefaultLanguage : "ko"
  const ready = enabled && query.isSuccess && (settings.isSuccess || settings.isError)

  const [sel, setSel] = useState<{ assetId: number; id: string | null } | null>(null)
  useEffect(() => {
    if (ready && sel?.assetId !== assetId) setSel({ assetId, id: pickTrack(tracks, { visible, sessionLanguage, defaultLanguage }) })
  }, [ready, sel, assetId, tracks, visible, sessionLanguage, defaultLanguage])

  // 표시 설정이 바뀌면: 꺼짐 → 선택을 버리고, 켜짐 → 선택이 "없음" 이었을 때만 다시 계산한다 (방금 고른 트랙은 유지)
  const prevVisible = useRef(visible)
  useEffect(() => {
    if (prevVisible.current === visible) return
    prevVisible.current = visible
    setSel((cur) => (cur !== null && (!visible || cur.id === null) ? null : cur))
  }, [visible])

  const selectedId = sel?.assetId === assetId ? sel.id : null
  const effectiveId = ready && visible && selectedId !== null && tracks.some((t) => t.id === selectedId) ? selectedId : null
  const select = useCallback(
    (id: string | null) => {
      setSel({ assetId, id })
      if (id === null) {
        if (visible) saveSettings({ subtitleVisible: false })
        return
      }
      const track = tracks.find((t) => t.id === id)
      if (!track) return
      useSubtitleStore.getState().setSessionLanguage(track.language)
      if (!visible) saveSettings({ subtitleVisible: true })
    },
    [assetId, tracks, visible, saveSettings]
  )
  return { tracks, selectedId, effectiveId, select, loading: waiting || (enabled && query.isPending) }
}
