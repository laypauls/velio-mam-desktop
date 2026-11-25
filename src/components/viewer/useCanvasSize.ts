import { useEffect, useRef, useState } from "react"
import { useApi } from "@/lib/api"
import type { CanvasSize } from "@/lib/ipc-contract-player"
import { MAX_CANVAS_PX } from "@/lib/ipc-player-wire"

/** 렌더 크기 변경을 호스트에 알리기 전 기다리는 시간(ms) — 창·패널을 끄는 동안 에포크가 계속 바뀌지 않게 */
export const RENDER_SIZE_DEBOUNCE_MS = 120
/** 영상 비율을 모를 때 (자산 폭·높이도 없음) */
export const FALLBACK_ASPECT = 16 / 9

/** 비율(폭/높이)을 유지하며 무대 안에 들어가는 가장 큰 크기 ("contain", CSS px) */
export function containFit(stageW: number, stageH: number, aspect: number): { width: number; height: number } {
  if (!(stageW > 0) || !(stageH > 0) || !Number.isFinite(aspect) || aspect <= 0) return { width: 0, height: 0 }
  return stageW / stageH > aspect ? { width: stageH * aspect, height: stageH } : { width: stageW, height: stageW / aspect }
}

/** CSS px → 장치 px (최소 2×2, 최대 16384 — Rust `CanvasSize` 범위 안) */
export function toDevicePx(css: { width: number; height: number }, dpr: number): CanvasSize {
  const px = (v: number) => Math.min(MAX_CANVAS_PX, Math.max(2, Math.round(v * dpr)))
  return { width: px(css.width), height: px(css.height) }
}

const same = (a: CanvasSize | null, b: CanvasSize | null) => a !== null && b !== null && a.width === b.width && a.height === b.height

export interface CanvasMetrics {
  /** 마지막으로 잰 0 이 아닌 크기(장치 px) — 처음 잴 때까지 null. 무대가 접혀 0 이 돼도 이전 값을 유지한다 */
  size: CanvasSize | null
  /** 무대가 지금 보이는 크기를 갖는가 (0×0 이면 false — 숨겨진 보기 영역) */
  visible: boolean
}

/**
 * 무대(`stageEl`)에 `aspect` 로 "contain" 한 영상의 장치 픽셀 크기를 잰다 (ResizeObserver + devicePixelRatio, 창 `resize` 포함).
 * 세션을 여는 훅이 이 크기를 필요로 하므로 보고(`useRenderSizeReport`)와 나뉘어 있다.
 */
export function useCanvasSize(stageEl: HTMLElement | null, aspect: number): CanvasMetrics {
  const [metrics, setMetrics] = useState<CanvasMetrics>({ size: null, visible: false })
  const aspectRef = useRef(aspect)
  aspectRef.current = aspect

  useEffect(() => {
    if (!stageEl) return
    const measure = () => {
      const r = stageEl.getBoundingClientRect()
      const fit = containFit(r.width, r.height, aspectRef.current)
      const visible = fit.width > 0 && fit.height > 0
      setMetrics((prev) => {
        const size = visible ? toDevicePx(fit, window.devicePixelRatio || 1) : prev.size
        return same(prev.size, size) && prev.visible === visible ? prev : { size, visible }
      })
    }
    const observer = new ResizeObserver(measure)
    observer.observe(stageEl)
    window.addEventListener("resize", measure)
    measure()
    return () => {
      observer.disconnect()
      window.removeEventListener("resize", measure)
    }
  }, [stageEl, aspect])

  return metrics
}

/**
 * `live`(세션이 열려 있음)인 동안 크기가 바뀌면 120ms 디바운스 뒤 `playerSetRenderSize` 로 알린다. 비교 기준은 `baseline`(열기에 실어 보낸 크기)
 * 또는 마지막으로 보낸 값. 무대가 접혀 있는 동안은 보내지 않는다. 실패는 삼킨다 — 다음 크기 변경 때 다시 보낸다.
 */
export function useRenderSizeReport(metrics: CanvasMetrics, live: boolean, baseline: CanvasSize | null): void {
  const api = useApi()
  const reported = useRef<CanvasSize | null>(null)
  const { size, visible } = metrics
  useEffect(() => {
    if (!live) {
      reported.current = null
      return
    }
    const known = reported.current ?? baseline
    if (size === null || !visible || same(known, size)) return
    const timer = setTimeout(() => {
      reported.current = size
      api.playerSetRenderSize(size.width, size.height).catch(() => undefined)
    }, RENDER_SIZE_DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [api, live, size, visible, baseline])
}
