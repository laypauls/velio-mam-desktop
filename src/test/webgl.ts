import { vi } from "vitest"

export interface GlUpload {
  w: number
  h: number
  rowLength: number
  bytes: number
  /** 업로드 시점의 캔버스 속성 크기 */
  canvas: { w: number; h: number }
}

export interface FakeWebgl {
  /** 호출 순서 기록 — "viewport:WxH" / "texImage2D:WxH" / "texSubImage2D:WxH" / "draw" */
  calls: string[]
  uploads: GlUpload[]
  /** `shaderSource` 로 넘어온 GLSL 원문 */
  sources: string[]
  /** `getContext("webgl2", options)` 에 넘어온 옵션 */
  contextOptions: unknown[]
  /** `drawArrays` 가 호출된 시점의 캔버스 속성 크기 */
  draws: { w: number; h: number }[]
  /** `getContext("webgl2")` 가 요청된 횟수 */
  contextRequests: () => number
  loseContext: (canvas: HTMLCanvasElement) => Event
  restoreContext: (canvas: HTMLCanvasElement) => void
}

interface Options {
  /** true 면 `getContext("webgl2")` 가 null (WebGL2 미지원) */
  unsupported?: boolean
}

const CONSTANTS = {
  VERTEX_SHADER: 1, FRAGMENT_SHADER: 2, COMPILE_STATUS: 3, LINK_STATUS: 4, TEXTURE_2D: 5, TEXTURE_MIN_FILTER: 6, TEXTURE_MAG_FILTER: 7,
  TEXTURE_WRAP_S: 8, TEXTURE_WRAP_T: 9, LINEAR: 10, CLAMP_TO_EDGE: 11, UNPACK_ALIGNMENT: 12, UNPACK_ROW_LENGTH: 13, RGBA8: 14, RGBA: 15,
  UNSIGNED_BYTE: 16, TRIANGLES: 17,
}

/**
 * WebGL2 스텁 — jsdom 에는 컨텍스트가 없다. `HTMLCanvasElement.prototype.getContext` 를 가로채 호출을 기록하는 가짜 컨텍스트를 준다.
 * `vi.restoreAllMocks()` 로 되돌린다.
 */
export function installFakeWebgl(options: Options = {}): FakeWebgl {
  let requests = 0
  const rec: FakeWebgl = {
    calls: [],
    uploads: [],
    sources: [],
    contextOptions: [],
    draws: [],
    contextRequests: () => requests,
    loseContext: (canvas) => {
      const e = new Event("webglcontextlost", { cancelable: true })
      canvas.dispatchEvent(e)
      return e
    },
    restoreContext: (canvas) => void canvas.dispatchEvent(new Event("webglcontextrestored")),
  }
  const contexts = new Map<HTMLCanvasElement, unknown>()
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(function (this: HTMLCanvasElement, type: string, attrs?: unknown) {
    if (type !== "webgl2") return null
    requests += 1
    rec.contextOptions.push(attrs)
    if (options.unsupported) return null
    const existing = contexts.get(this)
    if (existing) return existing as never
    const canvas = this
    let rowLength = 0
    const gl = {
      ...CONSTANTS,
      createShader: () => ({}),
      shaderSource: (_s: unknown, src: string) => void rec.sources.push(src),
      compileShader: () => undefined,
      getShaderParameter: () => true,
      createProgram: () => ({}),
      attachShader: () => undefined,
      linkProgram: () => undefined,
      getProgramParameter: () => true,
      useProgram: () => undefined,
      createTexture: () => ({}),
      bindTexture: () => undefined,
      texParameteri: () => undefined,
      pixelStorei: (pname: number, value: number) => {
        if (pname === CONSTANTS.UNPACK_ROW_LENGTH) rowLength = value
      },
      viewport: (_x: number, _y: number, w: number, h: number) => void rec.calls.push(`viewport:${w}x${h}`),
      texImage2D: (...a: unknown[]) => void rec.calls.push(`texImage2D:${String(a[3])}x${String(a[4])}`),
      texSubImage2D: (...a: unknown[]) => {
        const [, , , , w, h] = a as [number, number, number, number, number, number]
        const data = a[8] as Uint8Array
        rec.calls.push(`texSubImage2D:${w}x${h}`)
        rec.uploads.push({ w, h, rowLength, bytes: data.byteLength, canvas: { w: canvas.width, h: canvas.height } })
      },
      drawArrays: () => {
        rec.calls.push("draw")
        rec.draws.push({ w: canvas.width, h: canvas.height })
      },
    }
    contexts.set(this, gl)
    return gl as never
  } as never)
  return rec
}

/** rAF·cancelAnimationFrame 을 명시적 큐로 함께 스텁한다 — 콜백은 `flush()` 로만 실행된다. 프레임 시계 의존 없이 결정적이다 */
export function stubRafQueue() {
  let next = 1
  const queue = new Map<number, FrameRequestCallback>()
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    queue.set(next, cb)
    return next++
  })
  vi.stubGlobal("cancelAnimationFrame", (id: number) => void queue.delete(id))
  return {
    pending: () => queue.size,
    /** 대기 중인 콜백을 모두 실행한다 (실행 중 예약된 것은 다음 flush 로) */
    flush: () => {
      const callbacks = [...queue.values()]
      queue.clear()
      for (const cb of callbacks) cb(0)
    },
  }
}

/** ResizeObserver 스텁 — 관찰 대상별로 콜백을 직접 호출한다. `trigger(el)` 는 `el` 을 관찰하는 모든 관찰자를 호출한다 */
export function stubResizeObserver() {
  const entries: { callback: ResizeObserverCallback; observed: Set<Element> }[] = []
  class FakeResizeObserver {
    private readonly entry: { callback: ResizeObserverCallback; observed: Set<Element> }
    constructor(callback: ResizeObserverCallback) {
      this.entry = { callback, observed: new Set() }
      entries.push(this.entry)
    }
    observe(el: Element): void {
      this.entry.observed.add(el)
    }
    unobserve(el: Element): void {
      this.entry.observed.delete(el)
    }
    disconnect(): void {
      this.entry.observed.clear()
    }
  }
  vi.stubGlobal("ResizeObserver", FakeResizeObserver)
  return {
    trigger: (el: Element) => {
      for (const e of entries) if (e.observed.has(el)) e.callback([], {} as ResizeObserver)
    },
    observing: (el: Element) => entries.some((e) => e.observed.has(el)),
  }
}
