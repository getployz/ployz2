"use strict";
// Run against a real daemon: node node_tailcat.js <capability-file> [--uninitialized]
// Rung 2 locally; running the exact artifact on Cloud records runtime evidence.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const { setTimeout: delay } = require("node:timers/promises");
const sdk = require("..");
const capability = fs.readFileSync(process.argv[2], "utf8").trim();
const connections = [{ tailcat: capability }];
const ownHelpers = () => fs.readdirSync("/proc").filter((pid) => {
  if (!/^\d+$/.test(pid)) return false;
  try {
    const status = fs.readFileSync(`/proc/${pid}/status`, "utf8");
    if (!new RegExp(`^PPid:\\s+${process.pid}$`, "m").test(status)) return false;
    const args = fs.readFileSync(`/proc/${pid}/cmdline`, "utf8");
    assert.ok(!args.includes(capability), "capability escaped into child arguments");
    return args.includes("ployz-tailcat");
  } catch (error) { if (error.code === "ENOENT" || error.code === "ESRCH") return false; throw error; }
});
async function reaped(expected) {
  const until = Date.now() + 5000;
  while (ownHelpers().length !== expected && Date.now() < until) await delay(20);
  assert.equal(ownHelpers().length, expected, "helper lifecycle mismatch");
}
let stage = "validation";
async function main() {
  await reaped(0);
  await assert.rejects(sdk.connect({ connections: [{ tailcat: { capability } }] }),
    (error) => error instanceof sdk.RpcError && error.code === "invalid_argument" && !error.message.includes(capability));
  await assert.rejects(sdk.connect({ connections: [] }), (error) => error.code === "invalid_argument");
  const receipt = { node: process.version, platform: process.platform, arch: process.arch };
  const started = performance.now();
  const controller = new AbortController();
  stage = "cold connect";
  const first = await sdk.connect({ connections, signal: controller.signal, timeoutMs: 60000 });
  try {
    receipt.coldMs = Math.round(performance.now() - started);
    const warm = performance.now();
    stage = "warm read";
    const about = await first.about();
    receipt.warmReadMs = Math.round(performance.now() - warm);
    receipt.machineId = about.machine_id;
    receipt.daemonVersion = about.daemon_version;
    stage = "independent connect";
    const second = await sdk.connect({ connections, timeoutMs: 60000 });
    try {
      await reaped(2);
      stage = "stream";
      let retainedWatch;
      if (!process.argv.includes("--uninitialized")) {
        const stopWatch = new AbortController();
        const watch = first.runtime.watch({ signal: stopWatch.signal })[Symbol.asyncIterator]();
        const frame = await watch.next();
        assert.equal(frame.done, false);
        assert.ok(frame.value.observer_machine_id || frame.value.machines);
        stopWatch.abort();
        assert.equal((await watch.next()).done, true);
        receipt.streamFrameAndCancellation = true;
        retainedWatch = first.runtime.watch()[Symbol.asyncIterator]();
        assert.equal((await retainedWatch.next()).done, false);
      } else {
        receipt.streamFrameAndCancellation = "not exercised: daemon uninitialized";
      }
      stage = "cancel session";
      controller.abort();
      await reaped(1);
      if (retainedWatch) assert.equal((await retainedWatch.next()).done, true);
      assert.equal((await second.about()).machine_id, about.machine_id);
      receipt.independentSessionSurvivesCancellation = true;
      stage = "connection deadline";
      await assert.rejects(sdk.connect({ connections, timeoutMs: 1 }));
      await reaped(1);
      stage = "session deadline";
      const deadline = await sdk.connect({ connections, timeoutMs: 3000 });
      await delay(3100);
      await assert.rejects(deadline.about());
      await reaped(1);
      receipt.connectionAndSessionDeadlineReaped = true;
    } finally { await second.close(); }
  } finally { await first.close(); }
  await reaped(0);
  receipt.remainingHelpers = 0;
  console.log(JSON.stringify(receipt));
}
main().catch((error) => {
  // Never print transport causes or caller capability data in a receipt.
  console.error(JSON.stringify({ failed: true, stage, name: error.name, message: error.message.replaceAll(capability, "[redacted]") }));
  process.exitCode = 1;
});
