// Reads the license tables of THIRD_PARTY_NOTICES.md (libraries inside the bundled FFmpeg / whisper.cpp / libmpv binaries)
// and maps every SPDX id found there to a pinned text in licenses/.
import { existsSync } from "node:fs";
import { join } from "node:path";
import { lf } from "./texts.mjs";

/** Sections whose tables list bundled-binary components. */
export const NOTICE_SECTIONS = ["1.1.1", "1.2", "1.3.1"];

/** Prose names that are not SPDX ids, mapped to the pinned text file id. */
const PHRASES = [[/Alliance for Open Media Patent License 1\.0/gi, "AOM-Patent-1.0"]];
/** Single-word / digit-less ids (everything else must contain a digit and a hyphen). */
const NAME_ONLY = new Set(["MIT", "ISC", "FTL", "IJG", "WTFPL", "Unlicense", "Zlib", "MIT-Modern-Variant", "curl"]);

/** Pinned-file ids (`licenses/<id>.txt`) for every license / exception named in a license-column cell. */
export function licenseIds(cell) {
  let s = cell.replace(/`[^`]*`/g, " ");
  for (const [re, id] of PHRASES) s = s.replace(re, ` ${id} `);
  const out = new Set();
  for (const raw of s.match(/[A-Za-z0-9][A-Za-z0-9.+-]*/g) ?? []) {
    const tok = raw.replace(/[.+-]+$/, "");
    const ok = NAME_ONLY.has(tok) || (/^[A-Za-z0-9.]+(-[A-Za-z0-9.]+)+$/.test(tok) && /\d/.test(tok));
    if (ok) out.add(tok.replace(/-(or-later|only)$/i, ""));
  }
  return [...out].sort();
}

export const pinnedTextPath = (licensesDir, id) => join(licensesDir, `${id}.txt`);
export const hasPinnedText = (licensesDir, id) => existsSync(pinnedTextPath(licensesDir, id));

/** Table rows of the given sections: [{ section, component, version, license, copyright }]. */
export function parseNoticeTables(md, sections = NOTICE_SECTIONS) {
  const rows = [];
  let section = null;
  let cols = null;
  for (const line of lf(md).split("\n")) {
    const h = /^#{2,4}\s+(\d+(?:\.\d+)*)\b/.exec(line);
    if (h) {
      section = sections.includes(h[1]) ? h[1] : null;
      cols = null;
      continue;
    }
    if (!section || !line.startsWith("|")) continue;
    const cells = line.replace(/^\||\|\s*$/g, "").split("|").map((c) => c.trim());
    if (!cols) {
      cols = {
        component: cells.findIndex((c) => /^(library|component)$/i.test(c)),
        version: cells.findIndex((c) => /^version$/i.test(c)),
        license: cells.findIndex((c) => /^license/i.test(c)),
        copyright: cells.findIndex((c) => /^copyright$/i.test(c)),
      };
      continue;
    }
    if (/^:?-{3,}/.test(cells[0])) continue;
    rows.push({
      section,
      component: cells[cols.component] ?? "",
      version: cols.version >= 0 ? cells[cols.version] : "",
      license: cells[cols.license] ?? "",
      copyright: cols.copyright >= 0 ? cells[cols.copyright] : "",
    });
  }
  return rows;
}
