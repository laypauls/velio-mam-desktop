// SPDX expression parsing / allowlist evaluation (pure functions, no I/O).
import { ALLOWED, ALLOWED_EXCEPTIONS, PREFERENCE } from "./policy.mjs";

function tokenize(expr) {
  // Old Cargo manifests use "MIT/Apache-2.0" for OR.
  const s = String(expr).replace(/\//g, " OR ");
  const tokens = s.match(/\(|\)|[^\s()]+/g) ?? [];
  return tokens.map((t) => {
    const u = t.toUpperCase();
    return u === "AND" || u === "OR" || u === "WITH" ? u : t;
  });
}

/** Parse into { op: "id", id, exception? } | { op: "AND"|"OR", args: [...] }. Throws on malformed input. */
export function parseSpdx(expr) {
  const tokens = tokenize(expr);
  if (tokens.length === 0) throw new Error("empty SPDX expression");
  let i = 0;
  const peek = () => tokens[i];
  const next = () => tokens[i++];
  function parseOr() {
    const args = [parseAnd()];
    while (peek() === "OR") {
      next();
      args.push(parseAnd());
    }
    return args.length === 1 ? args[0] : { op: "OR", args };
  }
  function parseAnd() {
    const args = [parseAtom()];
    while (peek() === "AND") {
      next();
      args.push(parseAtom());
    }
    return args.length === 1 ? args[0] : { op: "AND", args };
  }
  function parseAtom() {
    const t = next();
    if (t === undefined) throw new Error(`unexpected end of SPDX expression: ${expr}`);
    if (t === "(") {
      const inner = parseOr();
      if (next() !== ")") throw new Error(`missing ")" in SPDX expression: ${expr}`);
      return inner;
    }
    if (t === ")" || t === "AND" || t === "OR" || t === "WITH") throw new Error(`unexpected "${t}" in SPDX expression: ${expr}`);
    const node = { op: "id", id: t };
    if (peek() === "WITH") {
      next();
      const ex = next();
      if (ex === undefined || ex === "(" || ex === ")") throw new Error(`missing exception after WITH: ${expr}`);
      node.exception = ex;
    }
    return node;
  }
  const ast = parseOr();
  if (i < tokens.length) throw new Error(`trailing tokens in SPDX expression: ${expr}`);
  return ast;
}

const canon = (list) => new Map(list.map((x) => [x.toLowerCase(), x]));

function rank(id, allowed) {
  const p = PREFERENCE.findIndex((x) => x.toLowerCase() === id.toLowerCase());
  if (p >= 0) return p;
  const a = allowed.findIndex((x) => x.toLowerCase() === id.toLowerCase());
  return PREFERENCE.length + (a >= 0 ? a : allowed.length);
}

/**
 * Evaluate against the allowlist.
 *   OR  -> ok if any option is ok; `picks` = ids of the most preferred ok option (MIT first)
 *   AND -> ok only if every part is ok; `picks` = union
 *   WITH -> base id must be allowed and the exception must be in ALLOWED_EXCEPTIONS
 * Returns { ok, picks: string[] (canonical allowlist spelling), violations: string[] }.
 */
export function evaluateSpdx(expr, allowed = ALLOWED, allowedExceptions = ALLOWED_EXCEPTIONS) {
  let ast;
  try {
    ast = parseSpdx(expr);
  } catch (e) {
    return { ok: false, picks: [], violations: [`${expr} (${e.message})`] };
  }
  const ids = canon(allowed);
  const excs = canon(allowedExceptions);
  const ev = (n) => {
    if (n.op === "id") {
      const id = ids.get(n.id.toLowerCase());
      if (!id) return { ok: false, picks: [], violations: [n.exception ? `${n.id} WITH ${n.exception}` : n.id] };
      if (n.exception && !excs.has(n.exception.toLowerCase())) return { ok: false, picks: [], violations: [`${n.id} WITH ${n.exception}`] };
      return { ok: true, picks: [id], violations: [] };
    }
    const rs = n.args.map(ev);
    if (n.op === "AND") {
      const ok = rs.every((r) => r.ok);
      return { ok, picks: ok ? [...new Set(rs.flatMap((r) => r.picks))] : [], violations: rs.flatMap((r) => r.violations) };
    }
    const good = rs.filter((r) => r.ok);
    if (good.length === 0) return { ok: false, picks: [], violations: rs.flatMap((r) => r.violations) };
    const best = good.reduce((a, b) => (Math.min(...b.picks.map((p) => rank(p, allowed))) < Math.min(...a.picks.map((p) => rank(p, allowed))) ? b : a));
    return { ok: true, picks: best.picks, violations: [] };
  };
  const r = ev(ast);
  return { ...r, violations: [...new Set(r.violations)] };
}
