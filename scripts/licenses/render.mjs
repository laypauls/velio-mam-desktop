// Group resolved packages by license text and render THIRD_PARTY_LICENSES.md (format contract: see
// docs/requests/2026-11/20261114-171520-settings-open-source-licenses.md). Pure functions, deterministic output.
import { createHash } from "node:crypto";
import { detectSpdx, lf, normalizeText } from "./texts.mjs";

export const NAMES = {
  MIT: "MIT License",
  "Apache-2.0": "Apache License 2.0",
  "BSD-2-Clause": "BSD 2-Clause License",
  "BSD-3-Clause": "BSD 3-Clause License",
  ISC: "ISC License",
  Zlib: "zlib License",
  "Unicode-3.0": "Unicode License v3",
  "Unicode-DFS-2016": "Unicode License Agreement (Data Files and Software, 2016)",
  "MPL-2.0": "Mozilla Public License 2.0",
  "CDLA-Permissive-2.0": "Community Data License Agreement Permissive 2.0",
  "OFL-1.1": "SIL Open Font License 1.1",
  "0BSD": "BSD Zero Clause License",
  "CC0-1.0": "Creative Commons Zero v1.0 Universal",
  "BSL-1.0": "Boost Software License 1.0",
  "LGPL-2.1": "GNU Lesser General Public License v2.1",
  "LGPL-3.0": "GNU Lesser General Public License v3.0",
  "GPL-3.0": "GNU General Public License v3.0",
  "BlueOak-1.0.0": "Blue Oak Model License 1.0.0",
  "CC-BY-4.0": "Creative Commons Attribution 4.0 International",
  "GPL-2.0": "GNU General Public License v2.0",
  "LGPL-2.0": "GNU Library General Public License v2.0",
  FTL: "FreeType Project License",
  IJG: "Independent JPEG Group License",
  "libpng-2.0": "PNG Reference Library License v2",
  "TU-Berlin-2.0": "Technische Universitaet Berlin License 2.0",
  "BSD-2-Clause-Patent": "BSD-2-Clause Plus Patent License",
  "BSD-3-Clause-Clear": "BSD 3-Clause Clear License",
  "MIT-Modern-Variant": "MIT License Modern Variant",
  WTFPL: "Do What The F*ck You Want To Public License",
  "ZPL-2.1": "Zope Public License 2.1",
  Unlicense: "The Unlicense",
  "MIT-0": "MIT No Attribution",
  "MPL-1.1": "Mozilla Public License 1.1",
  curl: "curl License",
  "PCRE2-exception": "PCRE2 exception",
  "GCC-exception-3.1": "GCC Runtime Library Exception 3.1",
  "Bison-exception-2.2": "Bison exception 2.2",
  "AOM-Patent-1.0": "Alliance for Open Media Patent License 1.0",
};

export const sha256 = (s) => createHash("sha256").update(lf(typeof s === "string" ? s : s.toString("utf8"))).digest("hex");

/**
 * Grouping key: wording only - lower-case letters/digits, so punctuation, markdown decoration, line wrapping and a leading
 * title line ("MIT License", "The MIT License (MIT)") do not split one license into many sections.
 */
export function groupKey(text) {
  const words = normalizeText(text).toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
  return words.replace(/^(the )?(mit|isc|bsd( [23] clause)?|zlib) license( (mit|isc|bsd|zlib))? /, "");
}

const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const KIND_ORDER = { rust: 0, npm: 1, bundled: 2 };
const pkgCmp = (a, b) => cmp(a.name.toLowerCase(), b.name.toLowerCase()) || cmp(a.name, b.name) || cmp(a.version, b.version) || (KIND_ORDER[a.kind] ?? 9) - (KIND_ORDER[b.kind] ?? 9);

const esc = (s) => s.replace(/[\\`*_<>[\]|]/g, "\\$&");

/** Code fence long enough not to be closed by the text itself. */
function fence(text, info = "") {
  const runs = text.match(/`{3,}/g) ?? [];
  const n = Math.max(3, ...runs.map((r) => r.length + 1));
  const f = "`".repeat(n);
  return `${f}${info}\n${text}\n${f}`;
}

/**
 * packages: resolved packages ({ name, version, kind, copyrights, entries, notices }). Bundled binaries use kind "bundled".
 * extraSections: [{ title, packages: [{name, version, kind, copyright}], text }] appended after the groups.
 * Returns { groups, uniquePackages }.
 */
export function groupPackages(packages) {
  const map = new Map();
  for (const p of packages) {
    let firstEntry = true;
    for (const e of p.entries) {
      const key = groupKey(e.text);
      let g = map.get(key);
      if (!g) map.set(key, (g = { key, text: e.text, ids: new Map(), members: [], notices: [] }));
      g.ids.set(e.id, (g.ids.get(e.id) ?? 0) + 1);
      g.members.push(p);
      if (firstEntry && p.notices?.length) g.notices.push({ pkg: p, notices: p.notices });
      firstEntry = false;
    }
  }
  const groups = [...map.values()].map((g) => {
    const detected = detectSpdx(g.text);
    const byCount = [...g.ids].sort((a, b) => b[1] - a[1] || cmp(a[0], b[0]));
    const id = detected.find((d) => g.ids.has(d)) ?? byCount[0][0];
    g.members.sort(pkgCmp);
    g.id = id;
    return g;
  });
  // Same SPDX id but a different wording -> "variant N" (N by package count desc, then text for a stable order).
  const byId = new Map();
  for (const g of groups) byId.set(g.id, [...(byId.get(g.id) ?? []), g]);
  for (const list of byId.values()) {
    list.sort((a, b) => b.members.length - a.members.length || cmp(a.key, b.key));
    list.forEach((g, i) => (g.variant = i + 1));
  }
  groups.sort((a, b) => b.members.length - a.members.length || cmp(a.id, b.id) || a.variant - b.variant);
  return groups;
}

const groupTitle = (g) => {
  const name = NAMES[g.id] ?? `License text (${g.id})`;
  const idLabel = g.variant > 1 ? `${g.id}, variant ${g.variant}` : g.id;
  return `${name} (${idLabel}) — ${g.members.length} packages`;
};

const bullet = (p) => `- \`${p.name}\` ${p.version} (${p.kind}) — ${p.copyrights.length ? p.copyrights.map(esc).join("; ") : "Copyright holders: not stated by the package"}`;

/** Header comment line (first machine-readable line). */
export const headerComment = ({ cargoSha, pnpmSha, count }) =>
  `<!-- generated-by: scripts/dump-licenses.mjs; cargo-lock-sha256: ${cargoSha}; pnpm-lock-sha256: ${pnpmSha}; packages: ${count} -->`;

export function renderDocument({ packages, extraSections = [], cargoSha, pnpmSha, counts }) {
  const groups = groupPackages(packages);
  const lockPkgs = new Set(packages.filter((p) => p.kind !== "bundled").map((p) => `${p.kind}:${p.name}@${p.version}`));
  const out = [
    "# Third-Party Licenses",
    headerComment({ cargoSha, pnpmSha, count: lockPkgs.size }),
    "",
    `This file reproduces the copyright notices and license texts of the third-party software shipped with velio.: ${counts.rust} Rust crates and ${counts.npm} npm packages linked into the application, plus ${counts.bundled ?? 0} bundled program components (FFmpeg, libmpv, whisper.cpp and the libraries built into them) and the onnxruntime notices.`,
    "Packages that share the same license text are grouped under one section. Each package line shows the copyright notice of that package; the license text applies to every package listed in its section.",
    "It is generated by scripts/dump-licenses.mjs from Cargo.lock and pnpm-lock.yaml. See THIRD_PARTY_NOTICES.md for the notices that apply to the bundled programs (FFmpeg, whisper.cpp, libmpv).",
    "",
  ];
  for (const g of groups) {
    out.push(`## ${groupTitle(g)}`, "", "### Packages", "", ...g.members.map(bullet), "", "### License text", "", fence(g.text, "text"), "");
    if (g.notices.length) {
      out.push("### NOTICE", "");
      for (const { pkg, notices } of g.notices.sort((a, b) => pkgCmp(a.pkg, b.pkg))) {
        for (const n of notices) out.push(`#### \`${pkg.name}\` ${pkg.version}${notices.length > 1 ? ` (${n.file})` : ""}`, "", fence(n.text.trim()), "");
      }
    }
  }
  for (const s of extraSections) {
    out.push(`## ${s.title} — ${s.packages.length} packages`, "", "### Packages", "", ...s.packages.sort(pkgCmp).map(bullet), "", "### License text", "", fence(s.text, "text"), "");
  }
  return out.join("\n").replace(/\n+$/, "\n");
}

/** Parse the committed file's header and package bullets (for --check). */
export function parseGenerated(md) {
  const text = lf(md);
  const h = /<!-- generated-by: [^;]+; cargo-lock-sha256: ([0-9a-f]+); pnpm-lock-sha256: ([0-9a-f]+); packages: (\d+) -->/.exec(text);
  const pkgs = new Set();
  for (const m of text.matchAll(/^- `([^`]+)` (\S+) \((rust|npm)\) — /gm)) pkgs.add(`${m[3]}:${m[1]}@${m[2]}`);
  return h ? { cargoSha: h[1], pnpmSha: h[2], count: Number(h[3]), packages: pkgs } : null;
}

/** Diff summary between the committed file header and the current lockfile state. */
export function diffSummary(committed, current) {
  if (!committed) return ["THIRD_PARTY_LICENSES.md has no readable generated-by header (missing or not generated)."];
  const lines = [];
  lines.push(`Cargo.lock sha256: ${committed.cargoSha === current.cargoSha ? "unchanged" : "CHANGED"}`);
  lines.push(`pnpm-lock.yaml sha256: ${committed.pnpmSha === current.pnpmSha ? "unchanged" : "CHANGED"}`);
  const added = [...current.packages].filter((p) => !committed.packages.has(p)).sort();
  const removed = [...committed.packages].filter((p) => !current.packages.has(p)).sort();
  lines.push(`packages: committed ${committed.packages.size}, current ${current.packages.size} (+${added.length} / -${removed.length})`);
  for (const p of added) lines.push(`  + ${p}`);
  for (const p of removed) lines.push(`  - ${p}`);
  const same = committed.cargoSha === current.cargoSha && committed.pnpmSha === current.pnpmSha && !added.length && !removed.length;
  lines.push(same ? "Up to date." : "Regeneration may be needed (node scripts/dump-licenses.mjs) - request it as a separate task.");
  return lines;
}
