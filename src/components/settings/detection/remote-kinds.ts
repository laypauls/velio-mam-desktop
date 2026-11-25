import type { RemoteSourceKind } from "@/lib/ipc-contract"

/**
 * 원격 접속(참조 옵션)을 쓰는 감지기 → 그 감지기가 받는 접속 종류.
 * 서버가 종류가 맞지 않는 접속을 거부하므로(`watch.reason.remote_source_kind`), 목록을 미리 걸러 보여 주기 위한 앞단 규칙이다.
 */
export const REMOTE_KIND_OF: Readonly<Record<string, RemoteSourceKind>> = {
  ddp_audit: "ddp_audit",
  fanotify_feed: "ses_queue",
}
