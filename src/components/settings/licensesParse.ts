/** `THIRD_PARTY_LICENSES.md` 형식 계약(생성 스크립트 ↔ 설정 화면)의 파서 — 형식이 어긋나도 던지지 않는다. */

export interface LicenseSection {
  /** `## ` 뒤 제목 전체 (예: `MIT License (MIT) — 612 packages`) */
  title: string
  /** 제목 뒤 본문(마크다운 원문) */
  body: string
}

export interface ParsedLicenses {
  /** 머리말 주석의 `packages: <n>` — 없으면 null */
  packageCount: number | null
  sections: LicenseSection[]
}

/** 한 줄을 본 뒤의 펜스 상태 — 여는 줄은 정보 문자열(백틱 없음)을 허용하고, 닫는 줄은 백틱만 허용한다. */
function nextFence(line: string, open: number): number {
  if (open === 0) {
    const m = /^ {0,3}(`{3,})[^`]*$/.exec(line)
    return m ? m[1].length : 0
  }
  const c = /^ {0,3}(`{3,})[ \t]*$/.exec(line)
  return c && c[1].length >= open ? 0 : open
}

const META_RE = /<!--[^>]*?\bpackages:\s*(\d+)[^>]*?-->/

export function parseLicenses(source: string): ParsedLicenses {
  const text = source.replace(/\r\n/g, "\n")
  const countMatch = META_RE.exec(text)
  const sections: LicenseSection[] = []
  let current: { title: string; lines: string[] } | null = null
  let fenceLen = 0 // 열린 코드 펜스의 백틱 수 (0 = 밖). CommonMark: 같거나 더 긴 백틱 줄로만 닫힌다
  for (const line of text.split("\n")) {
    fenceLen = nextFence(line, fenceLen)
    if (fenceLen === 0 && /^## /.test(line)) {
      if (current) sections.push({ title: current.title, body: current.lines.join("\n").trim() })
      current = { title: line.slice(3).trim(), lines: [] }
    } else if (current) {
      current.lines.push(line)
    }
  }
  if (current) sections.push({ title: current.title, body: current.lines.join("\n").trim() })
  return { packageCount: countMatch ? Number(countMatch[1]) : null, sections }
}
