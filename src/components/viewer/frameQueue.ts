import { MAX_RELEASE_SLOTS, SLOT_COUNT, PLAYER_WIRE, type BufferInfo, type FrameMsg, type ReleaseMsg } from "@/lib/ipc-player-wire"

/**
 * 프레임 대기·교체·반환 계획 (순수) — 계약 §3.5~3.7. DOM·WebView2 를 모른다: 버퍼는 불투명한 `B`(실제로는 ArrayBuffer)이고,
 * 이 클래스는 "무엇을 그리고, 무엇을 release 하고, 어떤 버퍼를 `releaseBuffer` 해야 하는가"만 계산한다.
 * 어댑터(`useFrameBuffers`)가 `drainReleases()` / `drainFreed()` 로 부수 효과를 꺼내 실행한다.
 *
 * 규칙 요약
 * - 현재 에포크 프레임: 슬롯 버퍼가 없거나 크기가 어긋나면 버리고(release), 최신 1장만 보관(이전 대기분은 release).
 * - 현재보다 큰 에포크(스테이징): 세 버퍼가 모두 도착했을 때만 프레임을 붙잡아 두었다가, 그릴 때 한 번에 교체한다.
 * - 현재보다 작은 에포크 프레임: 무시(release 도 안 보낸다 — 호스트가 이미 버렸다).
 * - 그리지 않는 중(숨김·렌더 비활성): 프레임을 그리지 않고 즉시 release. 스테이징 교체는 그리지 않고 확정한다.
 */

/** 새 에포크의 첫 프레임이 이 시간 안에 오지 않으면 스테이징을 포기한다 */
export const STAGING_TIMEOUT_MS = 2000

export interface SlotRelease {
  epoch: number
  slot: number
  seq: number
}

export interface DrawJob<B> {
  frame: FrameMsg
  buffer: B
  info: BufferInfo
  /** true 면 이 프레임이 새 에포크의 첫 프레임 — 캔버스 크기·텍스처를 바꿔 그린 같은 작업에서 교체가 확정된다 (`drawn`) */
  swap: boolean
}

interface Bank<B> {
  info: BufferInfo
  buffers: (B | undefined)[]
  count: number
}

export class FrameQueue<B> {
  private current = 0
  private staging = 0
  private readonly banks = new Map<number, Bank<B>>()
  private pending: FrameMsg | null = null
  private stagedPending: FrameMsg | null = null
  private rendering = true
  private disposed = false
  private releases: SlotRelease[] = []
  private freed: B[] = []

  get currentEpoch(): number {
    return this.current
  }
  get stagingEpoch(): number {
    return this.staging
  }

  /** 아직 그려지지 않은 프레임이 있는가 (rAF 예약 판단용) */
  hasPending(): boolean {
    return this.rendering && (this.pending !== null || this.stagedPending !== null)
  }

  /** SharedBuffer 하나 수신. 같은 에포크의 세 버퍼가 모두 오면 스테이징(또는 첫 에포크) 준비가 끝난다 */
  onBuffer(info: BufferInfo, buffer: B): void {
    if (this.disposed || info.epoch < this.current) return this.free(buffer) // 늦게 도착한 옛 에포크
    if (info.epoch === this.current) return this.addTo(this.banks.get(info.epoch), info, buffer)
    if (info.epoch < this.staging) return this.free(buffer)
    if (info.epoch > this.staging) {
      this.abandonStaging()
      this.staging = info.epoch
      this.banks.set(info.epoch, { info, buffers: [], count: 0 })
    }
    this.addTo(this.banks.get(info.epoch), info, buffer)
  }

  /** 호스트의 프레임 메시지 하나 수신 */
  onFrame(msg: FrameMsg): void {
    if (this.disposed || msg.epoch < this.current) return
    if (msg.epoch === this.current) return this.onCurrentFrame(msg)
    const bank = msg.epoch === this.staging ? this.banks.get(msg.epoch) : undefined
    if (!bank || bank.count < SLOT_COUNT || !this.fits(bank, msg)) return this.release(msg)
    if (!this.rendering) {
      this.commitSwap(msg.epoch)
      return this.release(msg)
    }
    if (this.stagedPending) this.release(this.stagedPending) // 더 새 프레임이 왔으니 이전 대기분은 돌려준다
    this.stagedPending = msg
  }

  /** rAF 에서 호출 — 그릴 프레임 하나(스테이징 우선) 또는 null. 반환 뒤 반드시 `drawn` 을 부른다 */
  takeDraw(): DrawJob<B> | null {
    if (this.disposed || !this.rendering) return null
    if (this.stagedPending) {
      const frame = this.stagedPending
      const bank = this.banks.get(frame.epoch)
      this.stagedPending = null
      const buffer = bank?.buffers[frame.slot]
      if (bank && buffer !== undefined) return { frame, buffer, info: bank.info, swap: true }
      this.release(frame)
    }
    if (this.pending) {
      const frame = this.pending
      const bank = this.banks.get(frame.epoch)
      this.pending = null
      const buffer = bank?.buffers[frame.slot]
      if (bank && buffer !== undefined) return { frame, buffer, info: bank.info, swap: false }
      this.release(frame)
    }
    return null
  }

  /** 그리기(또는 실패)가 끝났다 — 프레임을 돌려주고, 새 에포크의 첫 프레임이었다면 옛 버퍼를 놓는다 */
  drawn(job: DrawJob<B>): void {
    if (this.disposed) return
    if (job.swap) this.commitSwap(job.frame.epoch)
    this.release(job.frame)
  }

  /** 렌더 활성 여부 변경. 끄면 대기 중인 프레임을 모두 돌려주고 스테이징 교체는 그리지 않고 확정한다 */
  setRendering(rendering: boolean): void {
    if (this.disposed || this.rendering === rendering) return
    this.rendering = rendering
    if (rendering) return
    if (this.pending) this.release(this.pending)
    this.pending = null
    const staged = this.stagedPending
    this.stagedPending = null
    if (staged) {
      this.commitSwap(staged.epoch)
      this.release(staged)
    }
  }

  /** 스테이징 타임아웃(`STAGING_TIMEOUT_MS`) — `epoch` 가 아직 스테이징 중이면 포기하고 다음 에포크를 기다린다. 포기했으면 true */
  expireStaging(epoch: number): boolean {
    if (this.staging === 0 || this.staging !== epoch) return false
    this.abandonStaging()
    return true
  }

  /** 언마운트·세션 종료 — 모든 버퍼를 놓고 대기 프레임을 버린다. 보낼 release 도 버린다(플레이어가 닫힌다) */
  dispose(): void {
    if (this.disposed) return
    for (const bank of this.banks.values()) this.freeBank(bank)
    this.banks.clear()
    this.pending = null
    this.stagedPending = null
    this.releases = []
    this.current = 0
    this.staging = 0
    this.disposed = true
  }

  drainReleases(): SlotRelease[] {
    const out = this.releases
    this.releases = []
    return out
  }

  drainFreed(): B[] {
    const out = this.freed
    this.freed = []
    return out
  }

  // ─── 내부 ──────────────────────────────────────────────────────────────

  private onCurrentFrame(msg: FrameMsg): void {
    const bank = this.banks.get(msg.epoch)
    if (!bank || !this.fits(bank, msg)) return this.release(msg)
    if (!this.rendering) return this.release(msg)
    if (this.pending) this.release(this.pending)
    this.pending = msg
  }

  /** 프레임이 이 뱅크의 버퍼에 안전하게 들어가는가 (버퍼 있음, stride 일치, 범위 안) */
  private fits(bank: Bank<B>, msg: FrameMsg): boolean {
    const { info } = bank
    return (
      bank.buffers[msg.slot] !== undefined &&
      msg.stride === info.stride &&
      msg.stride >= msg.w * 4 &&
      msg.w <= info.w &&
      msg.h <= info.h &&
      msg.stride * msg.h <= info.size
    )
  }

  private addTo(bank: Bank<B> | undefined, info: BufferInfo, buffer: B): void {
    const same = bank && bank.info.w === info.w && bank.info.h === info.h && bank.info.stride === info.stride && bank.info.size === info.size
    if (!bank || !same || bank.buffers[info.slot] !== undefined) return this.free(buffer)
    bank.buffers[info.slot] = buffer
    bank.count += 1
  }

  private commitSwap(epoch: number): void {
    if (epoch !== this.staging) return
    if (this.current !== 0) {
      const old = this.banks.get(this.current)
      if (old) this.freeBank(old)
      this.banks.delete(this.current)
    }
    this.current = epoch
    this.staging = 0
    this.pending = null // 옛 에포크 대기 프레임 — 호스트가 이미 버렸으므로 release 없이 버린다
  }

  private abandonStaging(): void {
    if (this.staging === 0) return
    const bank = this.banks.get(this.staging)
    if (bank) this.freeBank(bank)
    this.banks.delete(this.staging)
    if (this.stagedPending) this.release(this.stagedPending)
    this.stagedPending = null
    this.staging = 0
  }

  private freeBank(bank: Bank<B>): void {
    for (const b of bank.buffers) if (b !== undefined) this.free(b)
    bank.buffers = []
    bank.count = 0
  }

  private free(buffer: B): void {
    this.freed.push(buffer)
  }

  private release(msg: FrameMsg): void {
    this.releases.push({ epoch: msg.epoch, slot: msg.slot, seq: msg.seq })
  }
}

/** 대기 중인 release 를 에포크별 `ReleaseMsg`(슬롯 1..8 개)로 묶는다. 같은 (epoch, slot, seq) 중복은 하나로 */
export function groupReleases(list: readonly SlotRelease[]): ReleaseMsg[] {
  const byEpoch = new Map<number, { slot: number; seq: number }[]>()
  for (const r of list) {
    const slots = byEpoch.get(r.epoch) ?? []
    if (!slots.some((s) => s.slot === r.slot && s.seq === r.seq)) slots.push({ slot: r.slot, seq: r.seq })
    byEpoch.set(r.epoch, slots)
  }
  const out: ReleaseMsg[] = []
  for (const [epoch, slots] of byEpoch) {
    for (let i = 0; i < slots.length; i += MAX_RELEASE_SLOTS) {
      out.push({ mam: PLAYER_WIRE, k: "release", epoch, slots: slots.slice(i, i + MAX_RELEASE_SLOTS) })
    }
  }
  return out
}
