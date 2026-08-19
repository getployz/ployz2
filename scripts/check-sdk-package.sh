#!/usr/bin/env bash

set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

cargo build -p ployz-sdk --locked

if [ -f target/debug/libployz_sdk.so ]; then
    cp target/debug/libployz_sdk.so ployz-sdk/ployz-sdk.node
elif [ -f target/debug/libployz_sdk.dylib ]; then
    cp target/debug/libployz_sdk.dylib ployz-sdk/ployz-sdk.node
elif [ -f target/debug/ployz_sdk.dll ]; then
    cp target/debug/ployz_sdk.dll ployz-sdk/ployz-sdk.node
else
    echo "ployz-sdk cdylib was not produced" >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/node_modules/@ployz"
ln -s "$ROOT/ployz-sdk" "$TMP/node_modules/@ployz/sdk"

cd "$TMP"
node --input-type=commonjs <<'EOF'
const fs = require("node:fs");
const path = require("node:path");
const pkg = require("@ployz/sdk/package.json");
if (pkg.name !== "@ployz/sdk") {
  throw new Error(`expected npm name @ployz/sdk, got ${pkg.name}`);
}
if (pkg.private === true) {
  throw new Error("@ployz/sdk must be publishable");
}
if (!pkg.publishConfig || pkg.publishConfig.access !== "public") {
  throw new Error("@ployz/sdk must publish as a public scoped package");
}

const rootExport = pkg.exports && pkg.exports["."];
if (!rootExport || typeof rootExport !== "object") {
  throw new Error('package.json exports["."] is missing');
}
if (rootExport.types !== "./index.d.ts") {
  throw new Error(`types export must be ./index.d.ts, got ${rootExport.types}`);
}
if (rootExport.node !== "./index.js") {
  throw new Error(`node export must be ./index.js, got ${rootExport.node}`);
}
if (rootExport.browser !== "./browser.mjs") {
  throw new Error(`browser export must be ./browser.mjs, got ${rootExport.browser}`);
}
const pkgDir = path.dirname(require.resolve("@ployz/sdk/package.json"));
const browserPath = path.join(pkgDir, rootExport.browser);
if (!fs.existsSync(browserPath)) {
  throw new Error("browser.mjs is missing");
}

const sdk = require("@ployz/sdk");
if (typeof sdk.packageName !== "function") {
  throw new Error("packageName export is missing");
}
if (sdk.packageName() !== "@ployz/sdk") {
  throw new Error(`expected packageName() to return @ployz/sdk, got ${sdk.packageName()}`);
}
if (typeof sdk.connect !== "function") {
  throw new Error("connect export is missing");
}
if (typeof sdk.Client !== "function") {
  throw new Error("Client export is missing");
}

const forbidden = [
  "call",
  "request",
  "watch",
  "preview",
  "deploy",
  "connectTcp",
  "connectSsh",
  "connectUnix",
  "connectSSH",
];
for (const name of forbidden) {
  if (Object.hasOwn(sdk, name)) {
    throw new Error(`${name} must not be exported`);
  }
}

const dts = fs.readFileSync(require.resolve("@ployz/sdk/index.d.ts"), "utf8");
if (!dts.includes("export declare function connect")) {
  throw new Error("index.d.ts is missing connect");
}
if (!dts.includes("about(): Promise<ContractDescription>")) {
  throw new Error("index.d.ts is missing about()");
}
if (!dts.includes("preview(intent: DeployIntent): Promise<PreparedDeploy>")) {
  throw new Error("index.d.ts is missing preview()");
}
if (!dts.includes("run(") || !dts.includes("Promise<DeployOutcome<ExecutionError>>")) {
  throw new Error("index.d.ts is missing run()");
}
if (dts.includes("deploy(intent: DeployIntent)")) {
  throw new Error("index.d.ts must not declare unary deploy()");
}
if (!dts.includes("removeVolumes(")) {
  throw new Error("index.d.ts is missing removeVolumes()");
}
if (!dts.includes("RemoveVolumesRequest")) {
  throw new Error("index.d.ts is missing RemoveVolumesRequest");
}
if (!dts.includes("dataLossIfMachineRemoved(machine: MachineTarget): Promise<ObservedDataLoss>")) {
  throw new Error("index.d.ts is missing dataLossIfMachineRemoved()");
}
if (!dts.includes("close(): Promise<void>")) {
  throw new Error("index.d.ts is missing close()");
}
if (!dts.includes("readonly runtime:")) {
  throw new Error("index.d.ts is missing Client.runtime");
}
if (!dts.includes("watch(options?: WatchOptions): AsyncIterable<RuntimeWatchFrame>")) {
  throw new Error("index.d.ts is missing runtime.watch()");
}
for (const needle of ["connectSsh", "connectTcp", "connectUnix", "watchRuntime"]) {
  if (dts.includes(needle)) {
    throw new Error(`index.d.ts must not declare ${needle}`);
  }
}
if (typeof sdk.Client.prototype.preview !== "function") {
  throw new Error("Client.preview must be a method");
}
if (typeof sdk.Client.prototype.run !== "function") {
  throw new Error("Client.run must be a method");
}
if (typeof sdk.Client.prototype.deploy !== "undefined") {
  throw new Error("Client.deploy must not exist");
}
if (typeof sdk.applyAll !== "function" || typeof sdk.applyOne !== "function") {
  throw new Error("applyAll / applyOne must be exported");
}
if (typeof sdk.Client.prototype.removeVolumes !== "function") {
  throw new Error("Client.removeVolumes must be a method");
}
if (typeof sdk.Client.prototype.dataLossIfMachineRemoved !== "function") {
  throw new Error("Client.dataLossIfMachineRemoved must be a method");
}

async function expectRpc(fn, code) {
  try {
    await fn();
    throw new Error(`expected ${code}`);
  } catch (error) {
    let rpc;
    try {
      rpc = JSON.parse(error.message);
    } catch {
      throw new Error(`expected generated RpcError JSON, got ${error && error.message}`);
    }
    if (rpc.code !== code) {
      throw new Error(`expected ${code}, got ${rpc.code}`);
    }
  }
}

(async () => {
  await expectRpc(
    () =>
      sdk.connect({
        relayUrl: "http://127.0.0.1:1",
        bearer: "",
        pairing: "pairing-secret",
        machineId: "0123456789abcdef0123456789abcdef",
      }),
    "unauthenticated",
  );
  await expectRpc(
    () =>
      sdk.connect({
        relayUrl: "http://127.0.0.1:1",
        bearer: "dial-secret",
        pairing: "pairing-secret",
        machineId: "not-a-machine-id",
      }),
    "invalid_argument",
  );
  console.log(`loaded ${path.join("@ployz", "sdk")} ${pkg.version}`);
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
EOF

# Nitro/Vinxi rewrite CJS as ESM with createRequire and no __dirname polyfill.
# Railway SSR loads that wrap; __dirname must not appear in the loader.
ROOT="$ROOT" node --input-type=module <<'EOF'
import { copyFileSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const root = process.env.ROOT;
const src = readFileSync(join(root, "ployz-sdk/index.js"), "utf8");
const body = src
  .replace(/^"use strict";\r?\n/, "")
  .replace(/^module\.exports = /m, "export default ");
const dir = mkdtempSync(join(tmpdir(), "ployz-sdk-esm-"));
writeFileSync(
  join(dir, "ployz__sdk.mjs"),
  `import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
${body}
`,
);
copyFileSync(join(root, "ployz-sdk/ployz-sdk.node"), join(dir, "ployz-sdk.node"));

let sdk;
try {
  sdk = (await import(pathToFileURL(join(dir, "ployz__sdk.mjs")).href)).default;
} catch (error) {
  if (error instanceof ReferenceError && String(error.message).includes("__dirname")) {
    throw new Error(
      "@ployz/sdk must load after a Nitro-style ESM wrap (__dirname is not defined there)",
    );
  }
  throw error;
}
if (typeof sdk.packageName !== "function" || sdk.packageName() !== "@ployz/sdk") {
  throw new Error("esm wrap must load the native binding");
}
console.log("esm wrap loaded");
EOF

cd "$ROOT/ployz-sdk"
if [ ! -d node_modules/typescript ]; then
  npm ci --ignore-scripts
fi
npx --no-install tsc --noEmit
