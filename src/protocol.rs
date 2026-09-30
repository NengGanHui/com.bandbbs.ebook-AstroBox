// 线协议消息构造与发送目标定义

use serde_json::{json, Map, Value};

/// 目标应用包名
pub const PACKAGE: &str = "com.bandbbs.ebook.plus";
/// 目标应用显示名
pub const APP_LABEL: &str = "弦电子书";
/// 目标应用的一句话说明
pub const APP_HINT: &str = "按章节传输，需 V26.5.4";

/// 新协议通道字段 `k` 的取值：握手通道
pub const CHANNEL_HANDSHAKE: &str = "8d7a";
/// 新协议通道字段 `k` 的取值：文件通道
pub const CHANNEL_FILE: &str = "7db1";

/// 旧版握手 tag（仅保留给归一化层/测试用，新协议不再对外发送）
pub const TAG_HANDSHAKE: &str = "__hs__";
/// 旧版文件 tag（同上）
pub const TAG_FILE: &str = "file";

/// 版本号旧编码规则：
/// `1` + 主版本(2 位) + 次版本(1 位) + 补丁(1 位) + `0`，
/// 例如 V26.5.1 → 126510、V26.5.4 → 126540。
pub const SENDER_VERSION: i64 = 126_540;

// ---------- 请求码（字段 `q`，手机 → 手环） ----------

pub const REQ_START_TRANSFER: &str = "cd1d";
pub const REQ_START_COVER_TRANSFER: &str = "6527";
pub const REQ_START_ILLUSTRATION_TRANSFER: &str = "b0ad";
pub const REQ_DATA: &str = "30fb";
pub const REQ_CHAPTER_COMPLETE: &str = "9127";
pub const REQ_TRANSFER_COMPLETE: &str = "5519";
pub const REQ_CANCEL: &str = "95cb";
pub const REQ_GET_BOOK_STATUS: &str = "3d11";
pub const REQ_COVER_CHUNK: &str = "809c";
pub const REQ_COVER_TRANSFER_COMPLETE: &str = "5f4d";
pub const REQ_ILLUSTRATION_CHUNK: &str = "5b43";
pub const REQ_ILLUSTRATION_TRANSFER_COMPLETE: &str = "b0fc";
pub const REQ_UPDATE_BOOK_INFO: &str = "8eba";
pub const REQ_GET_READING_DATA: &str = "a16c";
pub const REQ_RD_GET: &str = "56c4";
pub const REQ_SET_READING_DATA: &str = "1c43";
pub const REQ_SET_READING_DATA_START: &str = "10a6";
pub const REQ_SET_READING_DATA_CHUNK: &str = "cfcf";
pub const REQ_DELETE_CHAPTERS: &str = "fb21";
pub const REQ_DELETE_BOOK: &str = "f000";
pub const REQ_GET_STORAGE_INFO: &str = "821f";
pub const REQ_GET_SETTINGS: &str = "e242";
pub const REQ_SET_SETTINGS: &str = "379d";

// ---------- 回包码（字段 `a`，手环 → 手机）→ 逻辑名 ----------

/// `(码, 逻辑名)` 对照表，与手环端 `replies` 冻结对象一一对应
const REPLY_CODES: &[(&str, &str)] = &[
    ("7a3b", "book_info_updated"),
    ("c3e0", "book_status"),
    ("58f7", "cancel"),
    ("baf3", "chapter_chunk_complete"),
    ("eafc", "chapter_saved"),
    ("d567", "cover_chunk_received"),
    ("87ae", "cover_ready"),
    ("cc27", "cover_saved"),
    ("78f5", "error"),
    ("8615", "illustration_chunk_received"),
    ("e19b", "illustration_ready"),
    ("caff", "illustration_saved"),
    ("c69a", "next"),
    ("cd87", "next_chunk"),
    ("b8a0", "progress"),
    ("8869", "rd_chunk_received"),
    ("da4a", "rd_ready"),
    ("efe8", "ready"),
    ("226f", "settings_data"),
    ("fa99", "storage_info"),
    ("02c1", "success"),
    ("3969", "sync_reading_data"),
    ("3e94", "sync_reading_data_chunk"),
    ("73f4", "sync_reading_data_start"),
    ("89ae", "transfer_finished"),
];

/// 把回包码翻译回逻辑名
pub fn reply_code_name(code: &str) -> Option<&'static str> {
    REPLY_CODES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| *name)
}

/// 握手消息
pub fn handshake(count: usize, peer_version: Option<i64>) -> String {
    let version = peer_version.unwrap_or(0).max(SENDER_VERSION);
    json!({
        "k": CHANNEL_HANDSHAKE,
        "n": count,
        "v": version,
    })
    .to_string()
}

/// 构造文件请求骨架：`{"k":"7db1","q":<请求码>}`
fn file_request(request_code: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("k".to_string(), json!(CHANNEL_FILE));
    m.insert("q".to_string(), json!(request_code));
    m
}

// ---------- 传输协议 ----------

/// 开始传输
///
/// `total` 为整本书的章节总数（不是本次发送的章节数），`start_from` 为本批首章的
/// 书内下标——`send_order` 过滤掉已同步章节后，这两个值仍然描述整本书
pub fn plus_start_transfer(
    filename: &str,
    total_chapters: usize,
    word_count: usize,
    start_from: usize,
    has_cover: bool,
    author: Option<&str>,
    summary: Option<&str>,
) -> String {
    let mut m = file_request(REQ_START_TRANSFER);
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

/// 章节分块
///
/// `content_length_u16` / `content_checksum` 为整章内容按 UTF-16 码元计的长度与
/// Adler-32 校验（与手环端 `charCodeAt` 累加一致），每个分块都带全章声明
/// 手环端逐块累计后做完整性校验
pub fn plus_chapter_chunk(
    chapter_index: usize,
    chapter_name: &str,
    word_count: usize,
    content: &str,
    chunk_num: usize,
    total_chunks: usize,
    content_length_u16: usize,
    content_checksum: u64,
) -> String {
    let data = json!({
        "index": chapter_index,
        "name": chapter_name,
        "wordCount": word_count,
        "content": content,
        "chunkNum": chunk_num,
        "totalChunks": total_chunks,
        "contentLength": content_length_u16,
        "contentChecksum": content_checksum,
    })
    .to_string();

    let mut m = file_request(REQ_DATA);
    m.insert("count".to_string(), json!(chapter_index));
    m.insert("data".to_string(), json!(data));
    Value::Object(m).to_string()
}

pub fn plus_chapter_complete(chapter_index: usize) -> String {
    let mut m = file_request(REQ_CHAPTER_COMPLETE);
    m.insert("count".to_string(), json!(chapter_index));
    Value::Object(m).to_string()
}

pub fn plus_transfer_complete() -> String {
    Value::Object(file_request(REQ_TRANSFER_COMPLETE)).to_string()
}

pub fn cancel() -> String {
    Value::Object(file_request(REQ_CANCEL)).to_string()
}

// ---------- 扩展 ----------

/// 查询某本书在手环上的同步状态（已同步章节下标 + 封面 + 已同步插图清单）
pub fn plus_get_book_status(filename: &str) -> String {
    let mut m = file_request(REQ_GET_BOOK_STATUS);
    m.insert("filename".to_string(), json!(filename));
    Value::Object(m).to_string()
}

/// 查询手环存储信息
pub fn plus_get_storage_info() -> String {
    Value::Object(file_request(REQ_GET_STORAGE_INFO)).to_string()
}

/// 读取手环端阅读设置
pub fn plus_get_settings(keys: &[&str]) -> String {
    let mut m = file_request(REQ_GET_SETTINGS);
    m.insert("keys".to_string(), json!(keys));
    Value::Object(m).to_string()
}

/// 写入一项手环端阅读设置
pub fn plus_set_setting(key: &str, value: &str) -> String {
    let mut settings = Map::new();
    settings.insert(key.to_string(), json!(value));
    let mut m = file_request(REQ_SET_SETTINGS);
    m.insert("settings".to_string(), Value::Object(settings));
    Value::Object(m).to_string()
}

/// 封面分块（`data` 为 base64 文本）
pub fn plus_cover_chunk(chunk_index: usize, total_chunks: usize, data: &str) -> String {
    let mut m = file_request(REQ_COVER_CHUNK);
    m.insert("chunkIndex".to_string(), json!(chunk_index));
    m.insert("totalChunks".to_string(), json!(total_chunks));
    m.insert("data".to_string(), json!(data));
    Value::Object(m).to_string()
}

/// 封面传输结束
pub fn plus_cover_transfer_complete() -> String {
    Value::Object(file_request(REQ_COVER_TRANSFER_COMPLETE)).to_string()
}

/// 删除手环上的整本书（书籍目录、索引与书库条目一并清除）
pub fn plus_delete_book(filename: &str) -> String {
    let mut m = file_request(REQ_DELETE_BOOK);
    m.insert("filename".to_string(), json!(filename));
    Value::Object(m).to_string()
}

/// 删除某本书的指定章节（`chapter_indices` 为书内章节下标，从 0 开始）
pub fn plus_delete_chapters(filename: &str, chapter_indices: &[usize]) -> String {
    let mut m = file_request(REQ_DELETE_CHAPTERS);
    m.insert("filename".to_string(), json!(filename));
    m.insert("chapterIndices".to_string(), json!(chapter_indices));
    Value::Object(m).to_string()
}

/// 更新手环上某本书的信息；`None` / 空串字段不下发（手环只更新非 null 字段）
pub fn plus_update_book_info(
    filename: &str,
    author: Option<&str>,
    summary: Option<&str>,
) -> String {
    let mut m = file_request(REQ_UPDATE_BOOK_INFO);
    m.insert("filename".to_string(), json!(filename));
    if let Some(author) = author.filter(|s| !s.is_empty()) {
        m.insert("author".to_string(), json!(author));
    }
    if let Some(summary) = summary.filter(|s| !s.is_empty()) {
        m.insert("summary".to_string(), json!(summary));
    }
    Value::Object(m).to_string()
}

/// 查询某本书的阅读进度/阅读时长/书签
///
/// 带 `v: 2` 走分块协议：手环先回 `sync_reading_data_start`
/// 插件再逐块 `rd_get` 拉取 `sync_reading_data_chunk`
pub fn plus_get_reading_data(filename: &str) -> String {
    let mut m = file_request(REQ_GET_READING_DATA);
    m.insert("filename".to_string(), json!(filename));
    m.insert("v".to_string(), json!(2));
    Value::Object(m).to_string()
}

/// 拉取阅读数据的第 `chunk_index` 块（配合 `sync_reading_data_start` 使用）
pub fn plus_rd_get(chunk_index: usize) -> String {
    let mut m = file_request(REQ_RD_GET);
    m.insert("chunkIndex".to_string(), json!(chunk_index));
    Value::Object(m).to_string()
}

/// 整体写入某本书的阅读数据（数据小时可一次下发）
///
/// `progress` / `reading_time` 为 JSON 字符串（手环端会 `JSON.parse`）
/// `bookmarks` 为书签数组 JSON 值
pub fn plus_set_reading_data(
    filename: &str,
    progress: &str,
    reading_time: &str,
    bookmarks: Value,
) -> String {
    let mut m = file_request(REQ_SET_READING_DATA);
    m.insert("filename".to_string(), json!(filename));
    m.insert("progress".to_string(), json!(progress));
    m.insert("readingTime".to_string(), json!(reading_time));
    m.insert("bookmarks".to_string(), bookmarks);
    Value::Object(m).to_string()
}

/// 分块写入阅读数据：第一步，声明总长度与块数（数据 > 6144 字符时使用）
pub fn plus_set_reading_data_start(filename: &str, total_length: usize, total_chunks: usize) -> String {
    let mut m = file_request(REQ_SET_READING_DATA_START);
    m.insert("filename".to_string(), json!(filename));
    m.insert("totalLength".to_string(), json!(total_length));
    m.insert("totalChunks".to_string(), json!(total_chunks));
    Value::Object(m).to_string()
}

/// 分块写入阅读数据：第二步，发送第 `chunk_index` 块文本
pub fn plus_set_reading_data_chunk(chunk_index: usize, data: &str) -> String {
    let mut m = file_request(REQ_SET_READING_DATA_CHUNK);
    m.insert("chunkIndex".to_string(), json!(chunk_index));
    m.insert("data".to_string(), json!(data));
    Value::Object(m).to_string()
}

/// 开始传输一张插图（支断点续传）
///
/// - `relative_path`：书内相对路径（如 `images/01.jpg`），不允许 `..` 与开头 `/`
/// - `fingerprint`：内容指纹，手环端用它判断「已同步 / 断点续传 / 重传」
/// - `chunk_size`：每块 base64 字符数（≤16384），手环按 `chunk_size/4*3` 校验每块原始字节
pub fn plus_start_illustration_transfer(
    filename: &str,
    relative_path: &str,
    fingerprint: &str,
    total_bytes: usize,
    total_chunks: usize,
    chunk_size: usize,
) -> String {
    let mut m = file_request(REQ_START_ILLUSTRATION_TRANSFER);
    m.insert("filename".to_string(), json!(filename));
    m.insert("relativePath".to_string(), json!(relative_path));
    m.insert("fingerprint".to_string(), json!(fingerprint));
    m.insert("totalBytes".to_string(), json!(total_bytes));
    m.insert("totalChunks".to_string(), json!(total_chunks));
    m.insert("chunkSize".to_string(), json!(chunk_size));
    Value::Object(m).to_string()
}

/// 插图分块
pub fn plus_illustration_chunk(
    relative_path: &str,
    chunk_index: usize,
    total_chunks: usize,
    data: &str,
) -> String {
    let mut m = file_request(REQ_ILLUSTRATION_CHUNK);
    m.insert("relativePath".to_string(), json!(relative_path));
    m.insert("chunkIndex".to_string(), json!(chunk_index));
    m.insert("totalChunks".to_string(), json!(total_chunks));
    m.insert("data".to_string(), json!(data));
    Value::Object(m).to_string()
}

/// 插图传输结束（手环校验总字节数后写入插图清单）
pub fn plus_illustration_transfer_complete(relative_path: &str) -> String {
    let mut m = file_request(REQ_ILLUSTRATION_TRANSFER_COMPLETE);
    m.insert("relativePath".to_string(), json!(relative_path));
    Value::Object(m).to_string()
}

// ---------- 校验 ----------

/// Adler-32（模 65521），与手环端逐 `charCodeAt` 码元累加的算法一致
/// `A` 起始 1，`B` 起始 0；每单元 `A = (A + u) % 65521; B = (B + A) % 65521`
/// 结果为 `B * 65536 + A`。返回 `(UTF-16 码元长度, 校验值)`
pub fn utf16_len_and_adler32(content: &str) -> (usize, u64) {
    let mut a: u64 = 1;
    let mut b: u64 = 0;
    let mut len: usize = 0;
    for unit in content.encode_utf16() {
        a = (a + unit as u64) % 65_521;
        b = (b + a) % 65_521;
        len += 1;
    }
    (len, (b << 16) | a)
}

/// 按字节计算 Adler-32，用于插图内容指纹（只要求稳定，不要求跨端一致）
pub fn adler32_bytes(data: &[u8]) -> u64 {
    let mut a: u64 = 1;
    let mut b: u64 = 0;
    for &byte in data {
        a = (a + byte as u64) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// 插图内容指纹
pub fn fingerprint(data: &[u8]) -> String {
    format!("adler-{:08x}", adler32_bytes(data) as u32)
}

// ---------- 工具 ----------

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64 编码（带 `=` 填充），用于封面与插图图片
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
        assert_eq!(base64_encode(&[0x00, 0xFF]), "AP8=");
    }

    #[test]
    fn handshake_uses_new_format_and_lifted_version() {
        let v: Value = serde_json::from_str(&handshake(0, None)).unwrap();
        assert_eq!(v["k"], json!(CHANNEL_HANDSHAKE));
        assert_eq!(v["n"], json!(0));
        assert_eq!(v["v"], json!(SENDER_VERSION));

        // 手环新编码 versionCode 高于门禁时原样上报
        let v2: Value = serde_json::from_str(&handshake(1, Some(260_919))).unwrap();
        assert_eq!(v2["v"], json!(260_919));

        // 对端版本低于门禁时抬到 SENDER_VERSION
        let v3: Value = serde_json::from_str(&handshake(1, Some(126_510))).unwrap();
        assert_eq!(v3["v"], json!(SENDER_VERSION));
    }

    #[test]
    fn file_requests_use_channel_and_request_code() {
        let v: Value = serde_json::from_str(&plus_start_transfer("a.txt", 12, 3000, 3, true, Some("作者"), Some("简介")))
            .unwrap();
        assert_eq!(v["k"], json!(CHANNEL_FILE));
        assert_eq!(v["q"], json!(REQ_START_TRANSFER));
        assert_eq!(v["total"], json!(12));
        assert_eq!(v["startFrom"], json!(3));
        assert_eq!(v["hasCover"], json!(true));
        assert_eq!(v["author"], json!("作者"));
        assert_eq!(v["summary"], json!("简介"));

        let s = plus_start_transfer("a.txt", 1, 10, 0, false, None, Some(""));
        assert!(!s.contains("author"));
        assert!(!s.contains("summary"));
        assert!(s.contains("hasCover"));

        let v: Value = serde_json::from_str(&plus_get_storage_info()).unwrap();
        assert_eq!(v["q"], json!(REQ_GET_STORAGE_INFO));
    }

    #[test]
    fn chapter_chunk_declares_length_and_checksum() {
        let (len, sum) = utf16_len_and_adler32("foobar");
        assert_eq!(len, 6);
        assert_eq!(sum, 0x08AB_027A);
        assert_eq!(utf16_len_and_adler32("Wikipedia").1, 0x11E6_0398);

        let v: Value = serde_json::from_str(&plus_chapter_chunk(2, "第二章", 100, "abc", 0, 1, len, sum))
            .unwrap();
        assert_eq!(v["q"], json!(REQ_DATA));
        assert_eq!(v["count"], json!(2));
        let data: Value = serde_json::from_str(v["data"].as_str().unwrap()).unwrap();
        assert_eq!(data["index"], json!(2));
        assert_eq!(data["contentLength"], json!(len));
        assert_eq!(data["contentChecksum"], json!(sum));
    }

    #[test]
    fn new_feature_builders() {
        let v: Value = serde_json::from_str(&plus_delete_book("书名")).unwrap();
        assert_eq!(v["q"], json!(REQ_DELETE_BOOK));
        assert_eq!(v["filename"], json!("书名"));

        let v: Value = serde_json::from_str(&plus_delete_chapters("书名", &[0, 2, 5])).unwrap();
        assert_eq!(v["q"], json!(REQ_DELETE_CHAPTERS));
        assert_eq!(v["chapterIndices"], json!([0, 2, 5]));

        let v: Value = serde_json::from_str(&plus_update_book_info("书名", Some("新作者"), None))
            .unwrap();
        assert_eq!(v["q"], json!(REQ_UPDATE_BOOK_INFO));
        assert_eq!(v["author"], json!("新作者"));
        assert!(v.get("summary").is_none());

        let v: Value = serde_json::from_str(&plus_get_reading_data("书名")).unwrap();
        assert_eq!(v["q"], json!(REQ_GET_READING_DATA));
        assert_eq!(v["v"], json!(2));

        let v: Value = serde_json::from_str(&plus_rd_get(3)).unwrap();
        assert_eq!(v["chunkIndex"], json!(3));

        let v: Value =
            serde_json::from_str(&plus_start_illustration_transfer("书名", "images/01.jpg", "adler-00000001", 6144, 1, 8192))
                .unwrap();
        assert_eq!(v["q"], json!(REQ_START_ILLUSTRATION_TRANSFER));
        assert_eq!(v["relativePath"], json!("images/01.jpg"));
        assert_eq!(v["totalChunks"], json!(1));
        assert_eq!(v["chunkSize"], json!(8192));

        let v: Value = serde_json::from_str(&plus_illustration_chunk("images/01.jpg", 0, 1, "Zm9v"))
            .unwrap();
        assert_eq!(v["q"], json!(REQ_ILLUSTRATION_CHUNK));

        let v: Value =
            serde_json::from_str(&plus_illustration_transfer_complete("images/01.jpg")).unwrap();
        assert_eq!(v["q"], json!(REQ_ILLUSTRATION_TRANSFER_COMPLETE));
    }

    #[test]
    fn reply_codes_resolve_to_logical_names() {
        assert_eq!(reply_code_name("efe8"), Some("ready"));
        assert_eq!(reply_code_name("cd87"), Some("next_chunk"));
        assert_eq!(reply_code_name("eafc"), Some("chapter_saved"));
        assert_eq!(reply_code_name("89ae"), Some("transfer_finished"));
        assert_eq!(reply_code_name("78f5"), Some("error"));
        assert_eq!(reply_code_name("e19b"), Some("illustration_ready"));
        assert_eq!(reply_code_name("0000"), None);
    }

    #[test]
    fn fingerprint_is_stable() {
        assert_eq!(fingerprint(b"foobar"), format!("adler-{:08x}", adler32_bytes(b"foobar") as u32));
        assert_eq!(fingerprint(b""), "adler-00000001");
    }
}
