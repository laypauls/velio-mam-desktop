// Generates THIRD_PARTY_LICENSES.md (English): copyright notices + license texts of the third-party software that is
// actually distributed in the product, grouped by license text, plus the bundled-program license texts and the
// onnxruntime third-party notices from the pinned ./licenses folder. No extra tooling: node + cargo + the project's vite.
//
//   node scripts/dump-licenses.mjs            regenerate THIRD_PARTY_LICENSES.md (exit 1 + list on a policy failure, nothing written)
//   node scripts/dump-licenses.mjs --check    no write: compare Cargo.lock / pnpm-lock.yaml hashes and the package set with
//                                             the committed file's header and print the difference (always exit 0)
//
// Scope (dev/build tooling needs no notice):
//   Rust  crates reachable from the workspace through `normal` dependency edges for x86_64-pc-windows-msvc
//         (build/dev edges and proc-macro crates excluded)                                   -> scripts/licenses/rust.mjs
//   npm   packages whose modules are rendered into a Vite production build (run in memory, nothing written) plus the
//         packages imported by the app's CSS (@import "tailwindcss", fonts, ...)             -> scripts/licenses/npm.mjs
//   bundled programs: FFmpeg, libmpv, whisper.cpp, onnxruntime, Inter + every component listed in the license tables of
//         THIRD_PARTY_NOTICES.md (sections 1.1.1, 1.2, 1.3.1)                                -> scripts/licenses/notices.mjs
// Policy (allowlist, exceptions, preference for "A OR B"): scripts/licenses/policy.mjs. Texts: licenses/ (README.md).
// Regeneration is a manual decision (separate task) - CI and builds never run it.
import { execSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { resolvePackage } from "./licenses/collect.mjs";
import { collectNpmPackages, cssImportPackages, listCssFiles } from "./licenses/npm.mjs";
import { hasPinnedText, licenseIds, parseNoticeTables } from "./licenses/notices.mjs";
import { parseGenerated, diffSummary, renderDocument, sha256 } from "./licenses/render.mjs";
import { selectRustPackages } from "./licenses/rust.mjs";
import { lf, loadSpdxText } from "./licenses/texts.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const licensesDir = join(root, "licenses");
const outFile = join(root, "THIRD_PARTY_LICENSES.md");
const check = process.argv.includes("--check");
const run = (cmd, args) => execSync([cmd, ...args].join(" "), { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });

// --- Rust ---
function listRust() {
  const meta = JSON.parse(run("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", "x86_64-pc-windows-msvc"]));
  return selectRustPackages(meta).map((p) => ({ name: p.name, version: p.version, kind: "rust", dir: dirname(p.manifest_path), license: p.license ?? null, authors: p.authors ?? [] }));
}

// --- npm: modules rendered into a production build (opt-in, in memory: vite.config.ts is not touched) ---
async function listNpm() {
  const { build } = await import(pathToFileURL(join(root, "node_modules/vite/dist/node/index.js")).href);
  const moduleIds = new Set();
  const collect = {
    name: "collect-rendered-modules",
    apply: "build",
    generateBundle(_options, bundle) {
      for (const f of Object.values(bundle)) {
        if (f.type !== "chunk") continue;
        for (const [id, m] of Object.entries(f.modules)) if (m.renderedLength > 0) moduleIds.add(id);
      }
    },
  };
  await build({ root, logLevel: "error", plugins: [collect], build: { write: false, emptyOutDir: false } });
  const cssImports = [...new Set(listCssFiles(join(root, "src")).flatMap((f) => cssImportPackages(readFileSync(f, "utf8"))))].sort();
  return collectNpmPackages({ root, moduleIds, cssImports });
}

const key = (p) => `${p.kind}:${p.name}@${p.version}`;
const lockHashes = () => ({
  cargoSha: sha256(readFileSync(join(root, "Cargo.lock"), "utf8")),
  pnpmSha: sha256(readFileSync(join(root, "pnpm-lock.yaml"), "utf8")),
});

if (check) {
  const current = { ...lockHashes(), packages: new Set([...listRust(), ...(await listNpm())].map(key)) };
  const committed = existsSync(outFile) ? parseGenerated(readFileSync(outFile, "utf8")) : null;
  console.log(diffSummary(committed, current).join("\n"));
  process.exit(0);
}

// --- Bundled programs and libraries (not in the lockfiles): pinned texts from ./licenses ---
// Pinned texts are kept verbatim (their own copyright lines are part of the license text); only SPDX template lines go.
const pinned = (id) => {
  const text = loadSpdxText(licensesDir, id);
  if (text === null) throw new Error(`licenses/${id}.txt is missing`);
  return { id, text, source: "spdx" };
};
const bundled = (name, version, ids, copyright) => ({ name, version, kind: "bundled", entries: ids.map(pinned), copyrights: [copyright], notices: [] });
const bundledPackages = [
  // Versions/sources: THIRD_PARTY_NOTICES.md section 1 and scripts/fetch-sidecars.ps1.
  bundled("FFmpeg", "n8.1.3", ["LGPL-3.0"], "Copyright (c) the FFmpeg developers"),
  bundled("FFmpeg (GPL-3.0 text incorporated by reference in LGPL-3.0; FFmpeg itself is not distributed under the GPL)", "n8.1.3", ["GPL-3.0"], "Copyright (c) Free Software Foundation, Inc. (license text)"),
  bundled("libmpv (mpv with the FFmpeg and libraries built into libmpv-2.dll)", "0.42.0", ["LGPL-3.0"], "Copyright (c) the mpv developers and the FFmpeg developers"),
  bundled("whisper.cpp", "b5454", ["MIT"], "Copyright (c) 2023-2026 The ggml authors"),
  bundled("ONNX Runtime", "1.31.0", ["MIT"], "Copyright (c) Microsoft Corporation"),
  bundled("Inter (font, via @fontsource-variable/inter)", "4.1", ["OFL-1.1"], "Copyright (c) 2016 The Inter Project Authors (https://github.com/rsms/inter)"),
];

// Libraries inside the bundled binaries: one entry per table row, one text per license/exception id named in the row.
const noticesPath = join(root, "THIRD_PARTY_NOTICES.md");
const inline = (s) => s.replace(/[`*]/g, "").replace(/\s+/g, " ").trim();
const seenRows = new Set();
for (const r of existsSync(noticesPath) ? parseNoticeTables(readFileSync(noticesPath, "utf8")) : []) {
  const ids = licenseIds(r.license);
  const name = inline(r.component);
  const version = inline(r.version).replace(/\s+/g, "-");
  if (!name || ids.length === 0 || seenRows.has(`${name}|${version}`)) continue;
  seenRows.add(`${name}|${version}`);
  const missing = ids.filter((id) => !hasPinnedText(licensesDir, id));
  if (missing.length) {
    console.error(`THIRD_PARTY_NOTICES.md section ${r.section}: no pinned text in licenses/ for ${missing.join(", ")} (${name})`);
    process.exit(1);
  }
  const lic = `license listed in THIRD_PARTY_NOTICES.md ${r.section}: ${inline(r.license)}`;
  bundledPackages.push(bundled(name, version.length > 30 ? "see-notices" : version || "-", ids, `${inline(r.copyright) || "Copyright holders: see source"} (${lic})`));
}

const ORT_NOTICES = join(licensesDir, "ThirdPartyNotices-onnxruntime-1.31.0.txt");
const extraSections = [
  {
    title: "ONNX Runtime third-party notices (ThirdPartyNotices.txt of onnxruntime v1.31.0)",
    packages: [{ name: "ONNX Runtime third-party components", version: "1.31.0", kind: "bundled", copyrights: ["Copyright (c) Microsoft Corporation and the third parties named in the text below"] }],
    text: lf(readFileSync(ORT_NOTICES, "utf8")).trim(),
  },
];

const rust = listRust();
const npm = await listNpm();
const resolved = [...rust, ...npm].map((p) => resolvePackage(p, { licensesDir }));
const problems = resolved.flatMap((r) => r.problems);
if (problems.length) {
  console.error(`License policy failed - ${problems.length} problem(s), nothing written:`);
  for (const p of problems.sort((a, b) => a.type.localeCompare(b.type) || a.message.localeCompare(b.message))) console.error(`  [${p.type}] ${p.message}`);
  console.error("Fix: add a pinned text under licenses/, or - after checking the real terms - an entry with a reason in scripts/licenses/policy.mjs EXCEPTIONS.");
  process.exit(1);
}

const md = renderDocument({ packages: [...resolved, ...bundledPackages], extraSections, ...lockHashes(), counts: { rust: rust.length, npm: npm.length, bundled: bundledPackages.length } });
writeFileSync(outFile, md);
const sections = (md.match(/^## /gm) ?? []).length;
console.log(`wrote ${outFile}: rust ${rust.length}, npm ${npm.length}, bundled ${bundledPackages.length}, sections ${sections}, ${(Buffer.byteLength(md) / 1024).toFixed(0)} KiB`);
