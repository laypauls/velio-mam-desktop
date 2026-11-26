// 분류: Happy(모든 Rename 뒤 알림 존재) / Boundary(Rename 0건 가드) / Validation(알림 누락 거부)
// 회귀: 설치기가 velio.lnk 생성 직후 velio..lnk 로 Rename 하면 Explorer 바탕화면이 변경을 놓쳐 F5 전까지 안 보임 —
//       Rename 직후 SHChangeNotify(SHCNE_RENAMEITEM) 로 알려야 한다.
import { describe, expect, it } from "vitest"
import hooksRaw from "../../src-tauri/windows/hooks.nsh?raw"

const hooks = hooksRaw.replace(/\r\n/g, "\n")

const NOTIFY_CALL = /^\s*!insertmacro VELIO_NOTIFY_RENAME\s+\S+\s+\S+/

function codeLines(text: string): string[] {
  return text.split("\n").filter((l) => !l.trim().startsWith(";"))
}

/** Rename 명령 개수와, 바로 다음 줄이 알림 호출이 아닌 Rename 의 줄 번호(0 기반, 코드 줄 기준). */
function scanRenames(text: string): { total: number; unnotified: number[] } {
  const lines = codeLines(text)
  const unnotified: number[] = []
  let total = 0
  lines.forEach((l, i) => {
    if (/^\s*Rename\s/.test(l)) {
      total++
      if (!NOTIFY_CALL.test(lines[i + 1] ?? "")) unnotified.push(i)
    }
  })
  return { total, unnotified }
}

describe("hooks.nsh Rename 후 셸 변경 알림", () => {
  it("T-ui-nsis-01 Rename 이 1건 이상 존재한다 (가드)", () => {
    expect(scanRenames(hooks).total).toBeGreaterThanOrEqual(1)
  })

  it("T-ui-nsis-02 모든 Rename 바로 다음 줄이 VELIO_NOTIFY_RENAME 호출이다", () => {
    expect(scanRenames(hooks).unnotified).toEqual([])
  })

  it("T-ui-nsis-03 알림 매크로가 SHCNE_RENAMEITEM(0x1) + PATHW|FLUSH(0x1005) 로 SHChangeNotify 를 호출한다", () => {
    expect(hooks).toMatch(
      /!macro VELIO_NOTIFY_RENAME[^\n]*\n\s*System::Call 'shell32::SHChangeNotify\(i 0x1, i 0x1005, w "\$\{old\}", w "\$\{new\}"\)'/,
    )
  })

  it("T-ui-nsis-04 알림이 빠진 Rename 은 거부된다", () => {
    const bad = 'Rename "a.lnk" "b.lnk"\n  Delete "x"\n'
    expect(scanRenames(bad).unnotified).toEqual([0])
    const good = 'Rename "a.lnk" "b.lnk"\n  !insertmacro VELIO_NOTIFY_RENAME "a.lnk" "b.lnk"\n'
    expect(scanRenames(good).unnotified).toEqual([])
    expect(scanRenames("; Rename only in comment\n").total).toBe(0)
  })
})
