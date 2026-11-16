// Builds `termlabd` and `termlab-hook` for the platform being packaged and puts them where
// the bundler expects programs that ship next to the app: `src-tauri/binaries/<name>-<target>`.
//
// Run by `npm run bundle` (see `src-tauri/release.conf.json`). The Tauri CLI passes the
// target in TAURI_ENV_TARGET_TRIPLE; run by hand, this computer's own target is used.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "..");
const root = join(desktop, "..", "..");

function capture(program, args) {
  return execFileSync(program, args, { cwd: root, encoding: "utf8" });
}

const target =
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  capture("rustc", ["-vV"]).match(/^host: (.+)$/m)[1].trim();
const exe = target.includes("windows") ? ".exe" : "";
const targetDir = JSON.parse(
  capture("cargo", ["metadata", "--format-version", "1", "--no-deps"]),
).target_directory;

execFileSync(
  "cargo",
  ["build", "--release", "--target", target, "-p", "termlabd", "-p", "termlab-hook"],
  { cwd: root, stdio: "inherit" },
);

const out = join(desktop, "src-tauri", "binaries");
mkdirSync(out, { recursive: true });
for (const name of ["termlabd", "termlab-hook"]) {
  const built = join(targetDir, target, "release", name + exe);
  const placed = join(out, `${name}-${target}${exe}`);
  copyFileSync(built, placed);
  console.log(`${name}: ${placed}`);
}
