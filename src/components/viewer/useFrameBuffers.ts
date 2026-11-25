import { useEffect, useState } from "react"
import { getChromeWebview, type ChromeWebview, type SharedBufferEvent, type WebMessageEvent } from "@/lib/chromeWebview"
import { useApi } from "@/lib/api"
import { isBufferInfo, isFrameMsg, type ReleaseMsg } from "@/lib/ipc-player-wire"
import { FrameQueue, groupReleases, STAGING_TIMEOUT_MS, type DrawJob } from "./frameQueue"

export type FrameJob = DrawJob<ArrayBuffer>

/** 렌더러가 보는 프레임 공급원 — rAF 마다 `takeDraw` 로 1장을 받고, 업로드가 끝나면 `drawn` 을 부른다 */
export interface FrameSource {
  takeDraw(): FrameJob | null
  drawn(job: FrameJob): void
  /** 그릴 프레임이 생겼을 수 있을 때 알린다 (rAF 예약용). 해제 함수를 돌려준다 */
  subscribe(listener: () => void): () => void
}

export interface FrameSize {
  w: number
  h: number
}

interface Options {
  /** 테스트용 주입 — 기본은 `window.chrome.webview` */
  webview?: ChromeWebview | null
}

/**
 * `chrome.webview` 어댑터 — SharedBuffer(에포크당 3개)·프레임 메시지를 `FrameQueue` 에 먹이고, 그 결과(release·`releaseBuffer`)를 실행한다.
 * - release 는 주입된 `sendRelease`(= `api.playerReleaseSlots`, Tauri command)로 보낸다 (microtask 마다 에포크당 1메시지, fire-and-forget).
 *   `chrome.webview.postMessage`·`window.ipc.postMessage` 는 쓰지 않는다 — 객체 메시지는 wry 처리기가 거부해 호스트에 닿지 않는다.
 * - 새 에포크의 첫 프레임이 2초 안에 안 오면 스테이징 버퍼를 놓고 다음 에포크를 기다린다.
 */
class FrameBuffers implements FrameSource {
  private queue = new FrameQueue<ArrayBuffer>()
  private webview: ChromeWebview | null = null
  private readonly listeners = new Set<() => void>()
  private flushScheduled = false
  private stagingTimer: ReturnType<typeof setTimeout> | null = null
  private watchedStaging = 0
  private rendering = true
  private lastSize: FrameSize | null = null
  onFrameSize: (size: FrameSize) => void = () => undefined
  /** release 전송 — 훅이 `api.playerReleaseSlots` 로 채운다. 기다리지 않고 실패는 무시한다 */
  sendRelease: (msg: ReleaseMsg) => void = () => undefined

  attach(webview: ChromeWebview): () => void {
    this.queue = new FrameQueue<ArrayBuffer>()
    this.queue.setRendering(this.rendering)
    this.webview = webview
    this.lastSize = null
    const onBuffer = (e: SharedBufferEvent) => this.receiveBuffer(e)
    const onMessage = (e: WebMessageEvent) => this.receiveMessage(e)
    webview.addEventListener("sharedbufferreceived", onBuffer)
    webview.addEventListener("message", onMessage)
    return () => {
      webview.removeEventListener("sharedbufferreceived", onBuffer)
      webview.removeEventListener("message", onMessage)
      this.queue.dispose()
      this.applyEffects(false)
      this.webview = null
      this.clearTimer()
    }
  }

  readonly setRendering = (rendering: boolean): void => {
    this.rendering = rendering
    this.queue.setRendering(rendering)
    this.applyEffects()
  }

  takeDraw(): FrameJob | null {
    return this.queue.takeDraw()
  }

  drawn(job: FrameJob): void {
    this.queue.drawn(job)
    this.applyEffects()
    const { w, h } = job.frame
    if (this.lastSize?.w !== w || this.lastSize?.h !== h) {
      this.lastSize = { w, h }
      this.onFrameSize(this.lastSize)
    }
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => void this.listeners.delete(listener)
  }

  private receiveBuffer(e: SharedBufferEvent): void {
    let buffer: ArrayBuffer
    try {
      buffer = e.getBuffer()
    } catch {
      return
    }
    const info = e.additionalData
    // 모양이 틀렸거나 info.size 보다 작은 버퍼는 쓰지 않고 바로 놓는다 (그리기가 범위를 넘어 읽지 않게)
    if (!isBufferInfo(info) || buffer.byteLength < info.size) {
      this.webview?.releaseBuffer(buffer)
      return
    }
    this.queue.onBuffer(info, buffer)
    this.applyEffects()
  }

  private receiveMessage(e: WebMessageEvent): void {
    if (!isFrameMsg(e.data)) return
    this.queue.onFrame(e.data)
    this.applyEffects()
  }

  /** 큐가 계산한 부수 효과를 실행한다 — 놓을 버퍼, release 예약, 스테이징 타이머, 그리기 알림 */
  private applyEffects(notify = true): void {
    const webview = this.webview
    for (const buffer of this.queue.drainFreed()) {
      try {
        webview?.releaseBuffer(buffer)
      } catch {
        // 이미 놓였거나 WebView 가 닫히는 중 — 더 할 일 없음
      }
    }
    if (webview && !this.flushScheduled) {
      this.flushScheduled = true
      queueMicrotask(() => this.flushReleases())
    }
    this.watchStaging()
    if (notify && this.queue.hasPending()) for (const l of [...this.listeners]) l()
  }

  private flushReleases(): void {
    this.flushScheduled = false
    const releases = this.queue.drainReleases()
    if (!this.webview) return
    for (const msg of groupReleases(releases)) {
      try {
        this.sendRelease(msg)
      } catch {
        // 호스트가 받지 못해도 워커의 슬롯 점유 감시가 처리한다
      }
    }
  }

  private watchStaging(): void {
    const staging = this.queue.stagingEpoch
    if (staging === this.watchedStaging) return
    this.clearTimer()
    this.watchedStaging = staging // clearTimer 가 0 으로 되돌리므로 그 뒤에 기록한다 (아니면 이벤트마다 타이머가 다시 걸려 만료가 계속 밀린다)
    if (staging === 0) return
    this.stagingTimer = setTimeout(() => {
      this.stagingTimer = null
      this.queue.expireStaging(staging)
      this.applyEffects()
    }, STAGING_TIMEOUT_MS)
  }

  private clearTimer(): void {
    if (this.stagingTimer !== null) clearTimeout(this.stagingTimer)
    this.stagingTimer = null
    this.watchedStaging = 0
  }
}

/**
 * 프레임 수신·반환. `setRendering(false)`(숨김·렌더 비활성)이면 도착하는 프레임은 그리지 않고 즉시 돌려준다 — rAF 가 멈추는지에 기대지 않는다.
 * `frameSize` 는 마지막으로 그린 프레임의 크기 — 무대의 영상 비율을 정한다. 호출 순서: 이 훅은 세션을 여는 훅보다 먼저 호출해
 * 첫 SharedBuffer 가 오기 전에 리스너가 설치돼 있어야 한다 (`setRendering` 이 별도인 이유 — 렌더 여부는 세션 뒤에 정해진다).
 */
export function useFrameBuffers(options: Options = {}): { source: FrameSource; frameSize: FrameSize | null; setRendering: (rendering: boolean) => void } {
  const api = useApi()
  const [frameSize, setFrameSize] = useState<FrameSize | null>(null)
  const [controller] = useState(() => new FrameBuffers())
  controller.onFrameSize = setFrameSize
  controller.sendRelease = (msg) => void api.playerReleaseSlots(msg).catch(() => undefined)
  const webview = options.webview === undefined ? getChromeWebview() : options.webview
  useEffect(() => {
    if (!webview) return
    return controller.attach(webview)
  }, [controller, webview])
  return { source: controller, frameSize, setRendering: controller.setRendering }
}
