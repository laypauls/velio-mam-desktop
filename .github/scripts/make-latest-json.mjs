// Generates a latest.json stub for the Tauri updater (static-file format, see DEVELOPER.md).
// env: BUNDLE_DIR, OUT, VERSION, BASE_URL (https installer URL prefix), NOTES (optional)
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const { BUNDLE_DIR, OUT, VERSION, BASE_URL, NOTES = "" } = process.env;
if (!BUNDLE_DIR || !OUT || !VERSION || !BASE_URL) {
  console.error("make-latest-json: BUNDLE_DIR, OUT, VERSION, BASE_URL are required");
  process.exit(1);
}
const sigs = readdirSync(BUNDLE_DIR).filter((f) => f.endsWith(".exe.sig"));
if (sigs.length !== 1) {
  console.error(`make-latest-json: expected exactly one .exe.sig in ${BUNDLE_DIR}, found ${sigs.length}`);
  process.exit(1);
}
const installer = sigs[0].slice(0, -".sig".length);
const latest = {
  version: VERSION,
  notes: NOTES,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature: readFileSync(join(BUNDLE_DIR, sigs[0]), "utf8").trim(),
      url: `${BASE_URL.replace(/\/$/, "")}/${encodeURIComponent(installer)}`,
    },
  },
};
mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, JSON.stringify(latest, null, 2) + "\n");
console.log(`make-latest-json: wrote ${OUT} for ${installer}`);
