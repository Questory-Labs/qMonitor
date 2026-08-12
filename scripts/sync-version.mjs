/**
 * Keep Cargo.toml + tauri.conf.json in sync with package.json.
 * Usage:
 *   node scripts/sync-version.mjs           # read version from package.json
 *   node scripts/sync-version.mjs 1.2.3     # set package.json too, then sync
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pkgPath = path.join(root, "package.json");
const tauriPath = path.join(root, "src-tauri", "tauri.conf.json");
const cargoPath = path.join(root, "src-tauri", "Cargo.toml");

const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
const version = process.argv[2] ?? pkg.version;

if (!version || typeof version !== "string") {
  console.error("sync-version: missing version");
  process.exit(1);
}

if (pkg.version !== version) {
  pkg.version = version;
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
}

const tauri = JSON.parse(fs.readFileSync(tauriPath, "utf8"));
if (tauri.version !== version) {
  tauri.version = version;
  fs.writeFileSync(tauriPath, JSON.stringify(tauri, null, 2) + "\n");
}

let cargo = fs.readFileSync(cargoPath, "utf8");
const nextCargo = cargo.replace(/^version\s*=\s*"[^"]*"/m, `version = "${version}"`);
if (nextCargo === cargo && !/^version\s*=\s*"/m.test(cargo)) {
  console.error("sync-version: could not find version in Cargo.toml");
  process.exit(1);
}
if (nextCargo !== cargo) {
  fs.writeFileSync(cargoPath, nextCargo);
}

console.log(`sync-version: ${version}`);
