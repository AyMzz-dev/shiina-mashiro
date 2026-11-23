# -*- coding: utf-8 -*-
"""发版后本地跑：GitHub Release 产物 → Gitee 发行版附件，并重写 latest.json。

海外 CI 推不动 Gitee 大包（实测挂死），所以二进制搬运放在国内本机做：
  python scripts/sync-gitee.py <tag> [gitee_token]

- 下载 GitHub Release 的更新产物（exe/AppImage/app.tar.gz + .sig）
- 在 Gitee 更新仓创建发行版并传附件
- latest.json 的各平台 URL 改写为 Gitee 附件地址，经 Contents API 推送
"""
import base64
import io
import json
import os
import re
import sys
import urllib.request
import uuid

GH_REPO = "MoonSeaFishCat/shiina-mashiro"
GITEE_REPO = "isluoxiaoxiwork/shiina-mashiro-update"
GITEE_API = "https://gitee.com/api/v5"
# 参与更新的产物名模式（安装器/更新包本体，不含 .sig）
ART_RE = re.compile(r"(-setup\.exe$|\.AppImage$|\.app\.tar\.gz$)")


def get(url, token=None, binary=False):
    req = urllib.request.Request(url)
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    r = urllib.request.urlopen(req, timeout=120)
    return r.read() if binary else json.load(r)


def post(url, data=None, fields=None):
    if fields is not None:  # multipart 附件上传
        boundary = uuid.uuid4().hex
        buf = io.BytesIO()
        for k, v in fields.items():
            if k == "file":
                name, blob = v
                buf.write(f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{name}"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode())
                buf.write(blob)
                buf.write(b"\r\n")
            else:
                buf.write(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode())
        buf.write(f"--{boundary}--\r\n".encode())
        req = urllib.request.Request(url, data=buf.getvalue())
        req.add_header("Content-Type", f"multipart/form-data; boundary={boundary}")
        return json.load(urllib.request.urlopen(req, timeout=600))
    req = urllib.request.Request(url, data=json.dumps(data).encode(), method="POST")
    return json.load(urllib.request.urlopen(req, timeout=60))


def main():
    tag = sys.argv[1]
    token = sys.argv[2] if len(sys.argv) > 2 else os.environ["GITEE_TOKEN"]
    gh_tok = os.environ.get("GH_TOKEN")

    rel = get(f"https://api.github.com/repos/{GH_REPO}/releases/tags/{tag}", gh_tok)
    assets = {a["name"]: a["browser_download_url"] for a in rel["assets"]}
    arts = [n for n in assets if ART_RE.search(n) and not n.endswith(".sig")]
    if not arts:
        sys.exit("release 里没有更新产物")
    print("产物:", arts)

    # 建 Gitee 发行版（已存在则复用）
    rels = get(f"{GITEE_API}/repos/{GITEE_REPO}/releases/tag/{tag}?access_token={token}")
    rid = rels.get("id") if isinstance(rels, dict) else None
    if not rid:
        rels = post(f"{GITEE_API}/repos/{GITEE_REPO}/releases",
                    {"access_token": token, "tag_name": tag, "name": tag,
                     "body": "ShiinaMashiro 更新附件仓", "target_commitish": "main"})
        rid = rels["id"]
    print("gitee release id:", rid)

    # 逐产物：下载 → 传附件 → 记录下载地址
    att_urls = {}
    for name in arts + [n + ".sig" for n in arts]:
        blob = get(assets[name], gh_tok, binary=True)
        print(f"上传 {name} ({len(blob)//1024}KB)")
        att = post(f"{GITEE_API}/repos/{GITEE_REPO}/releases/{rid}/attach_files",
                   fields={"access_token": token, "file": (name, blob)})
        att_urls[name] = att.get("browser_download_url") or att.get("url") or \
            f"https://gitee.com/{GITEE_REPO}/releases/download/{tag}/{name}"
        print("  ->", att_urls[name])

    # 拉 GitHub 侧 latest.json，把 URL 改写为 Gitee 附件地址
    lj = get(assets["latest.json"], gh_tok) if "latest.json" in assets else None
    if not lj:
        sys.exit("release 里没有 latest.json")
    for ent in lj["platforms"].values():
        fn = ent["url"].rsplit("/", 1)[-1]
        if fn in att_urls:
            ent["url"] = att_urls[fn]
        elif "gitee.com" not in ent["url"]:
            ent["url"] = "https://gh-proxy.com/" + ent["url"]  # 没搬进 Gitee 的产物走加速代理
    print("platforms:", list(lj["platforms"]))

    # Contents API 推 latest.json 到 Gitee 仓 main
    api = f"{GITEE_API}/repos/{GITEE_REPO}/contents/latest.json"
    try:
        sha = get(f"{api}?access_token={token}&ref=main").get("sha")
    except Exception:
        sha = None
    body = {"access_token": token,
            "content": base64.b64encode(json.dumps(lj, ensure_ascii=False, indent=2).encode()).decode(),
            "message": f"latest.json {tag}", "branch": "main"}
    if sha:
        body["sha"] = sha
    req = urllib.request.Request(api, data=json.dumps(body).encode(), method="PUT")
    print("gitee put:", urllib.request.urlopen(req).status)


if __name__ == "__main__":
    main()
