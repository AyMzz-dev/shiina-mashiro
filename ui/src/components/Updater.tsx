import { useCallback, useEffect, useRef, useState } from "react";
import { App, Button, Modal, Progress, Tag } from "antd";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

/** 定时巡检间隔：30 分钟 */
const CHECK_INTERVAL = 30 * 60 * 1000;

/**
 * 在线更新：Gitee 清单查新 → 弹窗显示更新日志 → 下载进度 → 装完重启。
 * manual=true 渲染「检查更新」按钮（关于页用）；manual=false 无头模式，
 * 挂载即静默查一次并每 30 分钟巡检，发现新版本直接弹全局更新窗。
 */
export function Updater({ manual = false }: { manual?: boolean }) {
  const { message } = App.useApp();
  const [upd, setUpd] = useState<Update | null>(null);
  const [pct, setPct] = useState(0);
  const [busy, setBusy] = useState(false);
  const [installed, setInstalled] = useState(false);
  const busyRef = useRef(false);

  const doCheck = useCallback(
    async (silent = false) => {
      if (busyRef.current) return;
      busyRef.current = true;
      setBusy(true);
      try {
        const u = await check();
        if (u) {
          setUpd(u);
        } else if (!silent) {
          message.success("已是最新版本");
        }
      } catch (e) {
        if (!silent) message.error(`检查更新失败：${e}`);
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
    },
    [message],
  );

  const doInstall = async () => {
    if (!upd || installed) return;
    busyRef.current = true;
    setBusy(true);
    try {
      let total = 0;
      let got = 0;
      await upd.downloadAndInstall((ev) => {
        if (ev.event === "Started") {
          total = ev.data.contentLength ?? 0;
        } else if (ev.event === "Progress") {
          got += ev.data.chunkLength;
          if (total) setPct(Math.min(99, Math.round((got / total) * 100)));
        } else if (ev.event === "Finished") {
          setPct(100);
        }
      });
      setInstalled(true);
      message.success("更新完成，重启后生效");
    } catch (e) {
      message.error(`更新失败：${e}`);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  // 启动静默查一次 + 每 30 分钟巡检（侧栏常驻一份，全包了）
  useEffect(() => {
    doCheck(true).catch(() => {});
    const t = setInterval(() => doCheck(true).catch(() => {}), CHECK_INTERVAL);
    return () => clearInterval(t);
  }, [doCheck]);

  return (
    <>
      {manual && (
        <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
          <Button
            size="small"
            type="primary"
            loading={busy && !upd}
            onClick={() => doCheck()}
          >
            检查更新
          </Button>
          {upd && <Tag color="orange">v{upd.version}</Tag>}
        </span>
      )}
      <Modal
        open={!!upd}
        title={`发现新版本 v${upd?.version ?? ""}`}
        okText={installed ? "重启生效" : "立即更新"}
        cancelText="稍后"
        confirmLoading={busy}
        onCancel={() => setUpd(null)}
        onOk={() => (installed ? relaunch() : doInstall())}
        maskClosable={false}
      >
        <pre
          style={{
            maxHeight: 260,
            overflow: "auto",
            whiteSpace: "pre-wrap",
            fontFamily: "inherit",
            fontSize: 12.5,
          }}
        >
          {upd?.body || "（无更新说明）"}
        </pre>
        {(busy || installed) && <Progress percent={pct} size="small" />}
      </Modal>
    </>
  );
}
