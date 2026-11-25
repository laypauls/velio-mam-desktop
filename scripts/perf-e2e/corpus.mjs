import { DatabaseSync } from "node:sqlite"
import fs from "node:fs"
const db = new DatabaseSync(process.argv[2], { readOnly: true }) // usage: node corpus.mjs <mam.sqlite>
const tags = db.prepare("SELECT name FROM tags LIMIT 200").all().map((r) => r.name)
const kw = db.prepare("SELECT ai_keywords k FROM poc_asset_text WHERE ai_keywords IS NOT NULL LIMIT 3000").all().map((r) => r.k)
const desc = db.prepare("SELECT ai_description d FROM poc_asset_text WHERE ai_description IS NOT NULL LIMIT 400").all().map((r) => r.d)
const names = db.prepare("SELECT rel_path p FROM file_locations LIMIT 400").all().map((r) => r.p.split(/[\/]/).pop())
console.log(tags.slice(0, 12), kw.slice(0, 3), desc.slice(0, 3), names.slice(0, 6))
const cols = db.prepare("PRAGMA table_info(media_assets)").all().map((c) => c.name)
console.log(cols.join(","))
fs.writeFileSync("corpus.json", JSON.stringify({ tags, kw, desc, names }))
