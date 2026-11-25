import type { PlayerApi } from "@/lib/ipc-contract-player"
import { cmdError } from "./mock-search"

/** 내장 플레이어 mock — 브라우저(dev:web)에는 libmpv 워커·WebView2 SharedBuffer 가 없으므로 항상 "사용 불가"로 보고한다. */
export function createMockPlayer(): PlayerApi {
  const unavailable = () => cmdError("player_unavailable", "built-in player is not available in the browser mock")
  return {
    playerAvailable: async () => false,
    playerOpen: async () => {
      throw unavailable()
    },
    playerSetRanges: async () => {},
    playerControl: async () => {},
    playerSeek: async () => {},
    playerSetRate: async () => {},
    playerSetVolume: async () => {},
    playerSetRenderSize: async () => {},
    playerSetRenderActive: async () => {},
    playerReleaseSlots: async () => {},
    playerRestart: async () => {
      throw unavailable()
    },
    systemVolumeStep: async () => {
      throw cmdError("audio_device_unavailable", "system volume is not available in the browser mock")
    },
    playerClose: async () => {},
    onPlayerState: async () => () => {},
  }
}
