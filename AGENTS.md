# AGENTS.md — 椎名真白 / Shiina Mashiro

Rust workspace（`src-tauri` + `crates/mashiro-core`）+ Tauri 2 + React/TS 前端。

## 硬性规则

- **禁止使用终端批量修改文件**。文件读取/写入/替换一律走专用 read / write / edit 工具；不要用 `Set-Content`/`Out-File`/`-replace` 管道改文件——Windows PowerShell 默认 GBK 读取会把 UTF-8 无 BOM 的中文文件读成乱码再写坏（2026 已实际损坏过 api.rs 一次，靠 git checkout 恢复）。
- 同理，`Get-Content -Raw` + `Set-Content` 这类「读改写」脚本在含中文的源文件上禁止整套使用。

## 终端编码

本机终端已配置 UTF-8（PowerShell profile + cmd Autorun + `PYTHONIOENCODING`）。
即便如此，终端输出仍可能有历史乱码注释——**不要把终端回显的中文当文件真实内容**，改文件前一律用 read 工具确认原文。

## 常用命令

- `cargo fmt --all` / `cargo check --workspace`
- 前端：`cd ui && npm run build`（Vite chunk>500KB 警告正常）
- 插件 SDK：`node -e "require('./sdk/js/mashiro.js')"`、`python -c "import mashiro"` 语法验证

## 约定

- 官方 QQ Bot API v2 文档镜像：`E:\DeskTop\DevLuoxiaoxi\qqbot-docs\site\`（端点/事件清单在同级 `endpoints.txt`/`events.txt`）；频道管理域按需求不做，其余 API 以镜像文档为唯一真相。
- 提交规范：仓库历史用 conventional commits；不要替用户 push。
