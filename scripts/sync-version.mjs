/**
 * Keep Cargo.toml + tauri.conf.json in sync with package.json.
 * Usage:
 *   node scripts/sync-version.mjs           # read version from package.json
 *   node scripts/sync-version.mjs 1.2.3     # set package.json too, then sync
 *
 * Filenames always use the full semver (e.g. 0.0.1-canary.e5b0094).
 * WiX/MSI ProductVersion cannot encode a non-numeric prerelease, so canary
 * sets bundle.windows.wix.version to the core tag (0.0.1).
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

function coreVersion(v) {
  const m = /^(\d+)\.(\d+)\.(\d+)/.exec(v);
  if (!m) {
    console.error(`sync-version: cannot parse major.minor.patch from ${v}`);
    process.exit(1);
  }
  return `${m[1]}.${m[2]}.${m[3]}`;
}

/** True when Tauri's WiX convert_version() would reject this semver. */
function needsWixOverride(v) {
  const m = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$/.exec(v);
  if (!m) return true;
  const optionalNumeric = (s) => s == null || (/^\d+$/.test(s) && Number(s) <= 65535);
  return !optionalNumeric(m[4]) || !optionalNumeric(m[5]);
}

if (pkg.version !== version) {
  pkg.version = version;
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
}

const tauri = JSON.parse(fs.readFileSync(tauriPath, "utf8"));
tauri.version = version;
tauri.bundle = tauri.bundle ?? {};
tauri.bundle.windows = tauri.bundle.windows ?? {};

const wixTag = coreVersion(version);
if (needsWixOverride(version)) {
  tauri.bundle.windows.wix = { ...(tauri.bundle.windows.wix ?? {}), version: wixTag };
} else if (tauri.bundle.windows.wix) {
  delete tauri.bundle.windows.wix.version;
  if (Object.keys(tauri.bundle.windows.wix).length === 0) {
    delete tauri.bundle.windows.wix;
  }
}

fs.writeFileSync(tauriPath, JSON.stringify(tauri, null, 2) + "\n");

let cargo = fs.readFileSync(cargoPath, "utf8");
const nextCargo = cargo.replace(/^version\s*=\s*"[^"]*"/m, `version = "${version}"`);
if (nextCargo === cargo && !/^version\s*=\s*"/m.test(cargo)) {
  console.error("sync-version: could not find version in Cargo.toml");
  process.exit(1);
}
if (nextCargo !== cargo) {
  fs.writeFileSync(cargoPath, nextCargo);
}

if (needsWixOverride(version)) {
  console.log(`sync-version: ${version} (wix.version=${wixTag})`);
} else {
  console.log(`sync-version: ${version}`);
}
