//! 线协议消息构造与发送目标定义。
//!
//! 线上格式统一为 `{"tag": "<模块名>", ...payload}` 的 JSON 文本：
//! - 握手：`tag = "__hs__"`
//! - 文件：`tag = "file"`，出站消息用 `stat` 区分动作，对端回包用 `type`。
//!
//! 目标端为弦电子书（`com.bandbbs.ebook.plus`）：按章节拆分，每章多块。
//!
//! 除章节传输外，还对齐了安卓端的：书籍状态查询、存储信息、封面传输、
//! 手环阅读设置读写（`get_settings` / `set_settings`）。

use serde_json::{json, Map, Value};

/// 目标应用包名（弦电子书）。
pub const PACKAGE: &str = "com.bandbbs.ebook.plus";
/// 目标应用显示名。
pub const APP_LABEL: &str = "弦电子书";
/// 目标应用的一句话说明。
pub const APP_HINT: &str = "按章节传输，需 V26.5.1+";

pub const TAG_HANDSHAKE: &str = "__hs__";
pub const TAG_FILE: &str = "file";

/// 手机端声明版本号的下限（对应 V26.5.1）。
///
/// 弦电子书(plus) 的握手会校验：
/// ```js
/// if ((version && version < MIN_PHONE_VERSION) || !version) → 「版本不兼容 · 手机端版本过低」
/// ```
/// 当前手环端要求 `MIN_PHONE_VERSION = 126510`（V26.5.1）。
///
/// 版本号编码规则（由官方安卓端 `versionCode` 反推）：
/// `1` + 主版本(2 位) + 次版本(1 位) + 补丁(1 位) + `0`
/// 例如 V26.4.0 → 126400、V26.4.3 → 126430、V26.5.1 → 126510。
pub const SENDER_VERSION: i64 = 126_510;

/// 握手消息。
///
/// `peer_version` 为手环在握手中自报的版本号（弦电子书里是它自己的 `versionCode`）。
/// 取 `max(SENDER_VERSION, peer_version)`：既满足当前门禁，
/// 也能在手环后续升级、抬高门槛时自动跟上，不至于再次被判「版本过低」。
pub fn handshake(count: usize, peer_version: Option<i64>) -> String {
    let version = peer_version.unwrap_or(0).max(SENDER_VERSION);
    json!({
        "tag": TAG_HANDSHAKE,
        "count": count,
        "version": version,
    })
    .to_string()
}

// ---------- 传输协议（弦电子书） ----------

/// 开始传输。
///
/// `total` 为整本书的章节总数（不是本次发送的章节数），`start_from` 为本批首章的
/// 书内下标——`send_order` 过滤掉已同步章节后，这两个值仍然描述整本书。
pub fn plus_start_transfer(
    filename: &str,
    total_chapters: usize,
    word_count: usize,
    start_from: usize,
    has_cover: bool,
    author: Option<&str>,
    summary: Option<&str>,
) -> String {
    let mut m = Map::new();
    m.insert("tag".to_string(), json!(TAG_FILE));
    m.insert("stat".to_string(), json!("startTransfer"));
    m.insert("filename".to_string(), json!(filename));
    m.insert("total".to_string(), json!(total_chapters));
    m.insert("wordCount".to_string(), json!(word_count));
    m.insert("startFrom".to_string(), json!(start_from));
    m.insert("hasCover".to_string(), json!(has_cover));
    if let Some(author) = author.filter(|s| !s.is_empty()) {
        m.insert("author".to_string(), json!(author));
    }
    if let Some(summary) = summary.filter(|s| !s.is_empty()) {
        m.insert("summary".to_string(), json!(summary));
    }
    Value::Object(m).to_string()
}

pub fn plus_chapter_chunk(
    chapter_index: usize,
    chapter_name: &str,
    word_count: usize,
    content: &str,
    chunk_num: usize,
    total_chunks: usize,
) -> String {
    let data = json!({
        "index": chapter_index,
        "name": chapter_name,
        "wordCount": word_count,
        "content": content,
        "chunkNum": chunk_num,
        "totalChunks": total_chunks,
    })
    .to_string();

    json!({
        "tag": TAG_FILE,
        "stat": "d",
        "count": chapter_index,
        "data": data,
    })
    .to_string()
}

pub fn plus_chapter_complete(chapter_index: usize) -> String {
    json!({
        "tag": TAG_FILE,
        "stat": "chapter_complete",
        "count": chapter_index,
    })
    .to_string()
}

pub fn plus_transfer_complete() -> String {
    json!({
        "tag": TAG_FILE,
        "stat": "transfer_complete",
    })
    .to_string()
}

// ---------- plus 扩展（对齐安卓端 App 功能） ----------

/// 查询某本书在手环上的同步状态（已同步章节下标 + 是否已有封面）。
pub fn plus_get_book_status(filename: &str) -> String {
    json!({
        "tag": TAG_FILE,
        "stat": "get_book_status",
        "filename": filename,
    })
    .to_string()
}

/// 查询手环存储信息。
pub fn plus_get_storage_info() -> String {
    json!({
        "tag": TAG_FILE,
        "stat": "get_storage_info",
    })
    .to_string()
}

/// 读取手环端阅读设置。
pub fn plus_get_settings(keys: &[&str]) -> String {
    json!({
        "tag": TAG_FILE,
        "stat": "get_settings",
        "keys": keys,
    })
    .to_string()
}

/// 写入一项手环端阅读设置。
pub fn plus_set_setting(key: &str, value: &str) -> String {
    let mut settings = Map::new();
    settings.insert(key.to_string(), json!(value));
    json!({
        "tag": TAG_FILE,
        "stat": "set_settings",
        "settings": Value::Object(settings),
    })
    .to_string()
}

/// 封面分块（`data` 为 base64 文本）。
pub fn plus_cover_chunk(chunk_index: usize, total_chunks: usize, data: &str) -> String {
    json!({
        "tag": TAG_FILE,
        "stat": "cover_chunk",
        "chunkIndex": chunk_index,
        "totalChunks": total_chunks,
        "data": data,
    })
    .to_string()
}

/// 封面传输结束。
pub fn plus_cover_transfer_complete() -> String {
    json!({
        "tag": TAG_FILE,
        "stat": "cover_transfer_complete",
    })
    .to_string()
}

pub fn cancel() -> String {
    json!({
        "tag": TAG_FILE,
        "stat": "cancel",
    })
    .to_string()
}

// ---------- 工具 ----------

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64 编码（带 `=` 填充），用于封面图片。
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(B64_ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // 含 0x00 / 0xFF 的二进制
        assert_eq!(base64_encode(&[0x00, 0xFF]), "AP8=");
    }

    #[test]
    fn handshake_version_is_lifted() {
        let v: Value = serde_json::from_str(&handshake(0, None)).unwrap();
        assert_eq!(v["version"], json!(SENDER_VERSION));
        assert_eq!(v["tag"], json!(TAG_HANDSHAKE));

        let v2: Value = serde_json::from_str(&handshake(1, Some(260_510))).unwrap();
        assert_eq!(v2["version"], json!(260_510));

        let v3: Value = serde_json::from_str(&handshake(1, Some(1))).unwrap();
        assert_eq!(v3["version"], json!(SENDER_VERSION));
    }

    #[test]
    fn start_transfer_carries_meta() {
        let v: Value =
            serde_json::from_str(&plus_start_transfer("a.txt", 12, 3000, 3, true, Some("作者"), Some("简介")))
                .unwrap();
        assert_eq!(v["stat"], json!("startTransfer"));
        assert_eq!(v["total"], json!(12));
        assert_eq!(v["startFrom"], json!(3));
        assert_eq!(v["hasCover"], json!(true));
        assert_eq!(v["author"], json!("作者"));
        assert_eq!(v["summary"], json!("简介"));
    }

    #[test]
    fn start_transfer_omits_empty_meta() {
        let s = plus_start_transfer("a.txt", 1, 10, 0, false, None, Some(""));
        assert!(!s.contains("author"));
        assert!(!s.contains("summary"));
        assert!(s.contains("hasCover"));
    }
}
