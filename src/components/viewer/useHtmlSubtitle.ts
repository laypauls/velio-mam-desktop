import { useQuery } from "@tanstack/react-query"
import { useEffect } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"

/**
 * 요소마다 스크립트 텍스트 트랙 1개를 재사용한다 — `addTextTrack` 로 만든 트랙은 제거할 수 없어, 트랙을 바꿀 때마다 새로 만들면 쌓인다.
 * (CSP `media-src` 에 `blob:` 이 없어 Blob URL `<track>` 은 쓰지 않는다.)
 */
const TRACKS = new WeakMap<HTMLMediaElement, TextTrack>()

function trackOf(el: HTMLMediaElement): TextTrack {
  let track = TRACKS.get(el)
  if (!track) {
    track = el.addTextTrack("subtitles")
    TRACKS.set(el, track)
  }
  return track
}

function clearCues(track: TextTrack): void {
  for (const cue of Array.from(track.cues ?? [])) track.removeCue(cue)
}

/** `VTTCue.text` 는 WebVTT cue text 문법으로 해석된다 — `&`·`<` 를 이스케이프해야 자막 원문이 그대로 보인다 */
export function escapeCueText(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;")
}

/**
 * `<video>` 에 자막을 표시한다 — `effectiveId` 의 큐를 받아(트랙별 캐시) 요소의 텍스트 트랙에 채우고 `showing` 으로 둔다.
 * `effectiveId === null` 이면 큐를 비우고 `disabled`. 큐를 받지 못하면 토스트 1회 후 자막 없이 재생을 계속한다.
 */
export function useHtmlSubtitle(el: HTMLMediaElement | null, assetId: number, effectiveId: string | null): void {
  const api = useApi()
  const { t } = useTranslation()
  const query = useQuery({
    queryKey: qk.subtitleCues(assetId, effectiveId ?? ""),
    queryFn: () => api.subtitleCues(assetId, effectiveId ?? ""),
    enabled: el !== null && effectiveId !== null,
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 30_000,
    retry: false,
    refetchOnWindowFocus: false,
  })
  const { data, error } = query
  useEffect(() => {
    if (!el) return
    const existing = TRACKS.get(el)
    if (effectiveId === null || !data) {
      if (existing) {
        clearCues(existing)
        existing.mode = "disabled"
      }
      return
    }
    const track = trackOf(el)
    clearCues(track)
    for (const c of data) track.addCue(new VTTCue(c.startMs / 1000, c.endMs / 1000, escapeCueText(c.text)))
    track.mode = "showing"
  }, [el, effectiveId, data])
  useEffect(() => {
    if (error) toast.error(t("viewer.subtitle.loadFailed"))
  }, [error, t])
}
