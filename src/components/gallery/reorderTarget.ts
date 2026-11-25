import type { ReorderTarget } from "@/lib/ipc-contract"

export type ReorderOp = "start" | "up" | "down" | "end"

export interface ReorderContext {
  /** 옮길 자산들 */
  selected: ReadonlySet<number>
  /** 메모리에 있는 자산의 전역 위치 (없으면 null) */
  positionOf: (assetId: number) => number | null
  /** 전역 위치의 자산 id. 아직 받지 못한 위치는 조회(`at`)로 채운다 — 범위 밖이거나 없으면 undefined */
  idAt: (position: number) => number | undefined | Promise<number | undefined>
  /** 전체 항목 수 */
  total: number
}

/** 이동 계획 — 서버에 보낼 target 과, 이동 뒤 첫 이동 항목이 놓일 전역 위치(화면에 따라가게 할 위치) */
export interface ReorderPlan {
  target: ReorderTarget
  reveal: number
}

/**
 * 이동 동작 → `reorder_collection` 의 target. 순수 계산(위치 조회는 주입) — 옮길 수 없으면(이미 끝) null.
 * 선택이 흩어져 있어도 한 덩어리로 모아서 옮긴다: 앞으로 = 첫 선택 앞의 가장 가까운 비선택 항목 앞에,
 * 뒤로 = 마지막 선택 뒤의 가장 가까운 비선택 항목 바로 다음에(그 다음 비선택 항목 앞, 없으면 맨 뒤).
 */
export async function computeTarget(op: ReorderOp, ctx: ReorderContext): Promise<ReorderTarget | null> {
  return (await planReorder(op, ctx))?.target ?? null
}

/** `computeTarget` + 이동 뒤 따라갈 위치. start → 0, end → 마지막 위치(덩어리의 끝), 앞/뒤 → 기준 항목 위치에서 앞쪽 이동분을 뺀 위치 */
export async function planReorder(op: ReorderOp, ctx: ReorderContext): Promise<ReorderPlan | null> {
  if (op === "start") return { target: { kind: "start" }, reveal: 0 }
  if (op === "end") return { target: { kind: "end" }, reveal: Math.max(0, ctx.total - 1) }
  const positions: number[] = []
  for (const id of ctx.selected) {
    const p = ctx.positionOf(id)
    if (p !== null) positions.push(p)
  }
  if (positions.length === 0) return null

  /** `from` 에서 `dir` 방향으로 처음 만나는 비선택 항목의 위치 (없으면 null) */
  const nearestOther = async (from: number, dir: 1 | -1): Promise<{ position: number; id: number } | null> => {
    for (let p = from; p >= 0 && p < ctx.total; p += dir) {
      const id = await ctx.idAt(p)
      if (id !== undefined && !ctx.selected.has(id)) return { position: p, id }
    }
    return null
  }

  if (op === "up") {
    const q = await nearestOther(Math.min(...positions) - 1, -1)
    return q ? { target: { kind: "before", assetId: q.id }, reveal: q.position } : null
  }
  const q = await nearestOther(Math.max(...positions) + 1, 1)
  if (!q) return null
  const r = await nearestOther(q.position + 1, 1)
  if (!r) return { target: { kind: "end" }, reveal: Math.max(0, ctx.total - 1) }
  // r 앞에 놓이면 r 의 옛 위치 앞에 있던 이동 항목 수만큼 당겨진다
  return { target: { kind: "before", assetId: r.id }, reveal: r.position - positions.filter((p) => p < r.position).length }
}
