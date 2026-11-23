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
const readyPending = new Map(); // worker -> {resolve,reject} load 就绪握手
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
      token: worker.__pluginToken,
    });
  } else if (msg.ready !== undefined) {
    // worker 入口加载完成（成功或报错）→ 放行 load_plugin 请求方
    const res = readyPending.get(worker);
    if (res) {
      readyPending.delete(worker);
      msg.ready ? res.resolve() : res.reject(new Error(msg.error || "plugin load failed"));
    }
  } else if (msg.describe) {
    const res = describePending.get(msg.describe.key);
    if (res) {
      describePending.delete(msg.describe.key);
      res(msg.describe.handlers);
    }
  }
}

function loadPlugin({ id, dir, entry, token }) {
  if (plugins.has(id)) unloadPlugin({ id });
  const worker = new Worker(path.join(__dirname, "worker.js"), {
    workerData: { id, dir, entry },
    // stdout/stderr 独立流：插件自己写 stdout（或 fs.writeSync(1) 之外的输出）
    // 不会再糊进框架协议管道；输出转到宿主 stderr 便于排错
    stdout: true,
    stderr: true,
  });
  worker.stdout.on("data", (d) => process.stderr.write(`[${id}] ${d}`));
  worker.stderr.on("data", (d) => process.stderr.write(`[${id}!] ${d}`));
  worker.__pluginId = id;
  worker.__pluginToken = token || "";
  worker.on("message", (m) => handleWorkerMessage(worker, m));
  worker.on("error", (e) => console.error(`[host] plugin ${id} error:`, e));
  worker.on("exit", (code) => {
    plugins.delete(id);
    for (const [rid, p] of apiPending) if (p.worker === worker) apiPending.delete(rid);
    if (code !== 0) console.error(`[host] plugin ${id} exited ${code}`);
  });
  plugins.set(id, { worker });
  // 等 worker 入口文件 require 完的 ready 握手，才算「真实加载」
  return new Promise((resolve, reject) => {
    readyPending.set(worker, { resolve, reject });
    setTimeout(() => {
      if (readyPending.delete(worker)) reject(new Error("plugin load timeout"));
    }, 10000);
  }).then(() => {
    plugins.set(id, { worker });
    return { ok: true };
  });
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
  // 一条消息命中多个插件：单帧扇出，事件只传一次（items 逐个分发）
  invoke_batch: ({ items, event }) => {
    let delivered = 0;
    for (const it of items || []) {
      const p = plugins.get(it.plugin);
      if (!p) {
        console.error(`[host] invoke_batch: ${it.plugin} not loaded`);
        continue;
      }
      p.worker.postMessage({ invoke: { handler: it.handler, event } });
      delivered++;
    }
    return { delivered };
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
