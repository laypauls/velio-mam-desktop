import type { UiApi } from "@/lib/api"
import type { AssetDetail, CommandError, MediaType } from "@/lib/ipc-contract"
import { DEFAULT_FILTERS, DEFAULT_SORT } from "@/stores/search"

/** mock AI 타이머를 마이크로태스크로 즉시 진행시킨다 (대기→실행→완료가 결정적으로 끝난다). */
export const immediate = (fn: () => void): void => queueMicrotask(fn)

/** 조건에 맞는 첫 자산의 상세 (유형·재생 가능·온라인). */
export async function findAsset(api: UiApi, type: MediaType, opts: { playable?: boolean } = {}): Promise<AssetDetail> {
  const page = await api.searchAssets({
    query: "",
    filters: { ...DEFAULT_FILTERS, mediaTypes: [type], includeOffline: false },
    sort: DEFAULT_SORT,
    limit: 100,
    cursor: null,
  })
  for (const item of page.items) {
    const d = await api.getAssetDetail(item.assetId)
    if (d.availability === "online" && d.status === "active" && (!opts.playable || d.playable)) return d
  }
  throw new Error(`no ${type} asset`)
}

export async function rejection(p: Promise<unknown>): Promise<CommandError> {
  try {
    await p
  } catch (e) {
    return e as CommandError
  }
  throw new Error("expected rejection")
}
