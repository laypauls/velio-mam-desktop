// License policy for scripts/dump-licenses.mjs (D-4). Change the lists here, not in the generator.
// Only what is distributed is checked: Rust crates reachable through normal edges (no build/dev/proc-macro) and npm packages in the Vite production bundle / app CSS imports (see dump-licenses.mjs).

/** SPDX ids that may ship in the installer. GPL / AGPL / SSPL / unknown are never allowed (fail the generator). */
export const ALLOWED = [
  "MIT",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Zlib",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "MPL-2.0",
  "CDLA-Permissive-2.0",
  "OFL-1.1",
  "0BSD",
  "CC0-1.0",
  "BSL-1.0",
];

/** `WITH <exception>` clauses that only relax an allowed license (never add an obligation). */
export const ALLOWED_EXCEPTIONS = ["LLVM-exception"];

/**
 * For `A OR B` the text of the most preferred allowed option is delivered. MIT first (matches the existing notice),
 * then the rest in this order; ids not listed rank after the list in ALLOWED order.
 */
export const PREFERENCE = ["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Zlib", "Unicode-3.0", "0BSD", "CC0-1.0", "BSL-1.0"];

/**
 * Explicit exceptions: packages whose declared license is missing/non-SPDX or outside the allowlist, but whose actual
 * terms were checked by hand. Key = `name` (all versions) or `name@version`. Every entry needs a reason.
 *   { allow: ["BlueOak-1.0.0"], reason }  extra ids allowed for this package only (its own license file is delivered)
 *   { expr: "MIT", reason }              evaluate/deliver this SPDX expression instead of the declared one
 */
export const EXCEPTIONS = {};

/**
 * Hand-checked copyright lines for packages whose published files carry no usable notice (no copyright line in the
 * license files, no authors). Used only as the last fallback. Key = `name` or `name@version`; value = line(s) shown in
 * the package's bullet. Re-check the upstream LICENSE when bumping a version (checked 2026-11-14 against the default branch).
 */
export const COPYRIGHT_OVERRIDES = {
  "cargo-platform": ["Copyright holders: see https://github.com/rust-lang/cargo (upstream LICENSE-MIT has no copyright line)"],
  hashlink: ["Copyright (c) 2015 The Rust Project Developers (linked-hash-map, from which hashlink is derived in part); other holders: see https://github.com/djc/hashlink"], // wrapped over two lines in LICENSE-MIT
  "image-webp": ["Copyright holders: see https://github.com/image-rs/image-webp (upstream LICENSE-MIT has no copyright line)"],
  lindera: ["Copyright (c) 2019 by the project authors. (upstream LICENSE; this crate ships no license file)"],
  "lindera-dictionary": ["Copyright (c) 2019 by the project authors. (upstream LICENSE; this crate ships no license file)"],
  "pin-project-lite": ["Copyright holders: see https://github.com/taiki-e/pin-project-lite (upstream LICENSE-MIT has no copyright line)"],
  serde_json: ["Copyright holders: see https://github.com/serde-rs/json (upstream LICENSE-MIT has no copyright line)"],
  "tantivy-columnar": ["Copyright (c) 2018 by the project authors, as listed in the AUTHORS file. (upstream LICENSE; this crate ships no license file)"],
  "tantivy-sstable": ["Copyright (c) 2018 by the project authors, as listed in the AUTHORS file. (upstream LICENSE; this crate ships no license file)"],
  "tantivy-stacker": ["Copyright (c) 2018 by the project authors, as listed in the AUTHORS file. (upstream LICENSE; this crate ships no license file)"],
  "tantivy-tokenizer-api": ["Copyright (c) 2018 by the project authors, as listed in the AUTHORS file. (upstream LICENSE; this crate ships no license file)"],
  "webpki-roots": ["Copyright holders: see https://github.com/rustls/webpki-roots (CA data derived from the Mozilla CA Certificate Program; the license text has no copyright line)"],
  "webview2-com": ["Copyright (c) 2021 Bill Avery (upstream LICENSE; this crate ships no license file)"],
  "webview2-com-sys": ["Copyright (c) 2021 Bill Avery (upstream LICENSE; this crate ships no license file)"],
  winnow: ["Copyright holders: see https://github.com/winnow-rs/winnow (upstream LICENSE-MIT has no copyright line)"],
};
