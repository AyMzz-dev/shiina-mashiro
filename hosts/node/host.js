#!/usr/bin/env node
/**
 * Mashiro JS 插件宿主。
 *
 * 与框架之间走 stdin/stdout NDJSON（一行一个 JSON，见 src-tauri/src/plugin/proto.rs）。
 * 每个 JS 插件跑在一个 Worker 线程里做隔离；
 * 插件调框架 API = 反向请求，经宿主转发回框架。
 */
"use strict";

const readline = require("node:readline");
const { Worker } = require("node:worker_threads");
const path = require("node:path");

const plugins = new Map(); // id -> { worker }
const describePending = new Map(); // reqId -> resolve
let describeReqId = 1;

function send(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function respond(id, result, error) {
  send({ id, result: result === undefined ? null : result, error: error || null });
}

// 插件 Worker 的 API 调用 → 转发给框架，拿到回包后再投回 Worker
const apiPending = new Map(); // reqId -> { worker, localId }
let apiReqId = 1;

function handleWorkerMessage(worker, msg) {
  if (msg.api) {
    const rid = apiReqId++;
    apiPending.set(rid, { worker, localId: msg.api.id });
    send({
      id: rid,
      method: msg.api.method,
      params: msg.api.params,
      plugin: worker.__pluginId,
    });
  } else if (msg.describe) {
    const res = describePending.get(msg.describe.key);
    if (res) {
      describePending.delete(msg.describe.key);
      res(msg.describe.handlers);
    }
  }
}

function loadPlugin({ id, dir, entry }) {
  if (plugins.has(id)) unloadPlugin({ id });
  const worker = new Worker(path.join(__dirname, "worker.js"), {
    workerData: { id, dir, entry },
  });
  worker.__pluginId = id;
  worker.on("message", (m) => handleWorkerMessage(worker, m));
  worker.on("error", (e) => console.error(`[host] plugin ${id} error:`, e));
  worker.on("exit", (code) => {
    plugins.delete(id);
    if (code !== 0) console.error(`[host] plugin ${id} exited ${code}`);
  });
  plugins.set(id, { worker });
  return { ok: true };
}

function unloadPlugin({ id }) {
  const p = plugins.get(id);
  if (p) p.worker.terminate();
  plugins.delete(id);
  return { ok: true };
}

const methods = {
  ping: () => "pong",
  load_plugin: loadPlugin,
  unload_plugin: unloadPlugin,
  // 返回插件已注册的 handler 清单（cmd:xxx / event:xxx）
  describe: ({ plugin }) =>
    new Promise((resolve, reject) => {
      const p = plugins.get(plugin);
      if (!p) return reject(new Error(`plugin ${plugin} not loaded`));
      const key = describeReqId++;
      describePending.set(key, (handlers) => resolve({ handlers }));
      p.worker.postMessage({ describe: key });
      setTimeout(() => {
        if (describePending.has(key)) {
          describePending.delete(key);
          reject(new Error("describe timeout"));
        }
      }, 3000);
    }),
  invoke: ({ plugin, handler, event }) => {
    const p = plugins.get(plugin);
    if (!p) throw new Error(`plugin ${plugin} not loaded`);
    p.worker.postMessage({ invoke: { handler, event } });
    return { accepted: true };
  },
};

const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }

  // 框架回包（插件 API 调用的结果）→ 投回对应 Worker
  if (msg.id !== undefined && msg.method === undefined && apiPending.has(msg.id)) {
    const { worker, localId } = apiPending.get(msg.id);
    apiPending.delete(msg.id);
    worker.postMessage({
      apiResult: { id: localId, result: msg.result, error: msg.error },
    });
    return;
  }

  // 框架请求
  if (msg.method && msg.id !== undefined) {
    const fn = methods[msg.method];
    if (!fn) {
      respond(msg.id, null, `method ${msg.method} not found`);
      return;
    }
    Promise.resolve()
      .then(() => fn(msg.params || {}))
      .then((r) => respond(msg.id, r, null))
      .catch((e) => respond(msg.id, null, String(e && e.message ? e.message : e)));
    return;
  }

  // 框架通知
  if (msg.type === "shutdown") {
    for (const [, p] of plugins) p.worker.terminate();
    process.exit(0);
  }
});
