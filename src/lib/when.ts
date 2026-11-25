// 플러그인 `when` 식 평가기 — Rust `crates/mam-plugin-manifest/src/when/` 와 같은 의미다.
// 공용 시험 벡터(`crates/mam-plugin-manifest/vectors/when.json`)로 결과 일치를 검증한다 (when.test.ts).

export type WhenErrorKind = "syntax" | "unknownIdentifier" | "typeMismatch" | "tooLong" | "tooDeep"

export class WhenError extends Error {
  constructor(readonly kind: WhenErrorKind) {
    super(`invalid when expression (${kind})`)
  }
}

/** 컨텍스트 값 — 키가 없거나 null 이면 Null */
export type WhenValue = boolean | number | string | null
export type WhenContext = Readonly<Record<string, WhenValue | undefined>>

// manifest `limits` (MAX_WHEN_LEN: UTF-8 바이트 길이, MAX_WHEN_DEPTH)
const MAX_WHEN_LEN = 256
const MAX_WHEN_DEPTH = 16

type IdentType = "number" | "bool" | readonly string[]
/** 닫힌 식별자 표 — Rust `IDENTIFIERS` 및 벡터 파일 `identifiers` 와 같아야 한다 */
const IDENTIFIERS: Readonly<Record<string, IdentType>> = {
  "selection.count": "number",
  "asset.kind": ["image", "video", "audio"],
  "asset.offline": "bool",
}

type CmpOp = "==" | "!=" | "<" | "<=" | ">" | ">="
type Tok =
  | { t: "ident"; v: string }
  | { t: "num"; v: number }
  | { t: "str"; v: string }
  | { t: "bool"; v: boolean }
  | { t: "not" | "and" | "or" | "lp" | "rp" }
  | { t: "cmp"; v: CmpOp }

type Expr =
  | { k: "lit"; v: boolean | number | string }
  | { k: "ident"; name: string; type: IdentType }
  | { k: "not"; e: Expr }
  | { k: "and" | "or"; a: Expr; b: Expr }
  | { k: "cmp"; op: CmpOp; a: Expr; b: Expr }

const isIdentStart = (c: string) => /^[A-Za-z_]$/.test(c)
const isIdentChar = (c: string) => /^[A-Za-z0-9_]$/.test(c)
const isDigit = (c: string | undefined) => c !== undefined && c >= "0" && c <= "9"
// Rust `char::is_control` — Unicode Cc
const isControl = (c: string) => {
  const n = c.codePointAt(0) ?? 0
  return n < 0x20 || (n >= 0x7f && n <= 0x9f)
}

function lex(src: string): Tok[] {
  const out: Tok[] = []
  const chars = [...src]
  let i = 0
  const at = (j: number): string | undefined => chars[j]
  while (i < chars.length) {
    const c = chars[i]
    if (c === " " || c === "\t" || c === "\r" || c === "\n") {
      i++
    } else if (c === "(") {
      out.push({ t: "lp" })
      i++
    } else if (c === ")") {
      out.push({ t: "rp" })
      i++
    } else if (c === "!") {
      if (at(i + 1) === "=") {
        out.push({ t: "cmp", v: "!=" })
        i += 2
      } else {
        out.push({ t: "not" })
        i++
      }
    } else if (c === "=") {
      if (at(i + 1) !== "=") throw new WhenError("syntax")
      out.push({ t: "cmp", v: "==" })
      i += 2
    } else if (c === "<" || c === ">") {
      const eq = at(i + 1) === "="
      out.push({ t: "cmp", v: `${c}${eq ? "=" : ""}` as CmpOp })
      i += eq ? 2 : 1
    } else if (c === "&" || c === "|") {
      if (at(i + 1) !== c) throw new WhenError("syntax")
      out.push({ t: c === "&" ? "and" : "or" })
      i += 2
    } else if (c === "'" || c === '"') {
      let s = ""
      let j = i + 1
      for (;;) {
        const d = at(j)
        if (d === undefined || isControl(d)) throw new WhenError("syntax")
        if (d === c) break
        if (d === "\\") {
          const e = at(j + 1)
          if (e !== "\\" && e !== "'" && e !== '"') throw new WhenError("syntax")
          s += e
          j += 2
        } else {
          s += d
          j++
        }
      }
      out.push({ t: "str", v: s })
      i = j + 1
    } else if (c === "-" || isDigit(c)) {
      let j = i
      if (chars[j] === "-") j++
      if (!isDigit(at(j))) throw new WhenError("syntax")
      if (chars[j] === "0") {
        j++
      } else {
        while (isDigit(at(j))) j++
      }
      if (at(j) === ".") {
        j++
        if (!isDigit(at(j))) throw new WhenError("syntax")
        while (isDigit(at(j))) j++
      }
      const next = at(j)
      if (next !== undefined && (isIdentChar(next) || next === ".")) throw new WhenError("syntax")
      out.push({ t: "num", v: Number(chars.slice(i, j).join("")) })
      i = j
    } else if (isIdentStart(c)) {
      let j = i
      for (;;) {
        while (at(j) !== undefined && isIdentChar(chars[j])) j++
        if (at(j) !== ".") break
        j++
        const n = at(j)
        if (n === undefined || !isIdentStart(n)) throw new WhenError("syntax")
      }
      const word = chars.slice(i, j).join("")
      out.push(word === "true" || word === "false" ? { t: "bool", v: word === "true" } : { t: "ident", v: word })
      i = j
    } else {
      throw new WhenError("syntax")
    }
  }
  return out
}

type Ty = "bool" | "num" | "str" | readonly string[]
interface Node {
  e: Expr
  ty: Ty
}

const sameTy = (a: Ty, b: Ty) => a === b // enum 은 닫힌 표의 같은 배열 참조일 때만 같다

function litIn(e: Expr, values: readonly string[]): boolean {
  return e.k === "lit" && typeof e.v === "string" && values.includes(e.v)
}

function checkCmp(op: CmpOp, l: Node, r: Node): void {
  let ok: boolean
  if (op === "==" || op === "!=") {
    if (sameTy(l.ty, r.ty)) ok = true
    else if (typeof l.ty !== "string" && r.ty === "str") ok = litIn(r.e, l.ty)
    else if (l.ty === "str" && typeof r.ty !== "string") ok = litIn(l.e, r.ty)
    else ok = false
  } else {
    ok = l.ty === "num" && r.ty === "num"
  }
  if (!ok) throw new WhenError("typeMismatch")
}

function needBool(n: Node): void {
  if (n.ty !== "bool") throw new WhenError("typeMismatch")
}

function parse(toks: Tok[]): Expr {
  let pos = 0
  let depth = 0
  const peek = (): Tok | undefined => toks[pos]
  const enter = () => {
    depth++
    if (depth > MAX_WHEN_DEPTH) throw new WhenError("tooDeep")
  }

  const primary = (): Node => {
    const tok = toks[pos]
    if (tok === undefined) throw new WhenError("syntax")
    pos++
    switch (tok.t) {
      case "bool":
        return { e: { k: "lit", v: tok.v }, ty: "bool" }
      case "num":
        return { e: { k: "lit", v: tok.v }, ty: "num" }
      case "str":
        return { e: { k: "lit", v: tok.v }, ty: "str" }
      case "ident": {
        const type = Object.prototype.hasOwnProperty.call(IDENTIFIERS, tok.v) ? IDENTIFIERS[tok.v] : undefined
        if (type === undefined) throw new WhenError("unknownIdentifier")
        return { e: { k: "ident", name: tok.v, type }, ty: type === "number" ? "num" : type === "bool" ? "bool" : type }
      }
      case "lp": {
        enter()
        const inner = or()
        if (peek()?.t !== "rp") throw new WhenError("syntax")
        pos++
        depth--
        return inner
      }
      default:
        throw new WhenError("syntax")
    }
  }

  const unary = (): Node => {
    if (peek()?.t === "not") {
      pos++
      enter()
      const inner = unary()
      depth--
      needBool(inner)
      return { e: { k: "not", e: inner.e }, ty: "bool" }
    }
    return primary()
  }

  const cmp = (): Node => {
    const left = unary()
    const tok = peek()
    if (tok?.t !== "cmp") return left
    pos++
    const right = unary()
    checkCmp(tok.v, left, right)
    if (peek()?.t === "cmp") throw new WhenError("syntax")
    return { e: { k: "cmp", op: tok.v, a: left.e, b: right.e }, ty: "bool" }
  }

  const and = (): Node => {
    let left = cmp()
    while (peek()?.t === "and") {
      pos++
      needBool(left)
      const right = cmp()
      needBool(right)
      left = { e: { k: "and", a: left.e, b: right.e }, ty: "bool" }
    }
    return left
  }

  function or(): Node {
    let left = and()
    while (peek()?.t === "or") {
      pos++
      needBool(left)
      const right = and()
      needBool(right)
      left = { e: { k: "or", a: left.e, b: right.e }, ty: "bool" }
    }
    return left
  }

  const node = or()
  if (pos < toks.length) throw new WhenError("syntax")
  needBool(node)
  return node.e
}

/** 식을 정적 검사까지 마쳐 파싱한다 — 잘못된 식은 `WhenError` */
export function parseWhen(src: string): Expr {
  if (new TextEncoder().encode(src).length > MAX_WHEN_LEN) throw new WhenError("tooLong")
  return parse(lex(src))
}

function typeOk(t: IdentType, v: WhenValue): boolean {
  if (t === "number") return typeof v === "number"
  if (t === "bool") return typeof v === "boolean"
  return typeof v === "string"
}

function evalExpr(e: Expr, ctx: WhenContext): WhenValue {
  switch (e.k) {
    case "lit":
      return e.v
    case "ident": {
      const v = ctx[e.name]
      if (v === undefined || v === null) return null
      if (!typeOk(e.type, v)) throw new WhenError("typeMismatch")
      return v
    }
    case "not":
      return evalExpr(e.e, ctx) !== true
    case "and":
      return evalExpr(e.a, ctx) === true && evalExpr(e.b, ctx) === true
    case "or":
      return evalExpr(e.a, ctx) === true || evalExpr(e.b, ctx) === true
    case "cmp": {
      const l = evalExpr(e.a, ctx)
      const r = evalExpr(e.b, ctx)
      if (e.op === "==") return l === r
      if (e.op === "!=") return l !== r
      if (typeof l !== "number" || typeof r !== "number") return false
      if (e.op === "<") return l < r
      if (e.op === "<=") return l <= r
      return e.op === ">" ? l > r : l >= r
    }
  }
}

/** 파싱 + 실행. 파싱·실행 오류는 `WhenError` 로 던진다 (시험용) */
export function tryEvalWhen(src: string, ctx: WhenContext): boolean {
  return evalExpr(parseWhen(src), ctx) === true
}

/** 메뉴용 — 어떤 오류든 false(숨김), fail-closed. `src` 가 null 이면 조건 없음(true) */
export function evalWhen(src: string | null, ctx: WhenContext): boolean {
  if (src === null) return true
  try {
    return tryEvalWhen(src, ctx)
  } catch {
    return false
  }
}
