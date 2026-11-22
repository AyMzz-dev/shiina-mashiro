/**
 * 宿主协议冒烟测试（不经 Rust，直接驱动 host 子进程）。
 * 用法: node tests/host-smoke.js [node|python]
 *
 * 模拟框架侧：发 ping → load_plugin → invoke cmd:签到，
 * 插件执行里调的 kv_incr / send_message 由本脚本应答。
 */
"use strict";

const { spawn } = require("node:child_process");
const path = require("node:path");
const readline = require("node:readline");

const runtime = process.argv[2] || "node";
const [cmd, script, pluginDir, entry] =
  runtime === "python"
    ? ["python", "hosts/python/host.py", "hello-py", "main.py"]
    : ["node", "hosts/node/host.js", "hello-js", "main.js"];

const root = path.resolve(__dirname, "..");
const host = spawn(cmd, [path.join(root, script)], {
  stdio: ["pipe", "pipe", "inherit"],
  cwd: root,
});
const rl = readline.createInterface({ input: host.stdout });

const pending = new Map();
let nextId = 1;
const req = (method, params) =>
  new Promise((res, rej) => {
    const id = nextId++;
    pending.set(id, { res, rej });
    host.stdin.write(JSON.stringify({ id, method, params }) + "\n");
    setTimeout(() => pending.has(id) && (pending.delete(id), rej(new Error("timeout"))), 3000);
  });

rl.on("line", (line) => {
  const m = JSON.parse(line);
  if (m.method) {
    // 插件调框架 API → 应答
    const result = m.method === "kv_incr" ? 42 : { ok: true };
    console.log(`  api→ ${m.method} → respond ${JSON.stringify(result)}`);
    host.stdin.write(JSON.stringify({ id: m.id, result, error: null }) + "\n");
  } else {
    const p = pending.get(m.id);
    if (p) {
      pending.delete(m.id);
      p.res(m.error ? Promise.reject(m.error) : m.result);
    }
  }
});

(async () => {
  try {
    console.log(`[${runtime}] ping:`, await req("ping", {}));
    console.log(`[${runtime}] load:`,
      await req("load_plugin", { id: pluginDir, dir: path.join(root, "plugins", pluginDir), entry }));
    console.log(`[${runtime}] describe:`,
      await req("describe", { plugin: pluginDir }));
    console.log(`[${runtime}] invoke:`,
      await req("invoke", {
        plugin: pluginDir,
        handler: "cmd:签到",
        event: { appid: 1, kind: 0, group_id: "G1", user: "U1", text: "签到", msg_id: "m1" },
      }));
    await new Promise((r) => setTimeout(r, 600)); // 等插件异步跑完
    host.stdin.write(JSON.stringify({ type: "shutdown" }) + "\n");
    console.log(`[${runtime}] OK`);
    process.exit(0);
  } catch (e) {
    console.error(`[${runtime}] FAIL:`, e);
    host.kill();
    process.exit(1);
  }
})();
