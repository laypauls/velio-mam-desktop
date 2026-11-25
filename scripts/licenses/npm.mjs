// Which npm packages end up in the shipped frontend: packages whose modules are rendered into the Vite production
// bundle, plus packages whose CSS is inlined into the emitted stylesheet through `@import` (tailwindcss, fonts, ...).
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

/** `{ name, dir }` of the node_modules package a module id belongs to (last `node_modules/` wins - pnpm layout), or null. */
export function packageFromModuleId(id) {
  const clean = String(id).replace(/^\0/, "").replace(/[?#].*$/, "").replace(/\\/g, "/");
  const marker = "/node_modules/";
  const at = clean.lastIndexOf(marker);
  if (at < 0) return null;
  const segs = clean.slice(at + marker.length).split("/");
  const name = segs[0].startsWith("@") ? segs.slice(0, 2).join("/") : segs[0];
  if (!name || name.startsWith(".") || name.endsWith("/")) return null;
  return { name, dir: clean.slice(0, at + marker.length) + name };
}

/** Package names a stylesheet pulls in via `@import "pkg/..."` / `@plugin "pkg"` (relative, absolute and URL imports ignored). */
export function cssImportPackages(css) {
  const out = new Set();
  for (const m of css.matchAll(/@(?:import|plugin)\s+(?:url\()?\s*["']([^"']+)["']/g)) {
    const spec = m[1];
    if (/^(\.|\/|[a-z]+:)/i.test(spec)) continue;
    const segs = spec.split("/");
    out.add(spec.startsWith("@") ? segs.slice(0, 2).join("/") : segs[0]);
  }
  return [...out].sort();
}

/** All `.css` files below `dir` (sorted). */
export function listCssFiles(dir) {
  const out = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const full = join(d, e.name);
      if (e.isDirectory()) walk(full);
      else if (e.isFile() && e.name.endsWith(".css")) out.push(full);
    }
  };
  if (existsSync(dir) && statSync(dir).isDirectory()) walk(dir);
  return out.sort();
}

const personName = (a) => (typeof a === "string" ? a.replace(/\s*[<(].*$/, "").trim() : a?.name);

/** Normalize package.json license metadata to an SPDX-ish expression, or null. */
export function declaredLicense(pj) {
  if (typeof pj.license === "string") return pj.license;
  if (pj.license?.type) return pj.license.type;
  if (Array.isArray(pj.licenses) && pj.licenses.length) return pj.licenses.map((l) => l.type ?? l).join(" OR ");
  return null;
}

/**
 * moduleIds: ids of the modules rendered into the bundle; cssImports: package names imported from CSS (resolved in
 * `root/node_modules`); the root package itself is skipped. Returns [{ name, version, kind: "npm", dir, license, authors }]
 * sorted by name, version. Packages without a readable package.json are skipped.
 */
export function collectNpmPackages({ root, moduleIds, cssImports }) {
  const found = new Map();
  const add = (name, dir) => {
    const pjPath = join(dir, "package.json");
    if (!existsSync(pjPath)) return;
    const pj = JSON.parse(readFileSync(pjPath, "utf8"));
    const key = `${name}@${pj.version}`;
    if (found.has(key)) return;
    const authors = [pj.author, ...(Array.isArray(pj.contributors) ? pj.contributors : [])].map(personName).filter(Boolean).slice(0, 8);
    found.set(key, { name, version: pj.version, kind: "npm", dir, license: declaredLicense(pj), authors });
  };
  for (const id of moduleIds) {
    const p = packageFromModuleId(id);
    if (p) add(p.name, p.dir);
  }
  for (const name of cssImports) add(name, join(root, "node_modules", name));
  let rootName = null;
  try {
    rootName = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).name;
  } catch {
    /* no root package.json */
  }
  return [...found.values()]
    .filter((p) => p.name !== rootName)
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : a.version < b.version ? -1 : a.version > b.version ? 1 : 0));
}
