/**
 * Seed Tauri's NSIS cache so `tauri bundle` skips ureq's one-shot GitHub
 * download (common CI failure: `io: Peer disconnected` on nsis-3.11.zip).
 *
 * URLs/hashes match @tauri-apps/cli 2.11.x:
 * https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs
 */
import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const NSIS_URL =
  "https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip";
const NSIS_SHA1 = "EF7FF767E5CBD9EDD22ADD3A32C9B8F4500BB10D";
const UTILS_URL =
  "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll";
const UTILS_SHA1 = "75197FEE3C6A814FE035788D1C34EAD39349B860";

const REQUIRED_RELATIVE = [
  "makensis.exe",
  "Bin/makensis.exe",
  "Stubs/lzma-x86-unicode",
  "Include/MUI2.nsh",
  "Include/Win/COM.nsh",
];

function toolsDir() {
  const local = process.env.LOCALAPPDATA;
  if (!local) {
    throw new Error("LOCALAPPDATA is not set; this script is Windows-only");
  }
  return path.join(local, "tauri");
}

function sha1File(file) {
  return crypto.createHash("sha1").update(fs.readFileSync(file)).digest("hex");
}

function assertSha1(file, expected) {
  const actual = sha1File(file).toUpperCase();
  if (actual !== expected.toUpperCase()) {
    throw new Error(`SHA1 mismatch for ${file}: ${actual} != ${expected}`);
  }
}

async function download(url, dest, expectedSha1) {
  const headers = {};
  if (process.env.GITHUB_TOKEN) {
    headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  }
  let lastErr;
  for (let attempt = 1; attempt <= 6; attempt++) {
    try {
      const res = await fetch(url, { headers, redirect: "follow" });
      if (!res.ok) {
        throw new Error(`${url} -> HTTP ${res.status}`);
      }
      const buf = Buffer.from(await res.arrayBuffer());
      const actual = crypto.createHash("sha1").update(buf).digest("hex");
      if (actual.toUpperCase() !== expectedSha1.toUpperCase()) {
        throw new Error(`SHA1 mismatch for ${url}: ${actual} != ${expectedSha1}`);
      }
      fs.mkdirSync(path.dirname(dest), { recursive: true });
      fs.writeFileSync(dest, buf);
      return;
    } catch (err) {
      lastErr = err;
      const wait = Math.min(30, 2 ** attempt);
      console.warn(`download attempt ${attempt} failed: ${err}. retry in ${wait}s`);
      await new Promise((r) => setTimeout(r, wait * 1000));
    }
  }
  throw lastErr;
}

function nsisComplete(nsisDir) {
  return REQUIRED_RELATIVE.every((rel) => fs.existsSync(path.join(nsisDir, rel)));
}

function utilsPath(nsisDir) {
  return path.join(nsisDir, "Plugins", "x86-unicode", "additional", "nsis_tauri_utils.dll");
}

function utilsOk(nsisDir) {
  const file = utilsPath(nsisDir);
  if (!fs.existsSync(file)) return false;
  try {
    assertSha1(file, UTILS_SHA1);
    return true;
  } catch {
    return false;
  }
}

const nsisDir = path.join(toolsDir(), "NSIS");
fs.mkdirSync(toolsDir(), { recursive: true });

if (!nsisComplete(nsisDir)) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "qmonitor-nsis-"));
  const zip = path.join(tmp, "nsis-3.11.zip");
  await download(NSIS_URL, zip, NSIS_SHA1);
  execFileSync("tar", ["-xf", zip, "-C", tmp], { stdio: "inherit" });
  const extracted = path.join(tmp, "nsis-3.11");
  if (!fs.existsSync(extracted)) {
    throw new Error(`expected ${extracted} after extracting NSIS zip`);
  }
  fs.rmSync(nsisDir, { recursive: true, force: true });
  fs.cpSync(extracted, nsisDir, { recursive: true });
  fs.rmSync(tmp, { recursive: true, force: true });
  console.log(`extracted NSIS to ${nsisDir}`);
} else {
  console.log(`NSIS already present at ${nsisDir}`);
}

if (!utilsOk(nsisDir)) {
  const dest = utilsPath(nsisDir);
  await download(UTILS_URL, dest, UTILS_SHA1);
  console.log(`wrote ${dest}`);
} else {
  console.log("nsis_tauri_utils.dll already present");
}
