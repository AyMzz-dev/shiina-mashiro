"use strict";
/**
 * 插件 Worker 引导：由 host.js spawn，workerData = { id, dir, entry }。
 * 给插件提供全局 mashiro 传输层（SDK sdk/js/mashiro.js 直接 require 用），
 * 再加载插件入口文件。
 */
const { parentPort, workerData } = require("node:worker_threads");
const path = require("node:path");

let apiReqId = 1;
const pending = new Map();
const handlers = new Map(); // "cmd:xxx" / "event:xxx" -> fn

// 插件侧传输层：mashiro.js SDK 通过 require 时注入的这个对象和宿主通信
globalThis.__mashiro_transport = {
  callApi(method, params) {
    const id = apiReqId++;
    parentPort.postMessage({ api: { id, method, params } });
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      setTimeout(() => {
        if (pending.has(id)) {
          pending.delete(id);
          reject(new Error(`api ${method} timeout`));
        }
      }, 15000);
    });
  },
  register(handler, fn) {
    handlers.set(handler, fn);
  },
};

parentPort.on("message", (msg) => {
  if (msg.apiResult) {
    const p = pending.get(msg.apiResult.id);
    if (p) {
      pending.delete(msg.apiResult.id);
      msg.apiResult.error
        ? p.reject(new Error(msg.apiResult.error))
        : p.resolve(msg.apiResult.result);
    }
  } else if (msg.describe) {
    parentPort.postMessage({
      describe: { key: msg.describe, handlers: [...handlers.keys()] },
    });
  } else if (msg.invoke) {
    const fn = handlers.get(msg.invoke.handler);
    if (fn) {
      const report = (e) =>
        parentPort.postMessage({
          api: { id: apiReqId++, method: "log", params: { level: "error", msg: String(e) } },
        });
      try {
        Promise.resolve(fn(msg.invoke.event)).catch(report);
      } catch (e) {
        report(e);
      }
    }
  }
});

// SDK 路径：require("mashiro") 重定向到 sdk/js/mashiro.js（不用装 node_modules）
const Module = require("node:module");
const sdkPath = process.env.MASHIRO_SDK || path.resolve(__dirname, "../../sdk/js/mashiro.js");
const origResolve = Module._resolveFilename;
Module._resolveFilename = function (request, ...args) {
  if (request === "mashiro") return sdkPath;
  return origResolve.call(this, request, ...args);
};

const pluginEntry = path.join(workerData.dir, workerData.entry);
require(pluginEntry);
