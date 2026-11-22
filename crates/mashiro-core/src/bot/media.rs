//! 富媒体上传（图片/语音/视频/文件）。
//! 官方 v2 接口：POST {group|users}/{openid}/files，body 传 url 或本地 file 二进制。
//! P3 实现；注意保留「本地文件 → 上传 → file_info」的单链路，不做连接池。

// TODO(P3): upload_media / upload_rich_media
