// Categories: Business rule (T-ui-detect-13 registry table, 14 dynamic i18n families) / Happy + Validation (T-ui-detect-mock-01 mock parity)
import { describe, expect, it } from "vitest"
import ko from "@/i18n/locales/ko.json"
import { createMockApi } from "@/lib/api/mock"
import { KNOWN_TRIGGER_KINDS } from "@/lib/ipc-contract"
import { WEEKDAYS } from "@/lib/rescan-policy"
import { TRIGGER_FIELDS } from "./trigger-fields"

/** Rust 소스 원문 (정적 검사용) — CRLF → LF 정규화. 상대 경로여야 한다 (`/src-tauri/` 절대 glob 은 빈 결과). */
const RUST: Record<string, string> = Object.fromEntries(
  Object.entries(
    import.meta.glob<string>(["../../../../crates/mam-core/src/watch/**/*.rs", "!../../../../crates/mam-core/src/watch/**/*_tests.rs"], {
      query: "?raw",
      import: "default",
      eager: true,
    }),
  ).map(([path, text]) => [path.replace(/^.*\/watch\//, ""), text.replace(/\r\n/g, "\n")]),
)

const has = (key: string): boolean => {
  let node: unknown = ko
  for (const part of key.split(".")) {
    if (typeof node !== "object" || node === null || !(part in node)) return false
    node = (node as Record<string, unknown>)[part]
  }
  return typeof node === "string"
}

describe("change detection contract", () => {
  it("T-ui-detect-13 [Business rule] TRIGGER_FIELDS has exactly one entry per known trigger kind", () => {
    // Assert
    expect(KNOWN_TRIGGER_KINDS.length).toBeGreaterThanOrEqual(3)
    expect(Object.keys(TRIGGER_FIELDS).sort()).toEqual([...KNOWN_TRIGGER_KINDS].sort())
    expect(TRIGGER_FIELDS.manual).toBeNull()
    expect(TRIGGER_FIELDS.interval).not.toBeNull()
    expect(TRIGGER_FIELDS.schedule).not.toBeNull()
  })

  it("T-ui-detect-14 [Validation] dynamic key families exist: watch.detector.<id>.{name,description}, watch.reason.<key>, watch.weekday.<d>, trigger labels", async () => {
    // Arrange: 목 레지스트리의 감지기 + Rust 가 내는 폴백 사유 키
    const api = createMockApi({ count: 5 })
    const ids = (await api.listChangeDetectors(1)).map((d) => d.id)
    const reasons = ["start_failed", "unknown_mode", "invalid_options"]
    // Act
    const missing = [
      ...ids.flatMap((id) => [`watch.detector.${id}.name`, `watch.detector.${id}.description`]),
      ...reasons.map((r) => `watch.reason.${r}`),
      ...WEEKDAYS.map((d) => `watch.weekday.${d}`),
      "settings.folders.detection.policyInterval",
      "settings.folders.detection.policySchedule",
      "settings.folders.detection.policyManual",
    ].filter((k) => !has(k))
    // Assert
    expect(ids).toEqual(["os_events", "polling", "ddp_audit", "fanotify_feed", "scan_only"])
    expect(missing).toEqual([])
  })

  it("T-ui-detect-15 [Validation] every key the Rust watch code emits exists in ko.json (health, fallback/unavailable reasons, remote error keys)", () => {
    // Arrange: 스캐너가 소스를 못 읽은 회귀도 잡는다
    expect(Object.keys(RUST).length).toBeGreaterThanOrEqual(8)
    const all = Object.values(RUST)
    // Act
    const health = new Set(all.flatMap((t) => [...t.matchAll(/"(watch\.health\.[a-z_]+)"/g)].map((m) => m[1])))
    const reasons = new Set(all.flatMap((t) => [...t.matchAll(/const REASON_[A-Z_]+: &str = "([a-z_]+)"/g)].map((m) => `watch.reason.${m[1]}`)))
    const runnerKeys = /pub fn key\(&self\) -> &'static str \{([\s\S]*?)\n    \}/.exec(RUST["remote/runner.rs"])?.[1] ?? ""
    const remoteErrors = [...runnerKeys.matchAll(/=> "([a-z_]+)"/g)].map((m) => `remote.error.${m[1]}`)
    // Assert: 알려진 최소 집합이 실제로 잡혔고, 전부 번역이 있다
    expect([...health]).toEqual(expect.arrayContaining(["watch.health.overflow", "watch.health.error", "watch.health.remote_connect", "watch.health.remote_db_unreadable", "watch.health.poll_timeout"]))
    expect([...reasons]).toEqual(expect.arrayContaining(["watch.reason.remote_not_ready", "watch.reason.remote_path_unmapped"]))
    expect(remoteErrors.length).toBe(12)
    const missing = [...health, ...reasons, ...remoteErrors].filter((k) => !has(k))
    expect(missing).toEqual([])
  })

  it("T-ui-detect-16 [Validation] every detector option label and the detector option kinds are translated; unknown option kinds fail loudly", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    const detectors = await api.listChangeDetectors(1)
    // Act
    const labels = detectors.flatMap((d) => d.optionFields.filter((f) => f.kind !== "bool").map((f) => `watch.detector.${d.id}.option.${f.key}`))
    const kinds = new Set(detectors.flatMap((d) => d.optionFields.map((f) => f.kind)))
    // Assert
    expect(labels.length).toBeGreaterThanOrEqual(3)
    expect(labels.filter((k) => !has(k))).toEqual([])
    expect([...kinds].sort()).toEqual(["integer", "ref"])
    expect(detectors.filter((d) => d.experimental).map((d) => d.id)).toEqual(["polling", "ddp_audit", "fanotify_feed"])
  })

  it("T-ui-detect-mock-01 [Happy] the mock mirrors server rules: normalization, null reset, field-path errors, active/fallback fields", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    // Act: 정규화 + 저장
    const saved = await api.setFolderRescanPolicy(1, {
      trigger: { kind: "schedule", days: ["sun", "mon", "mon"], at: "03:00" },
      window: null,
      onStart: false,
    })
    // Assert
    expect(saved.rescanPolicy).toEqual({ trigger: { kind: "schedule", days: ["mon", "sun"], at: "03:00" }, window: null, onStart: false })
    expect(saved.nextRescanAt).not.toBeNull()
    expect((await api.setFolderRescanPolicy(1, null)).rescanPolicy).toBeNull()
    // Validation: 서버와 같은 필드 경로 메시지
    await expect(api.setFolderRescanPolicy(1, { trigger: { kind: "interval", hours: 0 }, window: null, onStart: false })).rejects.toMatchObject({
      code: "invalid_input",
      message: "invalid input: rescanPolicy.trigger.hours",
    })
    await expect(api.setFolderRescanPolicy(99, null)).rejects.toMatchObject({ code: "not_found" })
    // 방식
    expect((await api.listFolders()).find((f) => f.folderId === 1)).toMatchObject({ watchMode: null, watchActive: "os_events", watchFallbackKey: null })
    expect(await api.setFolderWatchMode(1, "scan_only", null)).toMatchObject({ watchMode: "scan_only", watchActive: "scan_only" })
    await expect(api.setFolderWatchMode(1, "nope", null)).rejects.toMatchObject({ code: "invalid_input", message: "invalid input: watchMode" })
    await expect(api.setFolderWatchMode(1, "os_events", { x: 1 })).rejects.toMatchObject({ message: "invalid input: watchOptions" })
    // 자동은 공통 옵션만 받는다 (감지기 전용 옵션은 거부)
    await expect(api.setFolderWatchMode(1, null, { intervalSec: 60 })).rejects.toMatchObject({ message: "invalid input: watchOptions" })
    expect(await api.setFolderWatchMode(1, null, { exclusiveOpenCheck: false })).toMatchObject({ watchMode: null, watchOptions: { exclusiveOpenCheck: false } })
    // 오프라인 폴더는 감시 중이 아니다 (시드: NAS 폴더는 오프라인 + 폴백 상태 보유)
    expect((await api.listFolders()).find((f) => f.folderId === 3)).toMatchObject({ watchActive: null, watchFallbackKey: null, nextRescanAt: null })
  })
})
