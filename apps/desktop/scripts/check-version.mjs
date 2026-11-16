// The version is written in three files. This checks they agree, and with the release tag
// when one is given: `node scripts/check-version.mjs v0.2.0`.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (...path) => readFileSync(join(desktop, ...path), "utf8");

const versions = {
  "Cargo.toml": read("..", "..", "Cargo.toml").match(/^version = "(.+)"$/m)?.[1],
  "apps/desktop/package.json": JSON.parse(read("package.json")).version,
  "apps/desktop/src-tauri/tauri.conf.json": JSON.parse(read("src-tauri", "tauri.conf.json")).version,
};
const tag = process.argv[2];
if (tag) versions[`tag ${tag}`] = tag.replace(/^v/, "");

const distinct = new Set(Object.values(versions));
if (distinct.size !== 1 || distinct.has(undefined)) {
  console.error("The version differs between:");
  for (const [where, version] of Object.entries(versions)) console.error(`  ${version}  ${where}`);
  process.exit(1);
}
console.log(`version ${[...distinct][0]}`);
