// Writes the tauri --config override for release builds. Never prints secrets.
// env: OUT (file), UPDATER_PUBKEY, UPDATER_ENDPOINT, CERT_THUMBPRINT (optional)
import { writeFileSync } from "node:fs";

const { OUT, UPDATER_PUBKEY, UPDATER_ENDPOINT, CERT_THUMBPRINT } = process.env;
if (!OUT || !UPDATER_PUBKEY || !UPDATER_ENDPOINT) {
  console.error("release-config: OUT, UPDATER_PUBKEY and UPDATER_ENDPOINT are required");
  process.exit(1);
}
if (!UPDATER_ENDPOINT.startsWith("https://")) {
  console.error("release-config: UPDATER_ENDPOINT must be https");
  process.exit(1);
}
const config = {
  plugins: { updater: { pubkey: UPDATER_PUBKEY, endpoints: [UPDATER_ENDPOINT] } },
};
if (CERT_THUMBPRINT) {
  config.bundle = {
    windows: {
      certificateThumbprint: CERT_THUMBPRINT,
      digestAlgorithm: "sha256",
      timestampUrl: "http://timestamp.digicert.com",
    },
  };
}
writeFileSync(OUT, JSON.stringify(config));
console.log(`release-config: wrote ${OUT} (authenticode=${Boolean(CERT_THUMBPRINT)})`);
