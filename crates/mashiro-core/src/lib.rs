//! 椎名真白 Mashiro — QQ 机器人框架核心库。
//! 不依赖 tauri：协议客户端 / 规则引擎 / 插件宿主 / 存储，可独立测试。

pub mod agent;
pub mod bot;
pub mod mcp;
pub mod plugin;
pub mod router;
pub mod scheduler;
pub mod storage;
pub mod webhook;
