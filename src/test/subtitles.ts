import { vi } from "vitest"
import type { MockOptions } from "@/lib/api/mock"
import type { SubtitleCue, SubtitleFormat, SubtitleTrack } from "@/lib/ipc-contract"

/** 자산별 자막 mock 옵션의 한 항목 */
export type SubEntry = NonNullable<MockOptions["subtitles"]>[number][number]

export function subTrack(id: string, language: string | null, format: SubtitleFormat = "srt", fileName: string = id): SubtitleTrack {
  return { id, language, format, fileName }
}

export function subEntry(track: SubtitleTrack, cues: SubtitleCue[] = [{ startMs: 0, endMs: 1000, text: track.id }]): SubEntry {
  return { track, cues }
}

/** jsdom 에는 VTTCue·addTextTrack 이 없다 — 결정적인 가짜로 대체한다 */
export class FakeCue {
  constructor(
    public startTime: number,
    public endTime: number,
    public text: string
  ) {}
}

export class FakeTextTrack {
  mode: "disabled" | "hidden" | "showing" = "hidden"
  cues: FakeCue[] = []
  constructor(
    public owner: HTMLMediaElement,
    public kind: string
  ) {}
  addCue(cue: FakeCue) {
    this.cues.push(cue)
  }
  removeCue(cue: FakeCue) {
    this.cues = this.cues.filter((c) => c !== cue)
  }
}

/**
 * `VTTCue` 전역과 `HTMLMediaElement.prototype.addTextTrack` 를 스텁한다. `tracks` 는 만들어진 텍스트 트랙 전부(요소당 1개여야 한다).
 * 반환된 `restore` 를 afterEach 에서 부른다 (VTTCue 전역은 `vi.unstubAllGlobals()` 가 되돌린다).
 */
export function stubTextTracks() {
  const tracks: FakeTextTrack[] = []
  vi.stubGlobal("VTTCue", FakeCue)
  const proto = HTMLMediaElement.prototype as unknown as { addTextTrack?: unknown }
  const original = proto.addTextTrack
  const addTextTrack = vi.fn(function (this: HTMLMediaElement, kind: string) {
    const track = new FakeTextTrack(this, kind)
    tracks.push(track)
    return track
  })
  proto.addTextTrack = addTextTrack
  return {
    tracks,
    addTextTrack,
    restore: () => {
      if (original === undefined) delete proto.addTextTrack
      else proto.addTextTrack = original
    },
  }
}
