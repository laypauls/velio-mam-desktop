// Resolve one package into the license texts / copyright lines / NOTICE files that must ship with it.
import { ALLOWED, COPYRIGHT_OVERRIDES, EXCEPTIONS } from "./policy.mjs";
import { evaluateSpdx } from "./spdx.mjs";
import { detectSpdx, extractCopyrights, findLicenseFiles, loadSpdxText, normalizeText, stripCopyrights } from "./texts.mjs";

/**
 * pkg: { name, version, kind: "rust"|"npm", dir, license: string|null, authors: string[] }
 * opts: { licensesDir, exceptions? }
 * Copyright lines: license files > NOTICE > authors > COPYRIGHT_OVERRIDES (hand-checked) > problem.
 * Returns { name, version, kind, expr, entries: [{ id, text, source: "file"|"spdx", file? }], copyrights: string[],
 *           notices: [{ file, text }], problems: [{ type: "unknown"|"violation"|"text"|"copyright", message }] }
 *   - entries[].text is the license text WITHOUT its copyright lines (copyrights are listed per package)
 *   - text priority per picked id: files whose wording matches the id > unrecognized single license file > SPDX standard text
 */
export function resolvePackage(pkg, { licensesDir, exceptions = EXCEPTIONS, overrides = COPYRIGHT_OVERRIDES }) {
  const res = { name: pkg.name, version: pkg.version, kind: pkg.kind, expr: pkg.license ?? null, entries: [], copyrights: [], notices: [], problems: [] };
  const exc = exceptions[`${pkg.name}@${pkg.version}`] ?? exceptions[pkg.name];
  const expr = exc?.expr ?? pkg.license;
  const label = `${pkg.kind} ${pkg.name} ${pkg.version}`;
  if (!expr || !String(expr).trim() || /^unknown$/i.test(String(expr).trim())) {
    res.problems.push({ type: "unknown", message: `${label}: no license declared` });
    return res;
  }
  res.expr = expr;
  const ev = evaluateSpdx(expr, [...ALLOWED, ...(exc?.allow ?? [])]);
  if (!ev.ok) {
    res.problems.push({ type: "violation", message: `${label}: license "${expr}" is not allowed (${ev.violations.join(", ")})` });
    return res;
  }

  const files = findLicenseFiles(pkg.dir);
  const licFiles = files.filter((f) => f.kind === "license").map((f) => ({ ...f, ids: detectSpdx(f.text) }));
  res.notices = files.filter((f) => f.kind === "notice").map(({ name, text }) => ({ file: name, text }));

  const seen = new Set();
  const add = (id, text, source, file) => {
    const stripped = stripCopyrights(text);
    const key = `${id}\u0000${normalizeText(stripped)}`;
    if (!stripped || seen.has(key)) return;
    seen.add(key);
    res.entries.push({ id, text: stripped, source, file, raw: text });
  };
  for (const id of ev.picks) {
    const matched = licFiles.filter((f) => f.ids.includes(id));
    const unrecognized = licFiles.filter((f) => f.ids.length === 0);
    if (matched.length > 0) {
      for (const f of matched) add(id, f.text, "file", f.name);
    } else if (ev.picks.length === 1 && unrecognized.length > 0) {
      for (const f of unrecognized) add(id, f.text, "file", f.name);
    } else {
      const std = loadSpdxText(licensesDir, id);
      if (std) add(id, std, "spdx");
      else res.problems.push({ type: "text", message: `${label}: no license file and no pinned text for ${id}` });
    }
  }

  const fromFiles = res.entries.filter((e) => e.source === "file").map((e) => e.raw);
  let cr = extractCopyrights(fromFiles);
  if (cr.length === 0) cr = extractCopyrights(res.notices.map((n) => n.text));
  if (cr.length === 0 && pkg.authors?.length) cr = [`Copyright holders: see authors: ${pkg.authors.join(", ")}`];
  if (cr.length === 0) cr = overrides[`${pkg.name}@${pkg.version}`] ?? overrides[pkg.name] ?? [];
  if (cr.length === 0) res.problems.push({ type: "copyright", message: `${label}: no copyright line in license files, no authors, no COPYRIGHT_OVERRIDES entry` });
  res.copyrights = cr;
  for (const e of res.entries) delete e.raw;
  return res;
}
