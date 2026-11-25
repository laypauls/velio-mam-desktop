// License file discovery and text helpers (fs reads only in findLicenseFiles / loadSpdxText).
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const CODE_EXT = /\.(rs|js|mjs|cjs|ts|tsx|jsx|json|toml|html?|css|ya?ml|map|py|c|h|cc|cpp|hpp|go|sh|ps1|lock|png|svg|gz|zip)$/i;
const MAX_FILE_BYTES = 2 * 1024 * 1024;

/** `\r\n` -> `\n`, BOM removed. */
export const lf = (s) => s.replace(/^﻿/, "").replace(/\r\n?/g, "\n");

/**
 * LICENSE* / LICENCE* / COPYING* / NOTICE* files (case-insensitive, incl. LICENSE-MIT, LICENSE-APACHE, LICENSE.md,
 * COPYING.LIB) directly in `dir`. Source/code extensions and directories are skipped.
 * Returns [{ name, kind: "license" | "notice", text }] sorted by name.
 */
export function findLicenseFiles(dir) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return [];
  }
  const out = [];
  for (const e of entries) {
    if (!e.isFile()) continue;
    const m = /^(licen[sc]e|copying|notice)/i.exec(e.name);
    if (!m || CODE_EXT.test(e.name)) continue;
    const full = join(dir, e.name);
    if (statSync(full).size > MAX_FILE_BYTES) continue;
    out.push({ name: e.name, kind: m[1].toLowerCase() === "notice" ? "notice" : "license", text: lf(readFileSync(full, "utf8")) });
  }
  return out.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

/** Whitespace-normalized text, used only to compare / group. */
export const normalizeText = (t) => lf(t).replace(/\s+/g, " ").trim();

/** Display form: LF, trailing spaces removed, runs of blank lines collapsed. */
export const cleanText = (t) =>
  lf(t)
    .split("\n")
    .map((l) => l.replace(/\s+$/, ""))
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();

const PLACEHOLDER = /(\byyyy\b|<year>|\[year\]|\{year\}|\{yyyy\}|<copyright holders?>|<owner>|\[name of copyright owner\]|<name of author>|\[fullname\]|<organization>|<insert|\[insert)/i;
const NOT_A_NOTICE = /^copyright\s+(notice|and|holders?|owner|law|license|statement|information|ownership)\b/i;

/** True for a real copyright notice line ("Copyright (c) 2015 Foo", "(c) 2020 Bar", "Copyright Foo"), not a template. */
export function isCopyrightLine(line) {
  const l = line.replace(/^[\s#*/>\-–—|]+/, "").trim();
  if (!l || l.length > 300 || PLACEHOLDER.test(l) || NOT_A_NOTICE.test(l)) return false;
  if (/^copyright\s*(?:\(c\)|©)/i.test(l)) return true;
  if (/^(?:\(c\)|©)\s*\S/i.test(l)) return true;
  if (/^copyright[:\s]+\d{4}/i.test(l)) return true;
  // Mixed-case "Copyright Foo" only: an upper-case "COPYRIGHT HOLDERS BE LIABLE" is a wrapped warranty-disclaimer line.
  return /^Copyright\s+[A-Z]/.test(l);
}

/** Unique copyright lines of the given texts, in first-seen order, cleaned. */
export function extractCopyrights(texts) {
  const seen = new Set();
  const out = [];
  for (const t of texts) {
    for (const raw of lf(t).split("\n")) {
      if (!isCopyrightLine(raw)) continue;
      const l = raw.replace(/^[\s#*/>\-–—|]+/, "").replace(/\s+/g, " ").trim();
      if (!seen.has(l)) {
        seen.add(l);
        out.push(l);
      }
    }
  }
  return out;
}

/** Text without real copyright lines (they are listed per package instead) — the basis for grouping. */
export const stripCopyrights = (t) =>
  cleanText(
    lf(t)
      .split("\n")
      .filter((l) => !isCopyrightLine(l))
      .join("\n"),
  );

/** SPDX ids whose standard wording appears in `text` (heuristic, empty when unrecognized). */
export function detectSpdx(text) {
  const t = normalizeText(text).toLowerCase().replace(/[’‘]/g, "'");
  const ids = [];
  if (/apache license/.test(t) && /version 2\.0/.test(t)) ids.push("Apache-2.0");
  if (/mozilla public license,? (version|v\.?) ?2\.0/.test(t)) ids.push("MPL-2.0");
  if (/unicode license v3/.test(t)) ids.push("Unicode-3.0");
  if (/unicode, inc\. license agreement/.test(t)) ids.push("Unicode-DFS-2016");
  if (/sil open font license,? version 1\.1/.test(t)) ids.push("OFL-1.1");
  if (/cc0 1\.0 universal/.test(t)) ids.push("CC0-1.0");
  if (/boost software license/.test(t)) ids.push("BSL-1.0");
  if (/community data license agreement.{1,6}permissive.{1,6}version 2\.0/.test(t)) ids.push("CDLA-Permissive-2.0");
  if (/gnu lesser general public license/.test(t) && /version 2\.1/.test(t)) ids.push("LGPL-2.1");
  if (/gnu lesser general public license/.test(t) && /version 3,? 29 june 2007/.test(t)) ids.push("LGPL-3.0");
  if (/gnu general public license/.test(t) && /version 3,? 29 june 2007/.test(t) && !/gnu lesser general public license/.test(t)) ids.push("GPL-3.0");
  if (/permission is hereby granted, free of charge/.test(t) && /the software is provided "as is"/.test(t)) ids.push("MIT");
  if (/permission to use, copy, modify, and\/or distribute this software for any purpose with or without fee/.test(t)) {
    ids.push(/provided that the above copyright notice/.test(t) ? "ISC" : "0BSD");
  }
  if (/redistribution and use in source and binary forms/.test(t) && /redistributions of source code must retain/.test(t)) {
    ids.push(/neither the name/.test(t) ? "BSD-3-Clause" : "BSD-2-Clause");
  }
  if (/this software is provided 'as-is'/.test(t) && /altered source versions must be plainly marked/.test(t)) ids.push("Zlib");
  return ids;
}

/** Standard text for an SPDX id from the pinned `licenses/` folder (template copyright line removed), or null. */
export function loadSpdxText(licensesDir, id) {
  const f = join(licensesDir, `${id}.txt`);
  if (!existsSync(f)) return null;
  return cleanText(
    lf(readFileSync(f, "utf8"))
      .split("\n")
      .filter((l) => !/^\s*copyright\b.*(<year>|<copyright holders?>|<owner>|\[yyyy\])/i.test(l))
      .join("\n"),
  );
}
