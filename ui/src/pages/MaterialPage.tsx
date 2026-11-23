import { useCallback, useEffect, useRef, useState } from "react";
import { App, Button, Empty, Modal, Popconfirm, Select, Tag } from "antd";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, type Account, type MaterialImage } from "../api";

/** 剩余天数：时间字段平台返回不稳定，多键名 + ttl 兜底（对齐旧版 timesOf）。*/
function daysLeft(img: MaterialImage): number {
  const pick = (keys: string[]) => {
    for (const k of keys) {
      const v = Number(img[k]);
      if (v > 0) return v;
    }
    return 0;
  };
  const created = pick(["created_at", "create_time", "created", "ctime"]);
  let expires = pick(["expires_at", "expire_time", "expires_time", "expireTime", "expire_at", "etime"]);
  const ttl = pick(["ttl", "expire_in", "expires_in"]);
  if (expires <= 0 && created > 0 && ttl > 0 && ttl < 1e8) expires = created + ttl;
  if (expires <= 0) return -1;
  return Math.floor((expires * 1000 - Date.now()) / 86400000);
}

const copy = (text: string, message: ReturnType<typeof App.useApp>["message"], tip: string) =>
  navigator.clipboard.writeText(text).then(
    () => message.success(tip),
    () => message.error("复制失败"),
  );

const IMG_RE = /\.(png|jpe?g|gif|webp|bmp|avif)$/i;

export default function MaterialPage() {
  const { message } = App.useApp();
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [appid, setAppid] = useState(0);
  const [images, setImages] = useState<MaterialImage[]>([]);
  const [cursor, setCursor] = useState("");
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [drag, setDrag] = useState(false);
  const [cur, setCur] = useState<MaterialImage | null>(null);
  const appidRef = useRef(0);
  appidRef.current = appid;

  useEffect(() => {
    api.listAccounts().then((accs) => {
      setAccounts(accs);
      if (accs.length > 0) setAppid((a) => (a === 0 ? accs[0].appid : a));
    }).catch(() => {});
  }, []);

  const load = useCallback(
    (cur_ = "", append = false) => {
      if (!appid) return;
      setLoading(true);
      api
        .materialList(appid, cur_)
        .then((r) => {
          setImages((old) => (append ? [...old, ...(r.images ?? [])] : (r.images ?? [])));
          setCursor(r.next_cursor ?? "");
          setTotal(r.total ?? r.images?.length ?? 0);
        })
        .catch((e) => message.error(String(e)))
        .finally(() => setLoading(false));
    },
    [appid, message],
  );

  useEffect(() => {
    setImages([]);
    load();
  }, [load]);

  const doUpload = useCallback(
    async (paths: string[]) => {
      const id = appidRef.current;
      if (!id) {
        message.warning("先选择机器人账号");
        return;
      }
      setUploading(true);
      let ok = 0;
      const errs: string[] = [];
      for (const p of paths) {
        try {
          await api.materialUpload(id, p);
          ok++;
        } catch (e) {
          errs.push(`${p.split(/[\\/]/).pop()}: ${e}`);
        }
      }
      setUploading(false);
      if (ok > 0) message.success(`上传成功 ${ok} 张`);
      errs.forEach((e) => message.error(e, 6));
      load();
    },
    [load, message],
  );

  const upload = async () => {
    const picked = await open({
      multiple: true,
      filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif"] }],
    });
    if (!picked) return;
    doUpload(Array.isArray(picked) ? picked : [picked]);
  };

  // 拖拽上传：Tauri 文件拖入事件
  useEffect(() => {
    let un: (() => void) | undefined;
    getCurrentWindow()
      .onDragDropEvent((ev) => {
        const p = ev.payload;
        if (p.type === "enter" || p.type === "over") setDrag(true);
        else if (p.type === "leave") setDrag(false);
        else if (p.type === "drop") {
          setDrag(false);
          const imgs = p.paths.filter((x) => IMG_RE.test(x));
          if (imgs.length > 0) doUpload(imgs);
          else if (p.paths.length > 0) message.warning("拖入的不是图片文件");
        }
      })
      .then((u) => (un = u));
    return () => un?.();
  }, [doUpload, message]);

  const del = async (img: MaterialImage) => {
    try {
      await api.materialDelete(appid, img.image_id);
      setImages((old) => old.filter((x) => x.image_id !== img.image_id));
      setCur(null);
      message.success("已删除");
    } catch (e) {
      message.error(String(e));
    }
  };

  const nameOf = (img: MaterialImage) => img.file_name || img.image_id;
  const mdOf = (img: MaterialImage) => `![${(nameOf(img) || "image").replace(/\.[^.]+$/, "")}](${img.url})`;

  return (
    <div className={`mat-page${drag ? " dragging" : ""}`}>
      <h2 className="page-title">素材</h2>
      <div className="card">
        <div className="mat-head">
          <Select
            size="small"
            style={{ width: 180 }}
            value={appid || undefined}
            onChange={setAppid}
            options={accounts.map((a) => ({
              label: `${a.name || a.appid}`,
              value: a.appid,
            }))}
            placeholder="选择机器人"
          />
          <span className="home-pill">共 {total} 张</span>
          <span className="mat-sub">配额约 100 张 / 90 天有效 · 引用后 Markdown 发图 · 支持拖拽上传</span>
          <div className="spacer" />
          <Button size="small" type="primary" loading={uploading} onClick={upload} disabled={!appid}>
            上传图片
          </Button>
          <Button size="small" loading={loading} onClick={() => load()} disabled={!appid}>
            刷新
          </Button>
        </div>

        {!appid ? (
          <Empty description="先在账号页添加并启动机器人" />
        ) : images.length === 0 && loading ? (
          <div className="mat-grid">
            {Array.from({ length: 8 }).map((_, i) => (
              <div className="mat-item skel" key={i}>
                <div className="mat-thumb" />
                <div className="mat-name skel-line" />
              </div>
            ))}
          </div>
        ) : images.length === 0 ? (
          <Empty description="暂无素材，点「上传图片」或直接把图片拖进来" />
        ) : (
          <>
            <div className="mat-grid">
              {images.map((img) => {
                const left = daysLeft(img);
                return (
                  <div className="mat-item" key={img.image_id} onClick={() => setCur(img)}>
                    <div className="mat-thumb">
                      {img.preview_url ? (
                        <img src={img.preview_url} alt={nameOf(img)} loading="lazy" />
                      ) : (
                        <div className="mat-noimg">无预览</div>
                      )}
                      {left >= 0 && (
                        <span className={`mat-days${left <= 7 ? " warn" : ""}`}>剩 {left} 天</span>
                      )}
                      <div
                        className="mat-overlay"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <button
                          className="mat-act"
                          onClick={() => copy(img.url, message, "引用链接已复制")}
                        >
                          链接
                        </button>
                        <button
                          className="mat-act"
                          onClick={() => copy(mdOf(img), message, "Markdown 已复制")}
                        >
                          MD
                        </button>
                      </div>
                    </div>
                    <div className="mat-name" title={nameOf(img)}>
                      {nameOf(img)}
                    </div>
                  </div>
                );
              })}
            </div>
            {cursor && (
              <div className="mat-more">
                <Button size="small" loading={loading} onClick={() => load(cursor, true)}>
                  加载更多
                </Button>
              </div>
            )}
          </>
        )}
      </div>

      {/* 拖拽遮罩 */}
      {drag && (
        <div className="mat-dropmask">
          <div className="mat-dropbox">松开鼠标上传图片</div>
        </div>
      )}

      <Modal
        open={!!cur}
        onCancel={() => setCur(null)}
        footer={null}
        width={560}
        title={cur ? nameOf(cur) : ""}
      >
        {cur && (
          <div className="mat-detail">
            {cur.preview_url && <img className="mat-big" src={cur.preview_url} alt="" />}
            <div className="mat-url mono">{cur.url}</div>
            <div className="mat-ops">
              <Button size="small" onClick={() => copy(cur.url, message, "引用链接已复制")}>
                复制引用链接
              </Button>
              <Button size="small" onClick={() => copy(mdOf(cur), message, "Markdown 已复制")}>
                复制 Markdown
              </Button>
              <Popconfirm title="删除后引用链接立即失效" onConfirm={() => del(cur)}>
                <Button size="small" danger>
                  删除
                </Button>
              </Popconfirm>
              <Tag style={{ marginLeft: "auto" }}>
                {daysLeft(cur) >= 0 ? `剩 ${daysLeft(cur)} 天` : "有效期未知"}
              </Tag>
            </div>
          </div>
        )}
      </Modal>
    </div>
  );
}
