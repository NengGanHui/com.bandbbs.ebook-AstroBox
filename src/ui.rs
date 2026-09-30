// 插件 UI 与传输状态

use crate::astrobox::psys_host::{
    self, device, dialog, interconnect, register, thirdpartyapp, timer, ui,
};
use crate::chapters::{self, Chapter, SplitOptions};
use crate::protocol;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::SystemTime;

/// 正文按章切分，再按块发送\消息需 JSON 包装，留足余量
const PLUS_CHUNK_BYTES: usize = 16 * 1024;
/// 封面按 base64 文本切块（对齐安卓端 8KB/块）
const COVER_CHUNK_CHARS: usize = 8 * 1024;
/// 封面原始字节上限（超过则不发送，避免手环存储与 BLE 压力过大）
const COVER_MAX_BYTES: usize = 400 * 1024;
/// 插图原始字节上限（手环端对单块 base64 限 16384 字符，总量自控）
const ILLUSTRATION_MAX_BYTES: usize = 1024 * 1024;
/// 阅读数据查询长响应未到期时的等待时间（每收到一块就顺延一次）
const MGMT_QUERY_TIMEOUT_MS: u64 = 8000;
/// 单块发送失败后的最大重试次数
const MAX_RETRY: usize = 3;
/// 传输期间链路巡检间隔（1s 轮询，掉线判定见 `STALL_TIMEOUT_SECS`）
const WATCHDOG_INTERVAL_MS: u64 = 1000;
/// 超过该时长没有收到任何回包（传输停住不动），即判定掉线。
/// 手环端登记章节要写正文/索引文件，且每 30 章触发一次 GC，留足冗余避免误判
const STALL_TIMEOUT_SECS: u64 = 10;
/// 连续多少次巡检找不到设备才判定掉线，避免单次查询抖动造成误判
const OFFLINE_CONFIRM_STRIKES: usize = 2;
/// 掉线后两次重连之间的等待时间。
const RECONNECT_DELAY_MS: u64 = 1500;
/// 掉线后最多自动重连次数（与「重新尝试传输(3次)」对应）
const MAX_RECONNECT: usize = 3;
/// 取消传输后延迟刷新设备状态的等待时间，避开手环还在处理 cancel 的窗口
const CANCEL_REFRESH_DELAY_MS: u64 = 900;
/// 接收端剩余空间告警阈值
const STORAGE_WARN_BYTES: u64 = 25 * 1024 * 1024;
/// 日志保留条数
const MAX_LOG_LINES: usize = 120;

const TAG_HANDSHAKE: &str = "__hs__";
const TAG_FILE: &str = "file";

const EVENT_PICK_FILE: &str = "pick_file";
const EVENT_SEND_FILE: &str = "send_file";
const EVENT_CANCEL_SEND: &str = "cancel_send";
const EVENT_PICK_COVER: &str = "pick_cover";
const EVENT_CLEAR_COVER: &str = "clear_cover";
const EVENT_PICK_ILLUSTRATION: &str = "pick_illustration";
const EVENT_CLEAR_ILLUSTRATIONS: &str = "clear_illustrations";
const EVENT_CYCLE_SPLIT: &str = "cycle_split";
const EVENT_WORDS_UP: &str = "words_up";
const EVENT_WORDS_DOWN: &str = "words_down";
const EVENT_TOGGLE_SKIP: &str = "toggle_skip";
const EVENT_PANEL_SEND: &str = "panel_send";
const EVENT_PANEL_DEVICE: &str = "panel_device";
const EVENT_PANEL_LOG: &str = "panel_log";
const EVENT_PANEL_ABOUT: &str = "panel_about";
const EVENT_REFRESH_DEVICE: &str = "refresh_device";
const EVENT_LOAD_SETTINGS: &str = "load_settings";
const EVENT_CLEAR_LOG: &str = "clear_log";
const EVENT_EDIT_AUTHOR: &str = "edit_author";
const EVENT_EDIT_SUMMARY: &str = "edit_summary";
const EVENT_EDIT_KEYWORD: &str = "edit_keyword";
const EVENT_CYCLE_DISCONNECT: &str = "cycle_disconnect";
const EVENT_MGMT_DELETE_BOOK: &str = "mgmt_delete_book";
const EVENT_MGMT_DELETE_CHAPTERS: &str = "mgmt_delete_chapters";
const EVENT_MGMT_UPDATE_INFO: &str = "mgmt_update_info";
const EVENT_MGMT_READING_DATA: &str = "mgmt_reading_data";
const EVENT_MGMT_ILLUSTRATION: &str = "mgmt_illustration";

const SETTING_CLICK_PREFIX: &str = "setting:";
const SETTING_DEC_PREFIX: &str = "set_dec:";
const SETTING_INC_PREFIX: &str = "set_inc:";

const TIMER_HIDE_MESSAGE: &str = "timer_hide_message";
const TIMER_START_HANDSHAKE: &str = "timer_start_handshake";
const TIMER_HANDSHAKE_TIMEOUT: &str = "timer_handshake_timeout";
const TIMER_QUERY_TIMEOUT: &str = "timer_query_timeout";
const TIMER_TRANSFER_WATCHDOG: &str = "timer_transfer_watchdog";
const TIMER_RECONNECT_ATTEMPT: &str = "timer_reconnect_attempt";
const TIMER_REFRESH_AFTER_CANCEL: &str = "timer_refresh_after_cancel";

// ---------------------------------------------------------------- 配se

const C_TEXT: &str = "#1F2937";
const C_SUB: &str = "#6B7280";
const C_MUTED: &str = "#9AA4B2";
const C_ACCENT: &str = "#2563EB";
const C_ACCENT_SOFT: &str = "#E8F0FE";
const C_ACCENT_TEXT: &str = "#1D4ED8";
const C_SUCCESS: &str = "#15803D";
const C_ERROR: &str = "#DC2626";
const C_SURFACE: &str = "#FFFFFF";
const C_FIELD: &str = "#F3F5F9";
const C_BORDER: &str = "#E4E8F0";
const C_TRACK: &str = "#EDF0F5";
const C_WARN: &str = "#B45309";

const BAR_WIDTH: u32 = 240;

// ---------------------------------------------------------------- 手环设置表

enum SettingKind {
    Bool,
    Int {
        min: i64,
        max: i64,
        step: i64,
        suffix: &'static str,
    },
    Choice(&'static [(&'static str, &'static str)]),
}

struct SettingDef {
    key: &'static str,
    label: &'static str,
    kind: SettingKind,
    default: &'static str,
}

const READ_MODE_OPTS: &[(&str, &str)] = &[("scroll", "滚动"), ("nostalgic", "怀旧")];
const TIME_FORMAT_OPTS: &[(&str, &str)] = &[("24h", "24 小时"), ("12h", "12 小时")];
const CHAPTER_SWITCH_OPTS: &[(&str, &str)] =
    &[("button", "按钮"), ("boundary", "越界"), ("swipe", "滑动")];
const GESTURE_OPTS: &[(&str, &str)] = &[("single", "单击"), ("double", "双击")];

const ABOUT_LINES: &[&str] = &[
    "本 AstroBox 同步器为非官方插件。协议已按弦电子书 V26.5.4 更新，需手环端为 V26.5.4 版本。",
    "本次同步支持：分章传输（含整章校验）、封面与插图传输（插图支持断点续传）、删除书籍/章节、更新书籍信息、阅读进度查询、存储与阅读设置读写。",
    "旧版本（≤26.5.2）的手环端无法识别新协议。",
    "反馈BUG可加群：1107974387。",
    "开发本插件的目的是让 iPhone 用户能够向手环传输书籍。",
];

/// 阅读设置
const BAND_SETTINGS: &[SettingDef] = &[
    SettingDef {
        key: "EBOOK_FONT",
        label: "字号",
        kind: SettingKind::Int {
            min: 16,
            max: 64,
            step: 2,
            suffix: "",
        },
        default: "30",
    },
    SettingDef {
        key: "EBOOK_OPACITY",
        label: "不透明度",
        kind: SettingKind::Int {
            min: 10,
            max: 100,
            step: 10,
            suffix: "%",
        },
        default: "100",
    },
    SettingDef {
        key: "EBOOK_VERTICAL_MARGIN",
        label: "上下边距",
        kind: SettingKind::Int {
            min: 0,
            max: 40,
            step: 2,
            suffix: "",
        },
        default: "10",
    },
    SettingDef {
        key: "EBOOK_BOLD_ENABLED",
        label: "加粗",
        kind: SettingKind::Bool,
        default: "true",
    },
    SettingDef {
        key: "EBOOK_READ_MODE",
        label: "阅读模式",
        kind: SettingKind::Choice(READ_MODE_OPTS),
        default: "scroll",
    },
    SettingDef {
        key: "EBOOK_TIME_FORMAT",
        label: "时间格式",
        kind: SettingKind::Choice(TIME_FORMAT_OPTS),
        default: "24h",
    },
    SettingDef {
        key: "EBOOK_CHAPTER_SWITCH_STYLE",
        label: "章节切换",
        kind: SettingKind::Choice(CHAPTER_SWITCH_OPTS),
        default: "button",
    },
    SettingDef {
        key: "EBOOK_GESTURE",
        label: "翻页手势",
        kind: SettingKind::Choice(GESTURE_OPTS),
        default: "single",
    },
    SettingDef {
        key: "EBOOK_SHOW_PROGRESS_BAR",
        label: "显示进度条",
        kind: SettingKind::Bool,
        default: "true",
    },
    SettingDef {
        key: "EBOOK_SHOW_PROGRESS_BAR_PERCENT",
        label: "进度条显示百分比",
        kind: SettingKind::Bool,
        default: "true",
    },
    SettingDef {
        key: "EBOOK_PREVENT_PARAGRAPH_SPLITTING",
        label: "防止段落截断",
        kind: SettingKind::Bool,
        default: "false",
    },
    SettingDef {
        key: "EBOOK_ALWAYS_SHOW_TIME",
        label: "常显时间",
        kind: SettingKind::Bool,
        default: "true",
    },
    SettingDef {
        key: "EBOOK_ALWAYS_SHOW_BATTERY",
        label: "常显电量",
        kind: SettingKind::Bool,
        default: "true",
    },
    SettingDef {
        key: "EBOOK_BRIGHTNESS_FOLLOW_SYSTEM",
        label: "亮度跟随系统",
        kind: SettingKind::Bool,
        default: "true",
    },
];

fn band_setting(key: &str) -> Option<&'static SettingDef> {
    BAND_SETTINGS.iter().find(|d| d.key == key)
}

// ---------------------------------------------------------------- 状态

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Send,
    Device,
    Log,
    About,
}

#[derive(Clone, Copy)]
enum Tone {
    Neutral,
    Success,
    Error,
}

/// 传输过程中设备掉线后的处置策略
#[derive(Clone, Copy, PartialEq, Eq)]
enum DisconnectAction {
    Retry,
    Cancel,
}

impl DisconnectAction {
    fn label(self) -> &'static str {
        match self {
            DisconnectAction::Retry => "重新尝试(3次)",
            DisconnectAction::Cancel => "取消传输",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            DisconnectAction::Retry => "掉线后自动重连，最多 3 次并续传",
            DisconnectAction::Cancel => "掉线后立即终止本次传输",
        }
    }

    fn next(self) -> Self {
        match self {
            DisconnectAction::Retry => DisconnectAction::Cancel,
            DisconnectAction::Cancel => DisconnectAction::Retry,
        }
    }
}

/// 会话：按章节发送，每章再切成若干分块；可选先发封面
struct PlusJob {
    file_name: String,
    chapters: Vec<Chapter>,
    send_order: Vec<usize>,
    pos: usize,
    chunk_idx: usize,
    chunk_texts: Vec<String>,
    total_words: usize,
    author: String,
    summary: String,
    cover_chunks: Vec<String>,
    cover_idx: usize,
    cover_done: bool,
    bytes_sent: usize,
    last_chunk_time: Option<SystemTime>,
    retries: usize,
    content_len_u16: usize,
    content_checksum: u64,
}

#[derive(Clone, PartialEq, Eq)]
enum QueryKind {
    Status,
    Settings,
    DeleteBook(String),
    DeleteChapters(String, Vec<usize>),
    UpdateBookInfo {
        filename: String,
        author: String,
        summary: String,
    },
    GetReadingData(String),
    SendIllustration(IllustrationJob),
}

#[derive(Clone, PartialEq, Eq)]
struct IllustrationJob {
    filename: String,
    relative_path: String,
    fingerprint: String,
    chunks: Vec<String>,
    total_bytes: usize,
    chunk_size: usize,
    idx: usize,
}

struct PendingIllustration {
    relative_path: String,
    bytes: Vec<u8>,
}

struct RdIncoming {
    total_length: usize,
    total_chunks: usize,
    chunks: Vec<String>,
    received: usize,
}

enum Job {
    Plus(PlusJob),
    Query(QueryKind),
}

struct Session {
    device_addr: String,
    job: Job,
    pending_start: bool,
    handshake_complete: bool,
}

impl Session {
    fn device_addr(&self) -> &str {
        &self.device_addr
    }

    fn is_transfer(&self) -> bool {
        !matches!(self.job, Job::Query(_))
    }

    fn query_kind(&self) -> Option<QueryKind> {
        match &self.job {
            Job::Query(kind) => Some(kind.clone()),
            _ => None,
        }
    }

    /// 推进握手状态，返回是否应当开始传输/查询
    fn handshake_progress(&mut self, count: usize) -> bool {
        if count > 0 {
            self.handshake_complete = true;
        }
        let should_start = self.pending_start && self.handshake_complete;
        if should_start {
            self.pending_start = false;
        }
        should_start
    }

    /// 记录一次失败；返回应执行的重试动作
    fn bump_retry(&mut self) -> Retry {
        match &mut self.job {
            Job::Plus(t) => {
                t.retries += 1;
                if t.retries <= MAX_RETRY {
                    Retry::Plus
                } else {
                    Retry::Fail
                }
            }
            Job::Query(_) => Retry::Fail,
        }
    }
}

enum Retry {
    Fail,
    Plus,
}

/// 手环信息（存储 / 书籍状态 / 阅读设置）
#[derive(Default)]
struct BandInfo {
    product: Option<String>,
    storage_known: bool,
    storage_total: u64,
    storage_avail: u64,
    status_known: bool,
    synced_chapters: Vec<usize>,
    has_cover: bool,
    settings: BTreeMap<String, String>,
    settings_loaded: bool,
}

struct UiState {
    root_element_id: Option<String>,
    panel: Panel,
    file_name: Option<String>,
    file_size_bytes: usize,
    file_text: Option<String>,
    split: SplitOptions,
    author: String,
    summary: String,
    cover_name: Option<String>,
    cover_bytes: Option<Vec<u8>>,
    pending_illustrations: Vec<PendingIllustration>,
    skip_synced: bool,
    chapter_estimate: Option<(usize, usize)>,
    progress: f32,
    speed_text: Option<String>,
    phase_text: Option<String>,
    status_message: Option<String>,
    status_tone: Tone,
    is_sending: bool,
    is_querying: bool,
    session: Option<Session>,
    pending_writes: Vec<(String, String)>,
    query_expect: u32,
    rd_incoming: Option<RdIncoming>,
    log: Vec<String>,
    band: BandInfo,
    disconnect_action: DisconnectAction,
    reconnect_attempts: usize,
    handling_offline: bool,
    last_inbound_at: Option<SystemTime>,
    offline_strikes: usize,
    hide_message_timer_id: Option<u64>,
    handshake_timer_id: Option<u64>,
    query_timer_id: Option<u64>,
    watchdog_timer_id: Option<u64>,
    reconnect_timer_id: Option<u64>,
    cancel_refresh_timer_id: Option<u64>,
    peer_version: Option<i64>,
    last_device_addr: Option<String>,
    started_at: Option<SystemTime>,
}

static UI_STATE: OnceLock<Mutex<UiState>> = OnceLock::new();

fn ui_state() -> &'static Mutex<UiState> {
    UI_STATE.get_or_init(|| {
        Mutex::new(UiState {
            root_element_id: None,
            panel: Panel::Send,
            file_name: None,
            file_size_bytes: 0,
            file_text: None,
            split: SplitOptions::default(),
            author: String::new(),
            summary: String::new(),
            cover_name: None,
            cover_bytes: None,
            pending_illustrations: Vec::new(),
            skip_synced: true,
            chapter_estimate: None,
            progress: 0.0,
            speed_text: None,
            phase_text: None,
            status_message: None,
            status_tone: Tone::Neutral,
            is_sending: false,
            is_querying: false,
            session: None,
            pending_writes: Vec::new(),
            query_expect: 0,
            rd_incoming: None,
            log: Vec::new(),
            band: BandInfo::default(),
            disconnect_action: DisconnectAction::Retry,
            reconnect_attempts: 0,
            handling_offline: false,
            last_inbound_at: None,
            offline_strikes: 0,
            hide_message_timer_id: None,
            handshake_timer_id: None,
            query_timer_id: None,
            watchdog_timer_id: None,
            reconnect_timer_id: None,
            cancel_refresh_timer_id: None,
            peer_version: None,
            last_device_addr: None,
            started_at: None,
        })
    })
}

fn state() -> MutexGuard<'static, UiState> {
    ui_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn last_device_addr() -> String {
    state().last_device_addr.clone().unwrap_or_default()
}

fn busy_now(s: &UiState) -> bool {
    s.is_sending || s.is_querying
}

// ---------------------------------------------------------------- 事件入口

pub fn ui_event_processor(evtype: ui::Event, event: &str, _event_payload: &str) {
    match evtype {
        ui::Event::Click => match event {
            EVENT_PICK_FILE => handle_pick_file(),
            EVENT_SEND_FILE => handle_send_file(),
            EVENT_CANCEL_SEND => handle_cancel(),
            EVENT_PICK_COVER => handle_pick_cover(),
            EVENT_CLEAR_COVER => handle_clear_cover(),
            EVENT_PICK_ILLUSTRATION => handle_pick_book_illustration(),
            EVENT_CLEAR_ILLUSTRATIONS => handle_clear_illustrations(),
            EVENT_CYCLE_SPLIT => handle_cycle_split(),
            EVENT_WORDS_UP => handle_words_step(500),
            EVENT_WORDS_DOWN => handle_words_step(-500),
            EVENT_TOGGLE_SKIP => handle_toggle_skip(),
            EVENT_PANEL_SEND => switch_panel(Panel::Send),
            EVENT_PANEL_DEVICE => switch_panel(Panel::Device),
            EVENT_PANEL_LOG => switch_panel(Panel::Log),
            EVENT_PANEL_ABOUT => switch_panel(Panel::About),
            EVENT_REFRESH_DEVICE => start_query(QueryKind::Status),
            EVENT_LOAD_SETTINGS => start_query(QueryKind::Settings),
            EVENT_CLEAR_LOG => handle_clear_log(),
            EVENT_EDIT_AUTHOR => handle_edit_author(),
            EVENT_EDIT_SUMMARY => handle_edit_summary(),
            EVENT_EDIT_KEYWORD => handle_edit_keyword(),
            EVENT_CYCLE_DISCONNECT => handle_cycle_disconnect(),
            EVENT_MGMT_DELETE_BOOK => handle_mgmt_delete_book(),
            EVENT_MGMT_DELETE_CHAPTERS => handle_mgmt_delete_chapters(),
            EVENT_MGMT_UPDATE_INFO => handle_mgmt_update_info(),
            EVENT_MGMT_READING_DATA => handle_mgmt_reading_data(),
            EVENT_MGMT_ILLUSTRATION => handle_mgmt_illustration(),
            _ => handle_dynamic_click(event),
        },
        _ => {}
    }
}

fn handle_dynamic_click(event: &str) {
    if let Some(key) = event.strip_prefix(SETTING_CLICK_PREFIX) {
        handle_setting_cycle(key);
    } else if let Some(key) = event.strip_prefix(SETTING_INC_PREFIX) {
        handle_setting_step(key, 1);
    } else if let Some(key) = event.strip_prefix(SETTING_DEC_PREFIX) {
        handle_setting_step(key, -1);
    }
}

pub fn handle_timer_event(event_payload: &str) {
    let payload = extract_payload_text(event_payload);
    match payload.as_str() {
        TIMER_HIDE_MESSAGE => {
            let should_render = {
                let mut s = state();
                s.status_message = None;
                s.status_tone = Tone::Neutral;
                s.hide_message_timer_id = None;
                s.root_element_id.is_some()
            };
            if should_render {
                render_from_state();
            }
        }
        TIMER_START_HANDSHAKE => {
            let device_addr = {
                let mut s = state();
                s.handshake_timer_id = None;
                let Some(session) = s.session.as_ref() else {
                    return;
                };
                session.device_addr().to_string()
            };

            {
                let mut s = state();
                schedule_handshake_timeout(&mut s);
            }

            send_handshake_message(&device_addr, 0);
        }
        TIMER_HANDSHAKE_TIMEOUT => {
            let (delegate, should_render) = {
                let mut s = state();
                s.handshake_timer_id = None;
                if s.is_sending {
                    log_line(&mut s, "[掉线] 握手超时");
                    (true, false)
                } else if s.is_querying {
                    set_status_message(&mut s, "手环未响应，请重试", Tone::Error, true);
                    log_line(&mut s, "[错误] 握手超时");
                    finish_transfer(&mut s, true);
                    (false, true)
                } else {
                    (false, false)
                }
            };
            if delegate {
                handle_link_lost("连接超时");
            } else if should_render {
                render_from_state();
            }
        }
        TIMER_TRANSFER_WATCHDOG => handle_watchdog(),
        TIMER_RECONNECT_ATTEMPT => attempt_reconnect(),
        TIMER_REFRESH_AFTER_CANCEL => {
            let should_go = {
                let mut s = state();
                s.cancel_refresh_timer_id = None;
                !busy_now(&s)
            };
            if should_go {
                refresh_device_status_if_online();
            }
        }
        TIMER_QUERY_TIMEOUT => {
            let should_render = {
                let mut s = state();
                s.query_timer_id = None;
                if s.is_querying {
                    set_status_message(&mut s, "手环未响应，请重试", Tone::Error, true);
                    log_line(&mut s, "[错误] 查询超时");
                    finish_transfer(&mut s, true);
                    true
                } else {
                    false
                }
            };
            if should_render {
                render_from_state();
            }
        }
        _ => {}
    }
}

pub fn handle_interconnect_message(event_payload: &str) {
    let payload = extract_payload_text(event_payload);
    let Ok(message) = serde_json::from_str::<Value>(&payload) else {
        tracing::warn!("无法解析互联消息: {}", payload);
        return;
    };

    {
        let mut s = state();
        s.last_inbound_at = Some(SystemTime::now());
    }

    let message = normalize_message(message);

    let tag = message.get("tag").and_then(|v| v.as_str()).unwrap_or("");
    match tag {
        TAG_HANDSHAKE => handle_handshake_message(&message),
        TAG_FILE => handle_file_message(&message),
        _ => {}
    }
}

fn normalize_message(mut message: Value) -> Value {
    let Some(obj) = message.as_object_mut() else {
        return message;
    };
    let Some(channel) = obj
        .get("k")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        return message;
    };
    obj.remove("k");

    match channel.as_str() {
        protocol::CHANNEL_HANDSHAKE => {
            obj.insert("tag".to_string(), json!(TAG_HANDSHAKE));
            if let Some(n) = obj.remove("n") {
                obj.insert("count".to_string(), n);
            }
            if let Some(v) = obj.remove("v") {
                obj.insert("version".to_string(), v);
            }
        }
        protocol::CHANNEL_FILE => {
            obj.insert("tag".to_string(), json!(TAG_FILE));
            obj.remove("q");
            let type_name = obj
                .remove("a")
                .and_then(|v| v.as_str().map(str::to_string))
                .and_then(|code| protocol::reply_code_name(&code).map(str::to_string))
                .unwrap_or_default();
            obj.insert("type".to_string(), json!(type_name));
        }
        _ => {}
    }
    message
}

// ---------------------------------------------------------------- 界面切换

fn switch_panel(panel: Panel) {
    {
        let mut s = state();
        s.panel = panel;
    }
    render_from_state();
}

fn handle_clear_log() {
    {
        let mut s = state();
        s.log.clear();
    }
    render_from_state();
}

// ---------------------------------------------------------------- 文本编辑

/// 按当前分章方式刷新「预计章数 / 字数」预览
fn recompute_estimate(s: &mut UiState) {
    s.chapter_estimate = match (&s.file_text, &s.file_name) {
        (Some(text), Some(name)) => {
            let chapters = chapters::split_chapters_with(text, name, &s.split);
            let words: usize = chapters.iter().map(|c| c.word_count).sum();
            Some((chapters.len(), words))
        }
        _ => None,
    };
}

/// 用弹窗输入文本；取消返回 `None`
fn prompt_text(title: &str, tip: &str) -> Option<String> {
    let result = wit_bindgen::block_on(async {
        dialog::show_dialog(
            dialog::DialogType::Input,
            dialog::DialogStyle::Website,
            &dialog::DialogInfo {
                title: title.to_string(),
                content: tip.to_string(),
                buttons: vec![
                    dialog::DialogButton {
                        id: "ok".to_string(),
                        primary: true,
                        content: "确定".to_string(),
                    },
                    dialog::DialogButton {
                        id: "cancel".to_string(),
                        primary: false,
                        content: "取消".to_string(),
                    },
                ],
            },
        )
        .await
    });

    if result.clicked_btn_id != "ok" {
        return None;
    }
    Some(result.input_result.trim().to_string())
}

fn handle_edit_author() {
    let Some(value) = prompt_text("作者", "留空表示不写入手环书库") else {
        return;
    };
    {
        let mut s = state();
        s.author = value;
    }
    render_from_state();
}

fn handle_edit_summary() {
    let Some(value) = prompt_text("简介", "留空表示不写入手环书库") else {
        return;
    };
    {
        let mut s = state();
        s.summary = value;
    }
    render_from_state();
}

fn handle_edit_keyword() {
    let Some(value) = prompt_text("行首关键字", "多个关键字用 | 分隔，例如：卷|###") else {
        return;
    };
    {
        let mut s = state();
        let shown = if value.is_empty() {
            "未设置".to_string()
        } else {
            value.clone()
        };
        s.split.keyword = value;
        recompute_estimate(&mut s);
        log_line(&mut s, &format!("[分章] 关键字：{}", shown));
    }
    render_from_state();
}

// ---------------------------------------------------------------- 选择文件

fn handle_pick_file() {
    {
        let s = state();
        if busy_now(&s) {
            return;
        }
    }

    let pick_result = wit_bindgen::block_on(async {
        let pick_config = dialog::PickConfig {
            read: true,
            copy_to: None,
        };
        let filter_config = dialog::FilterConfig {
            multiple: false,
            extensions: vec!["txt".to_string()],
            default_directory: "".to_string(),
            default_file_name: "".to_string(),
        };
        dialog::pick_file(&pick_config, &filter_config).await
    });

    let name = pick_result.name;
    let bytes = pick_result.data;
    if name.trim().is_empty() && bytes.is_empty() {
        return;
    }

    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            {
                let mut s = state();
                set_status_message(
                    &mut s,
                    "文件不是 UTF-8 文本，请另存为 UTF-8 后重试",
                    Tone::Error,
                    true,
                );
                log_line(&mut s, "[错误] 文件不是 UTF-8");
            }
            render_from_state();
            return;
        }
    };

    let size = text.as_bytes().len();

    {
        let mut s = state();
        s.file_size_bytes = size;
        s.file_text = Some(text);
        s.file_name = Some(name.clone());
        s.progress = 0.0;
        s.speed_text = None;
        s.phase_text = None;
        s.status_message = None;
        s.status_tone = Tone::Neutral;
        s.band.status_known = false;
        s.band.synced_chapters.clear();
        s.pending_illustrations.clear();
        log_line(&mut s, &format!("[文件] {} ({})", name, format_bytes(size)));
        recompute_estimate(&mut s);
    }
    render_from_state();
    refresh_device_status_if_online();
}

fn refresh_device_status_if_online() {
    let has_device = wit_bindgen::block_on(async { device::get_connected_device_list().await })
        .first()
        .is_some();

    if !has_device {
        let should_render = {
            let mut s = state();
            log_line(&mut s, "[设备] 未连接，跳过自动刷新");
            s.root_element_id.is_some()
        };
        if should_render {
            render_from_state();
        }
        return;
    }

    start_query(QueryKind::Status);
}

fn handle_pick_cover() {
    {
        let s = state();
        if busy_now(&s) {
            return;
        }
    }

    let pick_result = wit_bindgen::block_on(async {
        let pick_config = dialog::PickConfig {
            read: true,
            copy_to: None,
        };
        let filter_config = dialog::FilterConfig {
            multiple: false,
            extensions: vec!["jpg".to_string(), "jpeg".to_string(), "png".to_string()],
            default_directory: "".to_string(),
            default_file_name: "".to_string(),
        };
        dialog::pick_file(&pick_config, &filter_config).await
    });

    let name = pick_result.name;
    let mut bytes = pick_result.data;
    if bytes.is_empty() {
        return;
    }

    let mut notice: Option<(String, Tone)> = None;
    if bytes.len() > COVER_MAX_BYTES {
        bytes.clear();
        notice = Some((
            format!("封面超过 {}，已忽略", format_bytes(COVER_MAX_BYTES)),
            Tone::Error,
        ));
    }

    {
        let mut s = state();
        if notice.is_some() {
            s.cover_name = None;
            s.cover_bytes = None;
        } else {
            s.cover_name = Some(name.clone());
            s.cover_bytes = Some(bytes.clone());
            log_line(
                &mut s,
                &format!("[封面] {} ({})", name, format_bytes(bytes.len())),
            );
        }
        if let Some((msg, tone)) = notice {
            set_status_message(&mut s, &msg, tone, true);
        } else {
            s.status_message = None;
            s.status_tone = Tone::Neutral;
        }
    }
    render_from_state();
}

fn handle_clear_cover() {
    {
        let mut s = state();
        s.cover_name = None;
        s.cover_bytes = None;
        log_line(&mut s, "[封面] 已清除");
    }
    render_from_state();
}

fn handle_pick_book_illustration() {
    if busy_prompt() {
        render_from_state();
        return;
    }

    let pick_result = wit_bindgen::block_on(async {
        let pick_config = dialog::PickConfig {
            read: true,
            copy_to: None,
        };
        let filter_config = dialog::FilterConfig {
            multiple: false,
            extensions: vec!["jpg".to_string(), "jpeg".to_string(), "png".to_string()],
            default_directory: "".to_string(),
            default_file_name: "".to_string(),
        };
        dialog::pick_file(&pick_config, &filter_config).await
    });
    let image_name = pick_result.name;
    let bytes = pick_result.data;
    if bytes.is_empty() {
        return;
    }
    if bytes.len() > ILLUSTRATION_MAX_BYTES {
        let mut s = state();
        set_status_message(
            &mut s,
            &format!("插图超过 {}，已取消", format_bytes(ILLUSTRATION_MAX_BYTES)),
            Tone::Error,
            true,
        );
        render_from_state();
        return;
    }
    let base_name = image_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if base_name.is_empty() || base_name == "." || base_name == ".." {
        let mut s = state();
        set_status_message(&mut s, "插图文件名无效", Tone::Error, true);
        render_from_state();
        return;
    }
    let relative_path = format!("images/{}", base_name);
    let size = bytes.len();

    let mut s = state();
    let replaced = s
        .pending_illustrations
        .iter()
        .position(|item| item.relative_path == relative_path);
    let action = match replaced {
        Some(pos) => {
            s.pending_illustrations[pos] = PendingIllustration {
                relative_path: relative_path.clone(),
                bytes,
            };
            "替换"
        }
        None => {
            s.pending_illustrations.push(PendingIllustration {
                relative_path: relative_path.clone(),
                bytes,
            });
            "加入"
        }
    };
    let total = s.pending_illustrations.len();
    log_line(
        &mut s,
        &format!(
            "[插图] {} {}（{}），队列 {} 张",
            action,
            relative_path,
            format_bytes(size),
            total
        ),
    );
    s.status_message = None;
    s.status_tone = Tone::Neutral;
    render_from_state();
}

fn handle_clear_illustrations() {
    {
        let mut s = state();
        if s.pending_illustrations.is_empty() {
            return;
        }
        let count = s.pending_illustrations.len();
        s.pending_illustrations.clear();
        log_line(&mut s, &format!("[插图] 已清空队列（{} 张）", count));
    }
    render_from_state();
}

// ---------------------------------------------------------------- 传输选项

fn handle_cycle_split() {
    {
        let mut s = state();
        if busy_now(&s) {
            return;
        }
        s.split.mode = s.split.mode.next();
        let label = s.split.mode.label();
        log_line(&mut s, &format!("[分章] {}", label));
        recompute_estimate(&mut s);
    }
    render_from_state();
}

fn handle_words_step(delta: i64) {
    {
        let mut s = state();
        let next = s.split.words_per_chapter as i64 + delta;
        s.split.words_per_chapter = next.clamp(500, 20000) as usize;
        recompute_estimate(&mut s);
    }
    render_from_state();
}

fn handle_cycle_disconnect() {
    {
        let mut s = state();
        s.disconnect_action = s.disconnect_action.next();
        let (label, hint) = (s.disconnect_action.label(), s.disconnect_action.hint());
        log_line(&mut s, &format!("[设置] 掉线处理：{} · {}", label, hint));
    }
    render_from_state();
}

fn handle_toggle_skip() {
    {
        let mut s = state();
        s.skip_synced = !s.skip_synced;
        let on = s.skip_synced;
        log_line(
            &mut s,
            &format!("[选项] 跳过已同步章节：{}", if on { "开" } else { "关" }),
        );
    }
    render_from_state();
}

// ---------------------------------------------------------------- 查询流程

fn start_query(kind: QueryKind) {
    let already_busy = {
        let s = state();
        busy_now(&s)
    };
    if already_busy {
        {
            let mut s = state();
            set_status_message(&mut s, "正在忙，请稍候", Tone::Neutral, true);
        }
        render_from_state();
        return;
    }

    let filename = { state().file_name.clone() };

    {
        let mut s = state();
        s.is_querying = true;
        s.status_message = None;
        s.status_tone = Tone::Neutral;
        s.query_expect = 0;
    }
    render_from_state();

    if !prepare_connection() {
        return;
    }

    let session = Session {
        device_addr: last_device_addr(),
        job: Job::Query(kind),
        pending_start: true,
        handshake_complete: false,
    };

    {
        let mut s = state();
        s.session = Some(session);
        let timer_id =
            wit_bindgen::block_on(async { timer::set_timeout(1200, TIMER_START_HANDSHAKE).await });
        s.handshake_timer_id = Some(timer_id);
        let log_text = match &s.session.as_ref().unwrap().job {
            Job::Query(QueryKind::Status) => {
                let mut log = String::from("[设备] 查询存储");
                if filename.is_some() {
                    log.push_str("与书籍同步状态");
                }
                log
            }
            Job::Query(QueryKind::Settings) => "[设备] 读取手环阅读设置".to_string(),
            Job::Query(QueryKind::DeleteBook(_)) => "[管理] 删除书籍".to_string(),
            Job::Query(QueryKind::DeleteChapters(_, idx)) => {
                format!("[管理] 删除 {} 个章节", idx.len())
            }
            Job::Query(QueryKind::UpdateBookInfo { .. }) => "[管理] 更新书籍信息".to_string(),
            Job::Query(QueryKind::GetReadingData(_)) => "[管理] 查询阅读进度".to_string(),
            Job::Query(QueryKind::SendIllustration(job)) => format!(
                "[管理] 传输插图 {}（{} 块）",
                job.relative_path,
                job.chunks.len()
            ),
            Job::Plus(_) => String::new(),
        };
        if !log_text.is_empty() {
            log_line(&mut s, &log_text);
        }
    }
    render_from_state();
}

/// 发送设置写入
fn queue_setting_write(key: &str, value: &str) {
    let (direct_send, need_session) = {
        let mut s = state();
        let ready = s
            .session
            .as_ref()
            .map(|sess| sess.handshake_complete && sess.query_kind() == Some(QueryKind::Settings))
            .unwrap_or(false);
        if ready {
            (true, false)
        } else {
            s.pending_writes
                .push((key.to_string(), value.to_string()));
            (!s.is_querying, !s.is_querying)
        }
    };

    if direct_send {
        let addr = last_device_addr();
        send_to(&addr, &protocol::plus_set_setting(key, value));
        return;
    }
    if need_session {
        start_query(QueryKind::Settings);
    }
}

fn handle_setting_cycle(key: &str) {
    let Some(def) = band_setting(key) else {
        return;
    };
    let current = read_setting_text(&state(), key);
    let next = match &def.kind {
        SettingKind::Bool => {
            if current == "false" {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        SettingKind::Choice(opts) => {
            let idx = opts.iter().position(|(k, _)| *k == current).unwrap_or(0);
            let next_idx = (idx + 1) % opts.len();
            opts[next_idx].0.to_string()
        }
        SettingKind::Int { .. } => return,
    };

    {
        let mut s = state();
        s.band.settings.insert(key.to_string(), next.clone());
        let shown = def_value_text(def, &next);
        log_line(&mut s, &format!("[设置] {} → {}", def.label, shown));
    }
    queue_setting_write(key, &next);
    render_from_state();
}

fn handle_setting_step(key: &str, dir: i64) {
    let Some(def) = band_setting(key) else {
        return;
    };
    let SettingKind::Int {
        min, max, step, ..
    } = &def.kind
    else {
        return;
    };
    let (min, max, step) = (*min, *max, *step);
    let current = read_setting_text(&state(), key)
        .parse::<i64>()
        .unwrap_or_else(|_| def.default.parse().unwrap_or(min));
    let next = (current + dir * step).clamp(min, max);

    {
        let mut s = state();
        s.band.settings.insert(key.to_string(), next.to_string());
        let shown = def_value_text(def, &next.to_string());
        log_line(&mut s, &format!("[设置] {} → {}", def.label, shown));
    }
    queue_setting_write(key, &next.to_string());
    render_from_state();
}

fn read_setting_text(s: &UiState, key: &str) -> String {
    s.band
        .settings
        .get(key)
        .cloned()
        .or_else(|| band_setting(key).map(|d| d.default.to_string()))
        .unwrap_or_default()
}

fn def_value_text(def: &SettingDef, raw: &str) -> String {
    match &def.kind {
        SettingKind::Bool => {
            if raw == "false" {
                "关".to_string()
            } else {
                "开".to_string()
            }
        }
        SettingKind::Int { suffix, .. } => format!("{}{}", raw, suffix),
        SettingKind::Choice(opts) => opts
            .iter()
            .find(|(k, _)| *k == raw)
            .map(|(_, label)| label.to_string())
            .unwrap_or_else(|| raw.to_string()),
    }
}

// ---------------------------------------------------------------- 发送流程

fn handle_send_file() {
    let prepared = {
        let s = state();
        if busy_now(&s) {
            return;
        }
        match (&s.file_name, &s.file_text) {
            (Some(name), Some(text)) => Some((
                name.clone(),
                text.clone(),
                s.file_size_bytes,
                s.split.clone(),
                s.author.clone(),
                s.summary.clone(),
                s.cover_bytes.clone(),
                s.skip_synced,
                s.band.synced_chapters.clone(),
            )),
            _ => None,
        }
    };

    let Some((
        file_name,
        file_text,
        file_size,
        split,
        author,
        summary,
        cover_bytes,
        skip_synced,
        synced,
    )) = prepared
    else {
        {
            let mut s = state();
            set_status_message(&mut s, "请先选择文件", Tone::Error, true);
        }
        render_from_state();
        return;
    };

    {
        let mut s = state();
        s.is_sending = true;
        s.progress = 0.0;
        s.speed_text = None;
        s.phase_text = Some("准备连接手环".to_string());
        set_status_message(&mut s, "准备发送中...", Tone::Neutral, false);
        s.reconnect_attempts = 0;
        s.handling_offline = false;
        s.offline_strikes = 0;
        s.last_inbound_at = Some(SystemTime::now());
        arm_watchdog(&mut s);
    }
    render_from_state();

    if !prepare_connection() {
        return;
    }
    mark_link_activity();

    let session = match build_session(
        last_device_addr(),
        file_name,
        file_text,
        file_size,
        &split,
        &author,
        &summary,
        cover_bytes,
        skip_synced,
        &synced,
    ) {
        Ok(session) => session,
        Err(msg) => {
            fail_with(&msg);
            return;
        }
    };

    {
        let mut s = state();
        s.session = Some(session);
        let timer_id =
            wit_bindgen::block_on(async { timer::set_timeout(1500, TIMER_START_HANDSHAKE).await });
        s.handshake_timer_id = Some(timer_id);
        log_line(&mut s, "[连接] 等待手环端响应");
    }
    render_from_state();
}

fn connect_device() -> Result<String, String> {
    let device_addr = get_device_addr()?;

    let app = get_app_info(&device_addr)?;

    let launch_ok =
        wit_bindgen::block_on(async { thirdpartyapp::launch_qa(&device_addr, &app, "/index").await })
            .is_ok();
    if !launch_ok {
        return Err("启动应用失败，请确认手环端已安装并授权".to_string());
    }

    let _ = wit_bindgen::block_on(async {
        register::register_interconnect_recv(&device_addr, protocol::PACKAGE).await
    });

    Ok(device_addr)
}

fn prepare_connection() -> bool {
    match connect_device() {
        Ok(device_addr) => {
            {
                let mut s = state();
                s.last_device_addr = Some(device_addr);
            }
            true
        }
        Err(msg) => {
            fail_with(&msg);
            false
        }
    }
}

fn handle_cancel() {
    let cancel_target = {
        let s = state();
        if !busy_now(&s) {
            return;
        }
        s.session
            .as_ref()
            .map(|session| session.device_addr().to_string())
    };

    if let Some(device_addr) = cancel_target {
        send_to(&device_addr, &protocol::cancel());
    }

    {
        let mut s = state();
        set_status_message(&mut s, "已取消", Tone::Neutral, true);
        log_line(&mut s, "[操作] 已取消");
        finish_transfer(&mut s, true);
        schedule_cancel_refresh(&mut s);
    }
    render_from_state();
}

// ---------------------------------------------------------------- 掉线检测与重连

fn schedule_cancel_refresh(state: &mut UiState) {
    if state.cancel_refresh_timer_id.is_some() {
        return;
    }
    let timer_id = wit_bindgen::block_on(async {
        timer::set_timeout(CANCEL_REFRESH_DELAY_MS, TIMER_REFRESH_AFTER_CANCEL).await
    });
    state.cancel_refresh_timer_id = Some(timer_id);
}

fn arm_watchdog(state: &mut UiState) {
    disarm_watchdog(state);
    let timer_id = wit_bindgen::block_on(async {
        timer::set_timeout(WATCHDOG_INTERVAL_MS, TIMER_TRANSFER_WATCHDOG).await
    });
    state.watchdog_timer_id = Some(timer_id);
}

fn disarm_watchdog(state: &mut UiState) {
    if let Some(timer_id) = state.watchdog_timer_id.take() {
        let _ = wit_bindgen::block_on(async { timer::clear_timer(timer_id).await });
    }
}

fn arm_reconnect_timer(state: &mut UiState) {
    disarm_reconnect_timer(state);
    let timer_id = wit_bindgen::block_on(async {
        timer::set_timeout(RECONNECT_DELAY_MS, TIMER_RECONNECT_ATTEMPT).await
    });
    state.reconnect_timer_id = Some(timer_id);
}

fn disarm_reconnect_timer(state: &mut UiState) {
    if let Some(timer_id) = state.reconnect_timer_id.take() {
        let _ = wit_bindgen::block_on(async { timer::clear_timer(timer_id).await });
    }
}

fn device_still_connected(addr: &str) -> bool {
    let devices = wit_bindgen::block_on(async { device::get_connected_device_list().await });
    if devices.is_empty() {
        return false;
    }
    addr.is_empty() || devices.iter().any(|device| device.addr == addr)
}

fn mark_link_activity() {
    let mut s = state();
    s.last_inbound_at = Some(SystemTime::now());
}

fn handle_watchdog() {
    let (addr, stalled) = {
        let mut s = state();
        s.watchdog_timer_id = None;
        if !s.is_sending {
            return;
        }
        let stalled = s
            .last_inbound_at
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|elapsed| elapsed.as_secs() >= STALL_TIMEOUT_SECS)
            .unwrap_or(false);
        (s.last_device_addr.clone().unwrap_or_default(), stalled)
    };

    if !device_still_connected(&addr) {
        let strikes = {
            let mut s = state();
            s.offline_strikes += 1;
            s.offline_strikes
        };
        if strikes >= OFFLINE_CONFIRM_STRIKES {
            handle_link_lost("设备连接已断开");
            return;
        }
    } else {
        {
            let mut s = state();
            s.offline_strikes = 0;
        }
        if stalled {
            handle_link_lost(&format!("手环 {} 秒无响应", STALL_TIMEOUT_SECS));
            return;
        }
    }

    let mut s = state();
    if s.is_sending {
        arm_watchdog(&mut s);
    }
}

fn handle_link_lost(reason: &str) {
    let action = {
        let mut s = state();
        if !s.is_sending {
            disarm_watchdog(&mut s);
            return;
        }
        if s.handling_offline {
            return;
        }
        s.handling_offline = true;
        disarm_watchdog(&mut s);
        s.disconnect_action
    };

    match action {
        DisconnectAction::Cancel => {
            let addr = last_device_addr();
            if !addr.is_empty() {
                send_to(&addr, &protocol::cancel());
            }
            {
                let mut s = state();
                s.handling_offline = false;
                s.reconnect_attempts = 0;
                set_status_message(
                    &mut s,
                    &format!("{}，已取消传输", reason),
                    Tone::Error,
                    true,
                );
                log_line(&mut s, &format!("[掉线] {}，按设置取消传输", reason));
                finish_transfer(&mut s, true);
                schedule_cancel_refresh(&mut s);
            }
            render_from_state();
        }
        DisconnectAction::Retry => retry_reconnect(reason),
    }
}

fn retry_reconnect(reason: &str) {
    let attempts = {
        let mut s = state();
        s.reconnect_attempts += 1;
        s.reconnect_attempts
    };

    if attempts > MAX_RECONNECT {
        {
            let mut s = state();
            s.handling_offline = false;
            s.reconnect_attempts = 0;
        }
        let hint = if reason.contains("超时") {
            "，请确认手环端应用已打开并停留在前台"
        } else {
            ""
        };
        fail_with(&format!(
            "{}，已重连 {} 次仍失败{}",
            reason, MAX_RECONNECT, hint
        ));
        return;
    }

    {
        let mut s = state();
        if let Some(session) = s.session.as_mut() {
            session.pending_start = true;
            session.handshake_complete = false;
            if let Job::Plus(job) = &mut session.job {
                if !job.cover_done {
                    job.cover_idx = 0;
                }
            }
        }
        s.phase_text = Some(format!("重新连接 ({}/{})", attempts, MAX_RECONNECT));
        set_status_message(
            &mut s,
            &format!("{}，正在重连 {}/{}", reason, attempts, MAX_RECONNECT),
            Tone::Neutral,
            false,
        );
        log_line(
            &mut s,
            &format!(
                "[掉线] {}，第 {}/{} 次尝试重连",
                reason, attempts, MAX_RECONNECT
            ),
        );
        arm_reconnect_timer(&mut s);
    }
    render_from_state();
}

fn attempt_reconnect() {
    {
        let mut s = state();
        s.reconnect_timer_id = None;
        if !s.is_sending || !s.handling_offline {
            return;
        }
    }

    match connect_device() {
        Ok(device_addr) => {
            {
                let mut s = state();
                s.handling_offline = false;
                s.offline_strikes = 0;
                s.last_device_addr = Some(device_addr);
                s.last_inbound_at = Some(SystemTime::now());
                s.phase_text = Some("重连成功，恢复传输".to_string());
                log_line(&mut s, "[掉线] 已重新连接，等待手环响应");
                let timer_id = wit_bindgen::block_on(async {
                    timer::set_timeout(1500, TIMER_START_HANDSHAKE).await
                });
                s.handshake_timer_id = Some(timer_id);
                arm_watchdog(&mut s);
            }
            render_from_state();
        }
        Err(msg) => retry_reconnect(&msg),
    }
}

// ---------------------------------------------------------------- 握手

fn handle_handshake_message(message: &Value) {
    let count = message.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    if let Some(peer_version) = extract_version(message) {
        let mut s = state();
        s.peer_version = Some(s.peer_version.map_or(peer_version, |old| old.max(peer_version)));
    }

    let (device_addr, should_start, should_reply, timer_to_clear, query_kind) = {
        let mut s = state();
        let Some(session) = s.session.as_mut() else {
            return;
        };
        let should_start = session.handshake_progress(count);
        let device_addr = session.device_addr().to_string();
        let query_kind = session.query_kind();
        let should_reply = count < 2;
        let timer_to_clear = if count > 0 {
            s.handshake_timer_id.take()
        } else {
            None
        };
        (
            device_addr,
            should_start,
            should_reply,
            timer_to_clear,
            query_kind,
        )
    };

    if let Some(timer_id) = timer_to_clear {
        let _ = wit_bindgen::block_on(async { timer::clear_timer(timer_id).await });
    }

    if should_reply {
        send_handshake_message(&device_addr, count + 1);
    }

    if should_start {
        match query_kind {
            Some(kind) => {
                {
                    let mut s = state();
                    s.phase_text = Some("已连接".to_string());
                    log_line(&mut s, "[连接] 已连接手环");
                }
                render_from_state();
                start_query_messages(kind, &device_addr);
            }
            None => {
                {
                    let mut s = state();
                    s.phase_text = Some("已连接，开始传输".to_string());
                    set_status_message(&mut s, "已连接，开始传输...", Tone::Neutral, false);
                }
                render_from_state();
                session_start_transfer();
            }
        }
    }
}

fn send_handshake_message(device_addr: &str, count: usize) {
    let peer_version = state().peer_version;
    send_to(device_addr, &protocol::handshake(count, peer_version));
}

fn extract_version(message: &Value) -> Option<i64> {
    let value = message.get("version")?;
    if let Some(n) = value.as_i64() {
        return Some(n);
    }
    if let Some(f) = value.as_f64() {
        return Some(f as i64);
    }
    value.as_str().and_then(|s| s.trim().parse::<i64>().ok())
}

// ---------------------------------------------------------------- 查询下发

fn start_query_messages(kind: QueryKind, device_addr: &str) {
    let mut expect = 0u32;

    match kind {
        QueryKind::Status => {
            send_to(device_addr, &protocol::plus_get_storage_info());
            expect += 1;
            let file_name = { state().file_name.clone() };
            if let Some(name) = file_name {
                send_to(device_addr, &protocol::plus_get_book_status(&name));
                expect += 1;
            }
        }
        QueryKind::Settings => {
            let pending = {
                let mut s = state();
                s.pending_writes.drain(..).collect::<Vec<_>>()
            };
            if pending.is_empty() {
                let keys: Vec<&str> = BAND_SETTINGS.iter().map(|d| d.key).collect();
                send_to(device_addr, &protocol::plus_get_settings(&keys));
                expect += 1;
            } else {
                for (key, value) in &pending {
                    send_to(device_addr, &protocol::plus_set_setting(key, value));
                }
                expect += 1;
            }
        }
        QueryKind::DeleteBook(name) => {
            send_to(device_addr, &protocol::plus_delete_book(&name));
            expect += 1;
        }
        QueryKind::DeleteChapters(name, indices) => {
            send_to(device_addr, &protocol::plus_delete_chapters(&name, &indices));
            expect += 1;
        }
        QueryKind::UpdateBookInfo {
            filename,
            author,
            summary,
        } => {
            send_to(
                device_addr,
                &protocol::plus_update_book_info(&filename, Some(&author), Some(&summary)),
            );
            expect += 1;
        }
        QueryKind::GetReadingData(name) => {
            send_to(device_addr, &protocol::plus_get_reading_data(&name));
            expect += 1;
        }
        QueryKind::SendIllustration(job) => {
            send_to(
                device_addr,
                &protocol::plus_start_illustration_transfer(
                    &job.filename,
                    &job.relative_path,
                    &job.fingerprint,
                    job.total_bytes,
                    job.chunks.len(),
                    job.chunk_size,
                ),
            );
            expect += 1;
        }
    }

    let should_render = {
        let mut s = state();
        s.query_expect = expect;
        if expect == 0 {
            set_status_message(&mut s, "查询完成", Tone::Success, true);
            finish_transfer(&mut s, false);
            true
        } else {
            let timer_id =
                wit_bindgen::block_on(async { timer::set_timeout(5000, TIMER_QUERY_TIMEOUT).await });
            s.query_timer_id = Some(timer_id);
            s.phase_text = Some("读取中".to_string());
            false
        }
    };
    if should_render {
        render_from_state();
    }
}

fn settle_query_expect() {
    let should_render = {
        let mut s = state();
        if !s.is_querying || s.query_expect == 0 {
            return;
        }
        s.query_expect -= 1;
        if s.query_expect > 0 {
            return;
        }
        clear_query_timer(&mut s);
        set_status_message(&mut s, "手环信息已更新", Tone::Success, true);
        s.phase_text = None;
        finish_transfer(&mut s, false);
        true
    };
    if should_render {
        render_from_state();
    }
}

// ---------------------------------------------------------------- 书库管理

fn busy_prompt() -> bool {
    let already_busy = {
        let s = state();
        busy_now(&s)
    };
    if already_busy {
        let mut s = state();
        set_status_message(&mut s, "正在忙，请稍候", Tone::Neutral, true);
    }
    already_busy
}

fn resolve_mgmt_filename(input: String) -> Option<String> {
    let candidate = if input.trim().is_empty() {
        state().file_name.clone()
    } else {
        Some(input.trim().to_string())
    };
    if candidate.as_deref().map(str::is_empty).unwrap_or(true) {
        let mut s = state();
        set_status_message(&mut s, "未指定书名", Tone::Error, true);
        render_from_state();
        return None;
    }
    candidate
}

fn handle_mgmt_delete_book() {
    if busy_prompt() {
        render_from_state();
        return;
    }
    let Some(filename) = prompt_text("删除书籍", "输入手环上的书名（需与同步时的书名一致）")
    else {
        return;
    };
    let Some(filename) = resolve_mgmt_filename(filename) else {
        return;
    };
    let Some(confirm) = prompt_text(
        "确认删除",
        &format!("将删除手环上《{}》的全部内容，且无法恢复；输入 Y 确认", filename),
    ) else {
        return;
    };
    if !confirm.eq_ignore_ascii_case("y") {
        let mut s = state();
        set_status_message(&mut s, "已取消删除", Tone::Neutral, true);
        render_from_state();
        return;
    }
    start_query(QueryKind::DeleteBook(filename));
}

fn handle_mgmt_delete_chapters() {
    if busy_prompt() {
        render_from_state();
        return;
    }
    let Some(filename) = prompt_text("删除章节", "输入手环上的书名") else {
        return;
    };
    let Some(filename) = resolve_mgmt_filename(filename) else {
        return;
    };
    let Some(text) = prompt_text(
        "章节下标",
        "输入要删除的章节下标（从 0 开始），多个用逗号分隔，如：0,3,7",
    ) else {
        return;
    };
    let indices: Vec<usize> = text
        .split(|c| c == ',' || c == '，' || c == '、' || c == ' ')
        .filter_map(|part| part.trim().parse::<usize>().ok())
        .collect();
    if indices.is_empty() {
        let mut s = state();
        set_status_message(&mut s, "未解析到有效章节下标", Tone::Error, true);
        render_from_state();
        return;
    }
    start_query(QueryKind::DeleteChapters(filename, indices));
}

fn handle_mgmt_update_info() {
    if busy_prompt() {
        render_from_state();
        return;
    }
    let Some(filename) = prompt_text("更新书籍信息", "输入手环上的书名") else {
        return;
    };
    let Some(filename) = resolve_mgmt_filename(filename) else {
        return;
    };
    let Some(author) = prompt_text("作者", "新作者；留空表示不修改") else {
        return;
    };
    let Some(summary) = prompt_text("简介", "新简介；留空表示不修改") else {
        return;
    };
    if author.trim().is_empty() && summary.trim().is_empty() {
        let mut s = state();
        set_status_message(&mut s, "作者与简介均为空，无需更新", Tone::Neutral, true);
        render_from_state();
        return;
    }
    start_query(QueryKind::UpdateBookInfo {
        filename,
        author: author.trim().to_string(),
        summary: summary.trim().to_string(),
    });
}

fn handle_mgmt_reading_data() {
    if busy_prompt() {
        render_from_state();
        return;
    }
    let Some(input) = prompt_text(
        "查询阅读进度",
        "输入手环上的书名；留空则使用当前选择的书籍",
    ) else {
        return;
    };
    let Some(filename) = resolve_mgmt_filename(input) else {
        return;
    };
    {
        let mut s = state();
        s.rd_incoming = None;
    }
    start_query(QueryKind::GetReadingData(filename));
}

fn handle_mgmt_illustration() {
    if busy_prompt() {
        render_from_state();
        return;
    }

    let pick_result = wit_bindgen::block_on(async {
        let pick_config = dialog::PickConfig {
            read: true,
            copy_to: None,
        };
        let filter_config = dialog::FilterConfig {
            multiple: false,
            extensions: vec!["jpg".to_string(), "jpeg".to_string(), "png".to_string()],
            default_directory: "".to_string(),
            default_file_name: "".to_string(),
        };
        dialog::pick_file(&pick_config, &filter_config).await
    });
    let image_name = pick_result.name;
    let bytes = pick_result.data;
    if bytes.is_empty() {
        return;
    }
    if bytes.len() > ILLUSTRATION_MAX_BYTES {
        let mut s = state();
        set_status_message(
            &mut s,
            &format!("插图超过 {}，已取消", format_bytes(ILLUSTRATION_MAX_BYTES)),
            Tone::Error,
            true,
        );
        render_from_state();
        return;
    }

    let Some(input) = prompt_text(
        "目标书籍",
        "输入手环上的书名；留空则使用当前选择的书籍",
    ) else {
        return;
    };
    let Some(filename) = resolve_mgmt_filename(input) else {
        return;
    };
    let Some(relative_path) = prompt_text(
        "插图路径",
        &format!(
            "书内相对路径（不以 / 开头，不含 ..），留空则使用 {}",
            image_name
        ),
    ) else {
        return;
    };
    let relative_path = if relative_path.trim().is_empty() {
        image_name.clone()
    } else {
        relative_path.trim().trim_start_matches('/').to_string()
    };
    let segments: Vec<&str> = relative_path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() || segments.iter().any(|s| *s == "." || *s == "..") {
        let mut s = state();
        set_status_message(&mut s, "插图路径无效", Tone::Error, true);
        render_from_state();
        return;
    }
    let relative_path = segments.join("/");

    let b64 = protocol::base64_encode(&bytes);
    let chunks = chunk_string(&b64, COVER_CHUNK_CHARS);

    start_query(QueryKind::SendIllustration(IllustrationJob {
        filename,
        relative_path,
        fingerprint: protocol::fingerprint(&bytes),
        chunks,
        total_bytes: bytes.len(),
        chunk_size: COVER_CHUNK_CHARS,
        idx: 0,
    }));
}

fn plus_send_illustration_chunk() {
    enum IlluStep {
        Chunk(String),
        Finish(String),
    }
    let device_addr = last_device_addr();
    let step = {
        let mut s = state();
        let Some(Session {
            job: Job::Query(QueryKind::SendIllustration(job)),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };
        if job.idx < job.chunks.len() {
            let idx = job.idx;
            let total = job.chunks.len();
            let message = protocol::plus_illustration_chunk(
                &job.relative_path,
                idx,
                total,
                &job.chunks[idx],
            );
            s.phase_text = Some(format!("发送插图 {}/{}", idx + 1, total));
            IlluStep::Chunk(message)
        } else {
            let message = protocol::plus_illustration_transfer_complete(&job.relative_path);
            s.phase_text = Some("插图收尾".to_string());
            IlluStep::Finish(message)
        }
    };

    match step {
        IlluStep::Chunk(message) => {
            if !send_to(&device_addr, &message) {
                fail_with("插图发送失败");
                return;
            }
        }
        IlluStep::Finish(message) => {
            let _ = send_to(&device_addr, &message);
        }
    }
    render_from_state();
}

fn log_reading_data(
    s: &mut UiState,
    progress: Option<&Value>,
    reading_time: Option<&Value>,
    bookmarks: Option<&Value>,
) {
    fn as_parsed(v: Option<&Value>) -> Option<Value> {
        match v? {
            Value::String(t) if !t.is_empty() => serde_json::from_str(t).ok(),
            Value::Null | Value::String(_) => None,
            other => Some(other.clone()),
        }
    }
    fn summarize_progress(v: &Value) -> String {
        if let Some(obj) = v.as_object() {
            for key in ["chapterIndex", "index", "chapter"] {
                if let Some(n) = obj.get(key).and_then(|x| x.as_i64()) {
                    return format!("第 {} 章", n + 1);
                }
            }
        }
        let mut raw: String = v.to_string().chars().take(60).collect();
        if v.to_string().chars().count() > 60 {
            raw.push('…');
        }
        raw
    }

    let progress_text = as_parsed(progress)
        .map(|p| summarize_progress(&p))
        .unwrap_or_else(|| "无".to_string());
    let time_text = as_parsed(reading_time)
        .and_then(|t| {
            t.as_object()
                .and_then(|o| o.get("totalSeconds"))
                .and_then(|v| v.as_i64())
        })
        .map(|secs| format!("累计 {} 分钟", secs / 60))
        .unwrap_or_else(|| "无".to_string());
    let bookmark_count = as_parsed(bookmarks)
        .filter(|b| b.is_array())
        .map(|b| b.as_array().map(|a| a.len()).unwrap_or(0))
        .unwrap_or(0);

    log_line(
        s,
        &format!(
            "[进度] {} · 阅读时长 {} · 书签 {} 条",
            progress_text, time_text, bookmark_count
        ),
    );
}

// ---------------------------------------------------------------- 传输下发

fn session_start_transfer() {
    let emit = {
        let mut s = state();
        let Some(session) = s.session.as_mut() else {
            return;
        };
        let message = match &mut session.job {
            Job::Plus(t) if t.pos >= t.send_order.len() => protocol::plus_transfer_complete(),
            Job::Plus(t) => {
                let start_from = t.chapters[t.send_order[t.pos]].index;
                let has_cover = !t.cover_done && !t.cover_chunks.is_empty();
                protocol::plus_start_transfer(
                    &t.file_name,
                    t.chapters.len(),
                    t.total_words,
                    start_from,
                    has_cover,
                    Some(t.author.as_str()),
                    Some(t.summary.as_str()),
                )
            }
            Job::Query(_) => return,
        };
        (session.device_addr().to_string(), message)
    };

    let (device_addr, message) = emit;
    if !send_to(&device_addr, &message) {
        handle_link_lost("发送失败");
        return;
    }
    mark_link_activity();
}

// ---------------------------------------------------------------- 收包分发

fn handle_file_message(message: &Value) {
    let payload = message.get("data").unwrap_or(message);

    if handle_band_message(payload) {
        return;
    }

    {
        let s = state();
        match s.session.as_ref() {
            Some(session) if session.is_transfer() => {}
            _ => return,
        }
    }

    let message_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match message_type {
        "ready" => plus_on_ready(payload),
        "next_chunk" => plus_on_next_chunk(payload),
        "chapter_chunk_complete" => plus_on_chapter_chunk_complete(payload),
        "chapter_saved" => plus_on_chapter_saved(payload),
        "cover_chunk_received" | "cover_ready" => plus_on_cover_acked(payload),
        "cover_saved" => {}
        "transfer_finished" => on_success(),
        "error" => on_error(payload),
        "cancel" => on_remote_cancel(),
        _ => {}
    }
}

fn handle_band_message(payload: &Value) -> bool {
    let message_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match message_type {
        "book_status" => {
            let synced: Vec<usize> = payload
                .get("syncedChapters")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64().map(|n| n as usize))
                        .collect()
                })
                .unwrap_or_default();
            let has_cover = payload
                .get("hasCover")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let illustrations = payload
                .get("syncedIllustrations")
                .and_then(|v| v.as_object())
                .map(|m| m.len())
                .unwrap_or(0);
            {
                let mut s = state();
                s.band.status_known = true;
                s.band.synced_chapters = synced.clone();
                s.band.has_cover = has_cover;
                log_line(
                    &mut s,
                    &format!(
                        "[设备] 已同步 {} 章，插图 {} 张{}",
                        synced.len(),
                        illustrations,
                        if has_cover { "，已有封面" } else { "" }
                    ),
                );
            }
            settle_query_expect();
            true
        }
        "storage_info" => {
            let product = payload
                .get("product")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let total = payload
                .get("totalStorage")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let mut avail = payload
                .get("actualAvailable")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if avail == 0 {
                avail = payload
                    .get("availableStorage")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
            }
            {
                let mut s = state();
                s.band.storage_known = true;
                s.band.product = product.clone();
                s.band.storage_total = total;
                s.band.storage_avail = avail;
                log_line(
                    &mut s,
                    &format!(
                        "[设备] {} 可用 {} / {}",
                        product.clone().unwrap_or_else(|| "手环".to_string()),
                        format_bytes(avail as usize),
                        format_bytes(total as usize)
                    ),
                );
            }
            settle_query_expect();
            true
        }
        "settings_data" => {
            let mut count = 0usize;
            {
                let mut s = state();
                if let Some(map) = payload.get("settings").and_then(|v| v.as_object()) {
                    for (k, v) in map {
                        let text = match v {
                            Value::String(t) => t.clone(),
                            Value::Null => String::new(),
                            other => other.to_string(),
                        };
                        if !text.is_empty() {
                            s.band.settings.insert(k.clone(), text);
                        }
                        count += 1;
                    }
                }
                s.band.settings_loaded = true;
                log_line(&mut s, &format!("[设置] 已读取 {} 项", count));
            }
            settle_query_expect();
            true
        }
        "progress" => {
            let should = {
                let mut s = state();
                let should = s.is_querying;
                if should {
                    rearm_query_timer(&mut s);
                    let msg = payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("处理中")
                        .to_string();
                    log_line(&mut s, &format!("[手环] {}", msg));
                    s.phase_text = Some(msg);
                }
                should
            };
            if should {
                render_from_state();
            }
            true
        }
        "book_info_updated" => {
            let should = {
                let mut s = state();
                let should = s.is_querying;
                if should {
                    log_line(&mut s, "[管理] 书籍信息已更新");
                }
                should
            };
            if should {
                settle_query_expect();
            }
            true
        }
        "sync_reading_data" => {
            let should = {
                let mut s = state();
                let should = s.is_querying;
                if should {
                    log_reading_data(
                        &mut s,
                        payload.get("progress"),
                        payload.get("readingTime"),
                        payload.get("bookmarks"),
                    );
                }
                should
            };
            if should {
                settle_query_expect();
            }
            true
        }
        "sync_reading_data_start" => {
            let next = {
                let mut s = state();
                let next = if s.is_querying {
                    rearm_query_timer(&mut s);
                    let total_length = payload
                        .get("totalLength")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    let total_chunks = payload
                        .get("totalChunks")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    if total_chunks > 0 {
                        s.rd_incoming = Some(RdIncoming {
                            total_length,
                            total_chunks,
                            chunks: (0..total_chunks).map(|_| String::new()).collect(),
                            received: 0,
                        });
                        Some(0usize)
                    } else {
                        set_status_message(&mut s, "手环返回的阅读数据为空", Tone::Neutral, true);
                        finish_transfer(&mut s, false);
                        None
                    }
                } else {
                    None
                };
                next
            };
            if let Some(idx) = next {
                let device_addr = last_device_addr();
                let _ = send_to(&device_addr, &protocol::plus_rd_get(idx));
            }
            true
        }
        "sync_reading_data_chunk" => {
            enum RdAction {
                Settle,
                Fetch(usize),
            }
            let action = {
                let mut s = state();
                if !s.is_querying {
                    None
                } else {
                    rearm_query_timer(&mut s);
                    let chunk_index = payload
                        .get("chunkIndex")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    let data = payload
                        .get("data")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let Some(rd) = s.rd_incoming.as_mut() else {
                        return true;
                    };
                    if chunk_index < rd.total_chunks && rd.chunks[chunk_index].is_empty() {
                        rd.chunks[chunk_index] = data;
                        rd.received += 1;
                    }
                    if rd.received < rd.total_chunks {
                        Some(RdAction::Fetch(chunk_index + 1))
                    } else {
                        let joined: String = rd.chunks.concat();
                        let expected = rd.total_length;
                        s.rd_incoming = None;
                        if joined.encode_utf16().count() != expected {
                            log_line(&mut s, "[错误] 阅读数据长度校验失败");
                            set_status_message(&mut s, "阅读数据长度校验失败", Tone::Error, true);
                            finish_transfer(&mut s, true);
                            return true;
                        }
                        match serde_json::from_str::<Value>(&joined) {
                            Ok(doc) => {
                                log_reading_data(
                                    &mut s,
                                    doc.get("progress"),
                                    doc.get("readingTime"),
                                    doc.get("bookmarks"),
                                );
                                Some(RdAction::Settle)
                            }
                            Err(_) => {
                                log_line(&mut s, "[错误] 阅读数据 JSON 解析失败");
                                set_status_message(&mut s, "阅读数据解析失败", Tone::Error, true);
                                finish_transfer(&mut s, true);
                                return true;
                            }
                        }
                    }
                }
            };
            match action {
                Some(RdAction::Settle) => settle_query_expect(),
                Some(RdAction::Fetch(idx)) => {
                    let device_addr = last_device_addr();
                    let _ = send_to(&device_addr, &protocol::plus_rd_get(idx));
                }
                None => {}
            }
            true
        }
        "rd_chunk_received" => true,
        "illustration_ready" => {
            let started = {
                let mut s = state();
                if !s.is_querying {
                    false
                } else {
                    rearm_query_timer(&mut s);
                    let resume = payload
                        .get("resumeFromChunk")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    if let Some(Session {
                        job: Job::Query(QueryKind::SendIllustration(job)),
                        ..
                    }) = s.session.as_mut()
                    {
                        job.idx = resume.min(job.chunks.len());
                        if resume > 0 {
                            log_line(
                                &mut s,
                                &format!("[插图] 手环已有前 {} 块，断点续传", resume),
                            );
                        }
                        true
                    } else {
                        false
                    }
                }
            };
            if started {
                plus_send_illustration_chunk();
            }
            true
        }
        "illustration_chunk_received" => {
            let advance = {
                let mut s = state();
                if !s.is_querying {
                    0usize
                } else {
                    rearm_query_timer(&mut s);
                    if let Some(Session {
                        job: Job::Query(QueryKind::SendIllustration(job)),
                        ..
                    }) = s.session.as_mut()
                    {
                        match payload.get("chunkIndex").and_then(|v| v.as_u64()) {
                            Some(idx) => {
                                let next = idx as usize + 1;
                                if next <= job.idx {
                                    0
                                } else {
                                    job.idx = next.min(job.chunks.len());
                                    job.idx
                                }
                            }
                            None => {
                                job.idx += 1;
                                job.idx
                            }
                        }
                    } else {
                        0
                    }
                }
            };
            let _ = advance;
            plus_send_illustration_chunk();
            true
        }
        "illustration_saved" => {
            let should = {
                let mut s = state();
                let should = s.is_querying;
                if should {
                    log_line(&mut s, "[插图] 手环已保存并登记");
                }
                should
            };
            if should {
                settle_query_expect();
            }
            true
        }
        "success" => {
            {
                let s = state();
                if s.is_sending || !s.is_querying {
                    return false;
                }
            }
            let msg = payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("操作成功")
                .to_string();
            {
                let mut s = state();
                log_line(&mut s, &format!("[手环] {}", msg));
            }
            settle_query_expect();
            true
        }
        "error" => {
            {
                let s = state();
                if !s.is_querying {
                    return false;
                }
            }
            {
                let mut s = state();
                if matches!(
                    s.session.as_ref().and_then(|x| x.query_kind()),
                    Some(QueryKind::SendIllustration(_))
                ) && !s.pending_illustrations.is_empty()
                {
                    s.pending_illustrations.clear();
                    log_line(&mut s, "[插图] 传输失败，剩余随书插图已取消");
                }
            }
            let msg = payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("手环返回错误")
                .to_string();
            fail_with(&format!("手环返回：{}", msg));
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------- 传输协议

fn plus_on_ready(payload: &Value) {
    let start_from = payload.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let need_cover = {
        let mut s = state();
        s.handling_offline = false;
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };
        if let Some(pos) = t
            .send_order
            .iter()
            .position(|&idx| t.chapters[idx].index == start_from)
        {
            t.pos = pos;
        } else {
            tracing::warn!("plus 手环起始章节 {} 不在待发列表，保持当前进度", start_from);
        }
        t.chunk_idx = 0;
        t.chunk_texts.clear();
        t.retries = 0;
        t.cover_idx = 0;
        !t.cover_done && !t.cover_chunks.is_empty()
    };

    if need_cover {
        {
            let mut s = state();
            s.phase_text = Some("发送封面".to_string());
        }
        plus_send_cover_chunk();
    } else {
        plus_emit();
    }
}

fn plus_send_cover_chunk() {
    let device_addr = last_device_addr();
    let emit = {
        let mut s = state();
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };
        if t.cover_idx >= t.cover_chunks.len() {
            None
        } else {
            let idx = t.cover_idx;
            let total = t.cover_chunks.len();
            let message = protocol::plus_cover_chunk(idx, total, &t.cover_chunks[idx]);
            Some((device_addr.clone(), message, idx + 1, total))
        }
    };

    let Some((device_addr, message, done, total)) = emit else {
        plus_finish_cover();
        return;
    };

    {
        let mut s = state();
        s.phase_text = Some(format!("发送封面 {}/{}", done, total));
        set_status_message(&mut s, "正在发送封面...", Tone::Neutral, false);
    }

    if !send_to(&device_addr, &message) {
        handle_link_lost("发送失败");
        return;
    }
    render_from_state();
}

fn plus_on_cover_acked(payload: &Value) {
    {
        let mut s = state();
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };
        match payload.get("chunkIndex").and_then(|v| v.as_u64()) {
            Some(idx) => {
                let next = idx as usize + 1;
                if next <= t.cover_idx {
                    return;
                }
                t.cover_idx = next;
            }
            None => t.cover_idx += 1,
        }
        t.retries = 0;
    }
    plus_send_cover_chunk();
}

fn plus_finish_cover() {
    let device_addr = last_device_addr();
    let has_cover = {
        let mut s = state();
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };
        t.cover_done = true;
        !t.cover_chunks.is_empty()
    };

    if has_cover {
        let message = protocol::plus_cover_transfer_complete();
        let _ = send_to(&device_addr, &message);
        {
            let mut s = state();
            log_line(&mut s, "[封面] 发送完成");
        }
    }
    {
        let mut s = state();
        s.phase_text = Some("发送正文".to_string());
    }
    plus_emit();
}

fn plus_on_next_chunk(payload: &Value) {
    {
        let mut s = state();
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };
        let Some(&idx) = t.send_order.get(t.pos) else {
            return;
        };
        let chapter = t.chapters[idx].index;
        if let Some(count) = payload.get("count").and_then(|v| v.as_u64()) {
            if count != chapter as u64 {
                tracing::warn!("plus 忽略非当前章节的分块确认 {}（当前 {}）", count, chapter);
                return;
            }
        }
        match payload.get("chunkNum").and_then(|v| v.as_u64()) {
            Some(ack) => {
                if ack + 1 <= t.chunk_idx as u64 {
                    return;
                }
                t.chunk_idx = ack as usize + 1;
            }
            None => t.chunk_idx += 1,
        }
        t.retries = 0;
    }
    plus_emit();
}

fn plus_on_chapter_chunk_complete(payload: &Value) {
    {
        let mut s = state();
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };
        let Some(&idx) = t.send_order.get(t.pos) else {
            return;
        };
        let chapter = t.chapters[idx].index;
        if let Some(count) = payload.get("count").and_then(|v| v.as_u64()) {
            if count != chapter as u64 {
                tracing::warn!("plus 忽略过期章节完成确认 {}（当前 {}）", count, chapter);
                return;
            }
        }
        t.chunk_idx = t.chunk_texts.len();
        t.retries = 0;
    }
    send_current_chapter_complete();
}

fn send_current_chapter_complete() {
    let device_addr = last_device_addr();
    let message = {
        let s = state();
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_ref()
        else {
            return;
        };
        let Some(&idx) = t.send_order.get(t.pos) else {
            return;
        };
        protocol::plus_chapter_complete(t.chapters[idx].index)
    };

    if !send_to(&device_addr, &message) {
        handle_link_lost("发送失败");
    }
}

fn plus_on_chapter_saved(payload: &Value) {
    let all_sent = {
        let s = state();
        match s.session.as_ref() {
            Some(Session {
                job: Job::Plus(t),
                ..
            }) => t.pos >= t.send_order.len(),
            _ => false,
        }
    };
    if all_sent {
        plus_emit();
        return;
    }

    let (progress, phase) = {
        let mut s = state();
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };
        let Some(&idx) = t.send_order.get(t.pos) else {
            return;
        };
        let expected = t.chapters[idx].index;
        if let Some(saved) = payload.get("chapterIndex").and_then(|v| v.as_u64()) {
            if saved != expected as u64 {
                tracing::warn!("plus 忽略过期章节保存回执 {}（当前 {}）", saved, expected);
                return;
            }
        }
        t.pos += 1;
        t.chunk_idx = 0;
        t.chunk_texts.clear();
        t.retries = 0;
        let total = t.send_order.len().max(1);
        let progress = (t.pos as f32 / total as f32).clamp(0.0, 1.0);
        let phase = t.send_order.get(t.pos).map(|&next_idx| {
            format!(
                "第 {}/{} 章 · {}",
                t.pos + 1,
                t.send_order.len(),
                t.chapters[next_idx].name
            )
        });
        (progress, phase)
    };

    {
        let mut s = state();
        s.progress = progress;
        if let Some(phase) = phase {
            s.phase_text = Some(phase);
        }
        s.reconnect_attempts = 0;
        s.handling_offline = false;
        s.last_inbound_at = Some(SystemTime::now());
    }

    tracing::info!("plus 章节已保存，进度 {:.0}%", progress * 100.0);

    plus_emit();
}

fn plus_emit() {
    let device_addr = last_device_addr();
    let emit = {
        let mut s = state();
        let Some(Session {
            job: Job::Plus(t),
            ..
        }) = s.session.as_mut()
        else {
            return;
        };

        if t.pos >= t.send_order.len() {
            let message = protocol::plus_transfer_complete();
            Some((device_addr.clone(), message, None, 1.0f32, None))
        } else {
            let chapter_idx = t.send_order[t.pos];
            if t.chunk_texts.is_empty() {
                let content = t.chapters[chapter_idx].content.clone();
                let (len_u16, checksum) = protocol::utf16_len_and_adler32(&content);
                t.content_len_u16 = len_u16;
                t.content_checksum = checksum;
                t.chunk_texts = chapters::split_into_chunks(&content, PLUS_CHUNK_BYTES);
            }

            if t.chunk_idx >= t.chunk_texts.len() {
                None
            } else {
                let total = t.chunk_texts.len();
                let idx = t.chunk_idx;
                let content = t.chunk_texts[idx].clone();
                let name = t.chapters[chapter_idx].name.clone();
                let word_count = t.chapters[chapter_idx].word_count;

                let message = protocol::plus_chapter_chunk(
                    t.chapters[chapter_idx].index,
                    &name,
                    word_count,
                    &content,
                    idx,
                    total,
                    t.content_len_u16,
                    t.content_checksum,
                );
                t.bytes_sent += content.len();
                let speed = compute_speed_text(&mut t.last_chunk_time, content.len());
                let progress = (t.pos as f32 + (idx as f32 / total as f32))
                    / t.send_order.len().max(1) as f32;
                let phase = format!(
                    "第 {}/{} 章 · {} ({} 块)",
                    t.pos + 1,
                    t.send_order.len(),
                    name,
                    total
                );

                tracing::info!(
                    "plus 章节 {}/{} 分块 {}/{} ({} bytes)",
                    t.pos + 1,
                    t.send_order.len(),
                    idx + 1,
                    total,
                    content.len()
                );

                Some((device_addr.clone(), message, speed, progress, Some(phase)))
            }
        }
    };

    let Some((device_addr, message, speed, progress, phase)) = emit else {
        return;
    };

    {
        let mut s = state();
        s.progress = progress.clamp(0.0, 1.0);
        s.speed_text = speed;
        if let Some(phase) = phase {
            s.phase_text = Some(phase);
        }
        let status = format!("发送中 {}%", (progress * 100.0).round());
        set_status_message(&mut s, &status, Tone::Neutral, false);
    }

    if !send_to(&device_addr, &message) {
        handle_link_lost("发送失败");
        return;
    }
    render_from_state();
}

// ---------------------------------------------------------------- 收尾与错误

fn on_success() {
    let queue_len = {
        let mut s = state();
        s.progress = 1.0;
        s.phase_text = Some("传输完成".to_string());
        set_status_message(&mut s, "发送成功", Tone::Success, true);
        log_line(&mut s, "[完成] 发送成功");
        finish_transfer(&mut s, false);
        let queue_len = s.pending_illustrations.len();
        if queue_len > 0 {
            log_line(&mut s, &format!("[插图] 开始随书自动传输 {} 张", queue_len));
        }
        queue_len
    };
    render_from_state();
    if queue_len > 0 {
        start_next_auto_illustration();
    }
}

fn start_next_auto_illustration() -> bool {
    let job = {
        let mut s = state();
        if s.pending_illustrations.is_empty() {
            return false;
        }
        let item = s.pending_illustrations.remove(0);
        let filename = s.file_name.clone().unwrap_or_default();
        if filename.is_empty() {
            s.pending_illustrations.clear();
            log_line(&mut s, "[插图] 无目标书籍，剩余自动插图已取消");
            None
        } else {
            let b64 = protocol::base64_encode(&item.bytes);
            let chunks = chunk_string(&b64, COVER_CHUNK_CHARS);
            let remaining = s.pending_illustrations.len();
            log_line(
                &mut s,
                &format!(
                    "[插图] 传输 {}（{}），剩余 {} 张",
                    item.relative_path,
                    format_bytes(item.bytes.len()),
                    remaining
                ),
            );
            Some(IllustrationJob {
                filename,
                relative_path: item.relative_path,
                fingerprint: protocol::fingerprint(&item.bytes),
                chunks,
                total_bytes: item.bytes.len(),
                chunk_size: COVER_CHUNK_CHARS,
                idx: 0,
            })
        }
    };
    match job {
        Some(job) => {
            start_query(QueryKind::SendIllustration(job));
            true
        }
        None => {
            render_from_state();
            false
        }
    }
}

fn on_remote_cancel() {
    {
        let mut s = state();
        set_status_message(&mut s, "传输已被手环取消", Tone::Neutral, true);
        log_line(&mut s, "[中断] 手环端已取消");
        finish_transfer(&mut s, true);
    }
    render_from_state();
}

fn align_to_band_chapter(t: &mut PlusJob, band_chapter: Option<usize>) {
    let Some(band_index) = band_chapter else {
        return;
    };
    let current = t.send_order.get(t.pos).map(|&idx| t.chapters[idx].index);
    if current == Some(band_index) {
        return;
    }
    let Some(pos) = t
        .send_order
        .iter()
        .position(|&idx| t.chapters[idx].index == band_index)
    else {
        return;
    };
    tracing::warn!(
        "plus 对齐手环当前章节 {}（本地 {:?}）",
        band_index,
        current
    );
    t.pos = pos;
    t.chunk_idx = 0;
    t.chunk_texts.clear();
}

fn on_error(payload: &Value) {
    let message = payload
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("传输出错")
        .to_string();

    let retry = {
        let mut s = state();
        let Some(session) = s.session.as_mut() else {
            return;
        };
        session.bump_retry()
    };

    let band_chapter = payload
        .get("count")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    match retry {
        Retry::Plus => {
            tracing::warn!("plus 分块重试：{}", message);
            let in_cover = {
                let mut s = state();
                let Some(Session {
                    job: Job::Plus(t),
                    ..
                }) = s.session.as_mut()
                else {
                    return;
                };
                if !t.cover_done && !t.cover_chunks.is_empty() {
                    if t.retries >= 2 {
                        t.cover_idx = 0;
                    }
                    true
                } else {
                    align_to_band_chapter(t, band_chapter);
                    if t.pos < t.send_order.len() && t.retries >= 2 {
                        let all_sent =
                            !t.chunk_texts.is_empty() && t.chunk_idx >= t.chunk_texts.len();
                        if all_sent || t.chunk_idx > 0 {
                            let index = t.chapters[t.send_order[t.pos]].index;
                            tracing::warn!("plus 章节 {} 反复失败，从第 0 块重发", index);
                            t.chunk_idx = 0;
                            t.chunk_texts.clear();
                        }
                    }
                    false
                }
            };
            if in_cover {
                plus_send_cover_chunk();
                return;
            }
            let chapter_complete_pending = {
                let s = state();
                match s.session.as_ref() {
                    Some(Session {
                        job: Job::Plus(t),
                        ..
                    }) => {
                        t.pos < t.send_order.len()
                            && !t.chunk_texts.is_empty()
                            && t.chunk_idx >= t.chunk_texts.len()
                    }
                    _ => false,
                }
            };
            if chapter_complete_pending {
                send_current_chapter_complete();
            } else {
                plus_emit();
            }
        }
        Retry::Fail => {
            fail_with(&format!("传输出错：{}", message));
        }
    }
}

fn fail_with(message: &str) {
    {
        let mut s = state();
        set_status_message(&mut s, message, Tone::Error, true);
        log_line(&mut s, &format!("[错误] {}", message));
        finish_transfer(&mut s, true);
    }
    render_from_state();
}

// ---------------------------------------------------------------- 会话构造

#[allow(clippy::too_many_arguments)]
fn build_session(
    device_addr: String,
    file_name: String,
    text: String,
    bytes_len: usize,
    split: &SplitOptions,
    author: &str,
    summary: &str,
    cover_bytes: Option<Vec<u8>>,
    skip_synced: bool,
    synced: &[usize],
) -> Result<Session, String> {
    if bytes_len == 0 || text.chars().count() == 0 {
        return Err("文件为空".to_string());
    }

    let chapters = chapters::split_chapters_with(&text, &file_name, split);
    if chapters.is_empty() {
        return Err("未能切分出章节".to_string());
    }
    let total_words = chapters.iter().map(|c| c.word_count).sum();

    let skipped: std::collections::BTreeSet<usize> = if skip_synced {
        synced.iter().copied().collect()
    } else {
        std::collections::BTreeSet::new()
    };
    let send_order: Vec<usize> = chapters
        .iter()
        .enumerate()
        .map(|(i, _)| i)
        .filter(|i| !skipped.contains(i))
        .collect();

    if send_order.is_empty() {
        return Err("所有章节都已同步，无需重复发送".to_string());
    }

    let cover_chunks = match cover_bytes {
        Some(bytes) if !bytes.is_empty() => {
            let b64 = protocol::base64_encode(&bytes);
            chunk_string(&b64, COVER_CHUNK_CHARS)
        }
        _ => Vec::new(),
    };

    {
        let mut s = state();
        log_line(
            &mut s,
            &format!(
                "[分章] {} 章（发 {} 章），共 {} 字{}",
                chapters.len(),
                send_order.len(),
                total_words,
                if cover_chunks.is_empty() {
                    String::new()
                } else {
                    format!("，封面 {} 块", cover_chunks.len())
                }
            ),
        );
    }

    let job = Job::Plus(PlusJob {
        file_name,
        chapters,
        send_order,
        pos: 0,
        chunk_idx: 0,
        chunk_texts: Vec::new(),
        total_words,
        author: author.to_string(),
        summary: summary.to_string(),
        cover_chunks,
        cover_idx: 0,
        cover_done: false,
        bytes_sent: 0,
        last_chunk_time: None,
        retries: 0,
        content_len_u16: 0,
        content_checksum: 0,
    });

    Ok(Session {
        device_addr,
        job,
        pending_start: true,
        handshake_complete: false,
    })
}

fn chunk_string(s: &str, size: usize) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(size).map(|c| c.iter().collect()).collect()
}

// ---------------------------------------------------------------- 平台调用

fn send_to(device_addr: &str, payload: &str) -> bool {
    wit_bindgen::block_on(async {
        interconnect::send_qaic_message(device_addr, protocol::PACKAGE, payload).await
    })
    .is_ok()
}

fn get_device_addr() -> Result<String, String> {
    let devices = wit_bindgen::block_on(async { device::get_connected_device_list().await });
    devices
        .first()
        .map(|device| device.addr.clone())
        .ok_or_else(|| "未找到设备，请先在 AstroBox 连接手环".to_string())
}

fn get_app_info(device_addr: &str) -> Result<thirdpartyapp::AppInfo, String> {
    let app_list =
        wit_bindgen::block_on(async { thirdpartyapp::get_thirdparty_app_list(device_addr).await });
    let apps = app_list.map_err(|_| "获取应用列表失败".to_string())?;
    apps.into_iter()
        .find(|app| app.package_name == protocol::PACKAGE)
        .ok_or_else(|| format!("未找到「{}」，请先在设备上安装", protocol::APP_LABEL))
}

// ---------------------------------------------------------------- 计时器与状态

fn schedule_hide_message(state: &mut UiState) {
    clear_hide_message_timer(state);
    let timer_id =
        wit_bindgen::block_on(async { timer::set_timeout(3000, TIMER_HIDE_MESSAGE).await });
    state.hide_message_timer_id = Some(timer_id);
}

fn schedule_handshake_timeout(state: &mut UiState) {
    clear_handshake_timer(state);
    let timer_id =
        wit_bindgen::block_on(async { timer::set_timeout(3000, TIMER_HANDSHAKE_TIMEOUT).await });
    state.handshake_timer_id = Some(timer_id);
}

fn clear_hide_message_timer(state: &mut UiState) {
    if let Some(timer_id) = state.hide_message_timer_id.take() {
        let _ = wit_bindgen::block_on(async { timer::clear_timer(timer_id).await });
    }
}

fn clear_handshake_timer(state: &mut UiState) {
    if let Some(timer_id) = state.handshake_timer_id.take() {
        let _ = wit_bindgen::block_on(async { timer::clear_timer(timer_id).await });
    }
}

fn clear_query_timer(state: &mut UiState) {
    if let Some(timer_id) = state.query_timer_id.take() {
        let _ = wit_bindgen::block_on(async { timer::clear_timer(timer_id).await });
    }
}

fn rearm_query_timer(state: &mut UiState) {
    clear_query_timer(state);
    let timer_id = wit_bindgen::block_on(async {
        timer::set_timeout(MGMT_QUERY_TIMEOUT_MS, TIMER_QUERY_TIMEOUT).await
    });
    state.query_timer_id = Some(timer_id);
}

fn set_status_message(state: &mut UiState, message: &str, tone: Tone, auto_hide: bool) {
    state.status_message = Some(message.to_string());
    state.status_tone = tone;
    if auto_hide {
        schedule_hide_message(state);
    } else {
        clear_hide_message_timer(state);
    }
}

fn finish_transfer(state: &mut UiState, clear_progress: bool) {
    state.is_sending = false;
    state.is_querying = false;
    state.session = None;
    state.pending_writes.clear();
    state.query_expect = 0;
    if clear_progress {
        state.progress = 0.0;
    }
    state.speed_text = None;
    state.phase_text = None;
    state.handling_offline = false;
    state.reconnect_attempts = 0;
    state.offline_strikes = 0;
    clear_handshake_timer(state);
    clear_query_timer(state);
    disarm_watchdog(state);
    disarm_reconnect_timer(state);
}

fn log_line(state: &mut UiState, message: &str) {
    let stamp = state
        .started_at
        .or_else(|| {
            state.started_at = Some(SystemTime::now());
            state.started_at
        })
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| {
            let secs = d.as_secs();
            format!("{:02}:{:02}", secs / 60, secs % 60)
        })
        .unwrap_or_else(|| "00:00".to_string());

    state.log.push(format!("[{}] {}", stamp, message));
    if state.log.len() > MAX_LOG_LINES {
        let overflow = state.log.len() - MAX_LOG_LINES;
        state.log.drain(..overflow);
    }
}

fn compute_speed_text(last_time: &mut Option<SystemTime>, chunk_bytes: usize) -> Option<String> {
    let now = SystemTime::now();
    let speed_text = last_time.and_then(|prev| {
        now.duration_since(prev).ok().and_then(|elapsed| {
            let secs = elapsed.as_secs_f64();
            if secs <= 0.0 {
                None
            } else {
                let speed = (chunk_bytes as f64 / secs) as usize;
                Some(format!("{}/s", format_bytes(speed)))
            }
        })
    });
    *last_time = Some(now);
    speed_text
}

fn extract_payload_text(payload: &str) -> String {
    if let Ok(json) = serde_json::from_str::<Value>(payload) {
        if let Some(text) = json.get("payloadText").and_then(|v| v.as_str()) {
            return text.to_string();
        }
        if let Some(payload_value) = json.get("payload") {
            if let Some(text) = payload_value.as_str() {
                return text.to_string();
            }
            return payload_value.to_string();
        }
    }
    payload.to_string()
}

fn format_bytes(bytes: usize) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    let k = 1024_f64;
    let sizes = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut idx = 0usize;
    while size >= k && idx < sizes.len() - 1 {
        size /= k;
        idx += 1;
    }
    if idx == 0 {
        format!("{} {}", bytes, sizes[0])
    } else {
        format!("{:.2} {}", size, sizes[idx])
    }
}

// ---------------------------------------------------------------- 界面构件

fn text_el(text: &str, size: u32, color: &str) -> ui::Element {
    ui::Element::new(ui::ElementType::P, Some(text))
        .size(size)
        .text_color(color)
}

fn section_title(text: &str) -> ui::Element {
    text_el(text, 12, C_MUTED).margin_bottom(8)
}

fn row() -> ui::Element {
    ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Row)
        .align_center()
        .margin_bottom(8)
}

fn card() -> ui::Element {
    ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .padding(14)
        .radius(12)
        .bg(C_SURFACE)
        .border(1, C_BORDER)
        .width_full()
        .margin_bottom(10)
}

fn chip(label: &str, active: bool, event_id: &str, disabled: bool) -> ui::Element {
    let mut el = ui::Element::new(ui::ElementType::Button, Some(label))
        .size(12)
        .radius(14)
        .padding_left(10)
        .padding_right(10)
        .padding_top(6)
        .padding_bottom(6)
        .margin_right(6);
    el = if active {
        el.bg(C_ACCENT).text_color("#FFFFFF")
    } else {
        el.bg(C_FIELD).text_color("#44506A")
    };
    if disabled {
        el = el.disabled().opacity(0.5);
    }
    el.on(ui::Event::Click, event_id)
}

fn primary_button(label: &str, event_id: &str, disabled: bool) -> ui::Element {
    let mut el = ui::Element::new(ui::ElementType::Button, Some(label))
        .size(14)
        .radius(10)
        .padding_left(16)
        .padding_right(16)
        .padding_top(9)
        .padding_bottom(9)
        .bg(C_ACCENT)
        .text_color("#FFFFFF")
        .margin_right(8);
    if disabled {
        el = el.disabled().opacity(0.45);
    }
    el.on(ui::Event::Click, event_id)
}

fn secondary_button(label: &str, event_id: &str, disabled: bool) -> ui::Element {
    let mut el = ui::Element::new(ui::ElementType::Button, Some(label))
        .size(14)
        .radius(10)
        .padding_left(14)
        .padding_right(14)
        .padding_top(9)
        .padding_bottom(9)
        .bg(C_FIELD)
        .text_color("#44506A")
        .margin_right(8);
    if disabled {
        el = el.disabled().opacity(0.45);
    }
    el.on(ui::Event::Click, event_id)
}

fn ghost_button(label: &str, event_id: &str) -> ui::Element {
    let mut el = ui::Element::new(ui::ElementType::Button, Some(label))
        .size(13)
        .radius(10)
        .padding_left(12)
        .padding_right(12)
        .padding_top(8)
        .padding_bottom(8)
        .bg(C_FIELD)
        .text_color("#44506A")
        .margin_right(6);
    el = el.disabled().opacity(0.9);
    el.on(ui::Event::Click, event_id)
}

fn tiny_button(label: &str, event_id: &str) -> ui::Element {
    let mut el = ui::Element::new(ui::ElementType::Button, Some(label))
        .size(13)
        .radius(8)
        .padding_left(10)
        .padding_right(10)
        .padding_top(5)
        .padding_bottom(5)
        .bg(C_FIELD)
        .text_color("#44506A")
        .margin_right(6);
    el = el.disabled().opacity(0.95);
    el.on(ui::Event::Click, event_id)
}

// ---------------------------------------------------------------- 界面组装

fn build_main_ui(s: &UiState) -> ui::Element {
    let mut root = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .padding(12)
        .width_full();

    root = root.child(build_header(s));
    root = root.child(build_nav(s));

    match s.panel {
        Panel::Send => {
            root = root.child(build_send_panel(s));
        }
        Panel::Device => {
            root = root.child(build_device_panel(s));
        }
        Panel::Log => {
            root = root.child(build_log_panel(s));
        }
        Panel::About => {
            root = root.child(build_about_panel());
        }
    }

    root.child(build_status_bar(s))
}

fn build_header(_s: &UiState) -> ui::Element {
    ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .padding(14)
        .radius(12)
        .bg(C_ACCENT_SOFT)
        .width_full()
        .margin_bottom(10)
        .child(
            ui::Element::new(ui::ElementType::P, Some("弦电子书同步器"))
                .size(16)
                .text_color(C_ACCENT_TEXT)
                .margin_bottom(2),
        )
        .child(
            text_el("把手环上的弦电子书当接收端，把 TXT 分章推过去", 11, C_SUB)
                .margin_bottom(6),
        )
        .child(text_el(
            &format!("{} · {}", protocol::PACKAGE, protocol::APP_HINT),
            11,
            C_MUTED,
        ))
}

fn build_nav(s: &UiState) -> ui::Element {
    row()
        .child(chip(
            "发送",
            s.panel == Panel::Send,
            EVENT_PANEL_SEND,
            false,
        ))
        .child(chip(
            "设备",
            s.panel == Panel::Device,
            EVENT_PANEL_DEVICE,
            false,
        ))
        .child(chip("日志", s.panel == Panel::Log, EVENT_PANEL_LOG, false))
        .child(chip("关于", s.panel == Panel::About, EVENT_PANEL_ABOUT, false))
        .margin_bottom(10)
}

fn build_send_panel(s: &UiState) -> ui::Element {
    let busy = busy_now(s);
    let has_file = s.file_text.is_some();

    // ① 选书
    let file_info = match &s.file_name {
        Some(name) => format!("{} · {}", name, format_bytes(s.file_size_bytes)),
        None => "未选择文件".to_string(),
    };
    let file_color = if has_file { C_TEXT } else { C_MUTED };

    let pick_row = row()
        .child(secondary_button("选择 TXT", EVENT_PICK_FILE, busy))
        .child(text_el(&file_info, 12, file_color));

    let author_text = if s.author.is_empty() {
        "未设置".to_string()
    } else {
        s.author.clone()
    };
    let summary_text = if s.summary.is_empty() {
        "未设置".to_string()
    } else {
        s.summary.clone()
    };
    let author_color = if s.author.is_empty() { C_MUTED } else { C_TEXT };
    let summary_color = if s.summary.is_empty() { C_MUTED } else { C_TEXT };

    let meta_card = card()
        .child(section_title("书籍信息（可选，会写入手环书库）"))
        .child(
            row()
                .child(text_el("作者", 12, C_SUB).margin_right(8))
                .child(text_el(&author_text, 12, author_color).margin_right(8))
                .child(tiny_button("编辑", EVENT_EDIT_AUTHOR)),
        )
        .child(
            row()
                .child(text_el("简介", 12, C_SUB).margin_right(8))
                .child(text_el(&summary_text, 12, summary_color).margin_right(8))
                .child(tiny_button("编辑", EVENT_EDIT_SUMMARY)),
        );

    let cover_info = match &s.cover_name {
        Some(name) => match &s.cover_bytes {
            Some(bytes) => format!("{} · {}", name, format_bytes(bytes.len())),
            None => name.clone(),
        },
        None => "未选择封面（可跳过）".to_string(),
    };
    let cover_row = row()
        .child(secondary_button("选择封面", EVENT_PICK_COVER, busy))
        .child(tiny_button("清除", EVENT_CLEAR_COVER))
        .child(text_el(&cover_info, 12, C_MUTED));

    let cover_card = card()
        .child(section_title("封面（可选，JPG/PNG）"))
        .child(cover_row);

    let illus_card = {
        let mut card_el = card().child(section_title("插图（可选，发送完成后自动逐张传输）"));
        if s.pending_illustrations.is_empty() {
            card_el = card_el.child(
                row()
                    .child(secondary_button("添加插图", EVENT_PICK_ILLUSTRATION, busy))
                    .child(text_el("未选择（可跳过）", 12, C_MUTED)),
            );
        } else {
            card_el = card_el.child(
                row()
                    .child(secondary_button("添加插图", EVENT_PICK_ILLUSTRATION, busy))
                    .child(tiny_button("清空", EVENT_CLEAR_ILLUSTRATIONS)),
            );
            for item in &s.pending_illustrations {
                card_el = card_el.child(
                    text_el(
                        &format!("{} · {}", item.relative_path, format_bytes(item.bytes.len())),
                        11,
                        C_MUTED,
                    )
                    .margin_bottom(2),
                );
            }
        }
        card_el
    };

    let mut split_card = card().child(section_title("分章方式"));
    let split_row = row()
        .child(chip(
            s.split.mode.label(),
            true,
            EVENT_CYCLE_SPLIT,
            busy,
        ))
        .child(text_el("点击切换", 11, C_MUTED));
    split_card = split_card.child(split_row);

    if let Some((count, words)) = s.chapter_estimate {
        split_card = split_card.child(
            text_el(&format!("预计 {} 章 · {} 字", count, words), 11, C_SUB)
                .margin_bottom(6),
        );
    }

    if s.split.mode.uses_words() {
        let words_row = row()
            .child(text_el("每章字数", 12, C_SUB))
            .child(tiny_button("−", EVENT_WORDS_DOWN))
            .child(
                text_el(&format!("{}", s.split.words_per_chapter), 13, C_TEXT).margin_right(6),
            )
            .child(tiny_button("＋", EVENT_WORDS_UP));
        split_card = split_card.child(words_row);
    }

    if s.split.mode.uses_keyword() {
        let keyword = if s.split.keyword.is_empty() {
            "未设置".to_string()
        } else {
            s.split.keyword.clone()
        };
        let keyword_color = if s.split.keyword.is_empty() {
            C_MUTED
        } else {
            C_TEXT
        };
        split_card = split_card.child(
            row()
                .child(text_el("行首关键字", 12, C_SUB).margin_right(8))
                .child(text_el(&keyword, 12, keyword_color).margin_right(8))
                .child(tiny_button("编辑", EVENT_EDIT_KEYWORD)),
        );
    }

    let synced_hint = if s.band.status_known {
        format!("已同步 {} 章", s.band.synced_chapters.len())
    } else {
        "未查询设备状态".to_string()
    };
    let skip_row = row()
        .child(chip(
            if s.skip_synced {
                "跳过已同步 ✓"
            } else {
                "跳过已同步"
            },
            s.skip_synced,
            EVENT_TOGGLE_SKIP,
            false,
        ))
        .child(text_el(&synced_hint, 11, C_MUTED));
    split_card = split_card.child(skip_row);

    // ② 传输
    let progress_row = row().child(
        text_el(
            &format!("{:.0}%", (s.progress * 100.0).round()),
            13,
            C_TEXT,
        )
        .margin_right(10),
    );
    let progress_row = progress_row.child(text_el(
        &format!(
            "速率 {}",
            s.speed_text.clone().unwrap_or_else(|| "-".to_string())
        ),
        11,
        C_SUB,
    ));

    let bar_width = if s.progress <= 0.0 {
        0
    } else {
        ((s.progress.clamp(0.0, 1.0) * BAR_WIDTH as f32).round() as u32).max(2)
    };
    let fill = ui::Element::new(ui::ElementType::Div, None)
        .bg(C_ACCENT)
        .height(6)
        .width(bar_width)
        .radius(6)
        .transition("width 200ms ease");
    let bar = ui::Element::new(ui::ElementType::Div, None)
        .bg(C_TRACK)
        .radius(6)
        .width(BAR_WIDTH)
        .height(6)
        .margin_bottom(10)
        .child(fill);

    let phase_text = s
        .phase_text
        .clone()
        .unwrap_or_else(|| "等待开始".to_string());

    let disconnect_row = row()
        .child(text_el("掉线处理", 12, C_SUB).margin_right(8))
        .child(chip(
            s.disconnect_action.label(),
            s.disconnect_action == DisconnectAction::Retry,
            EVENT_CYCLE_DISCONNECT,
            busy,
        ))
        .child(text_el(s.disconnect_action.hint(), 11, C_MUTED));

    let send_card = card()
        .child(section_title("传输"))
        .child(
            row()
                .child(primary_button("开始发送", EVENT_SEND_FILE, busy || !has_file))
                .child(secondary_button("取消", EVENT_CANCEL_SEND, !busy)),
        )
        .child(disconnect_row)
        .child(progress_row)
        .child(bar)
        .child(text_el(&phase_text, 11, C_SUB));

    // 组合
    let mut panel = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .width_full();
    panel = panel.child(
        card()
            .child(section_title("① 选择书籍"))
            .child(pick_row),
    );
    panel = panel.child(meta_card);
    panel = panel.child(cover_card);
    panel = panel.child(illus_card);
    panel = panel.child(split_card);
    panel = panel.child(send_card);
    panel
}

fn build_device_panel(s: &UiState) -> ui::Element {
    let busy = busy_now(s);

    let product = s.band.product.clone().unwrap_or_else(|| "-".to_string());
    let storage_text = if s.band.storage_known {
        format!(
            "{} / {}",
            format_bytes(s.band.storage_avail as usize),
            format_bytes(s.band.storage_total as usize)
        )
    } else {
        "未查询".to_string()
    };
    let status_text = if s.band.status_known {
        format!(
            "{} 章已同步{}",
            s.band.synced_chapters.len(),
            if s.band.has_cover { "，已有封面" } else { "" }
        )
    } else {
        "未查询".to_string()
    };
    let file_name = s
        .file_name
        .clone()
        .unwrap_or_else(|| "（未选择书籍）".to_string());

    let info_card = card()
        .child(section_title("手环信息"))
        .child(row().child(text_el("型号", 12, C_SUB)).child(text_el(&product, 12, C_TEXT)))
        .child(
            row()
                .child(text_el("可用存储", 12, C_SUB))
                .child(text_el(&storage_text, 12, C_TEXT)),
        )
        .child(
            row()
                .child(text_el("当前书籍", 12, C_SUB))
                .child(text_el(&file_name, 12, C_TEXT)),
        )
        .child(
            row()
                .child(text_el("同步状态", 12, C_SUB))
                .child(text_el(&status_text, 12, C_TEXT)),
        )
        .child(
            row()
                .child(secondary_button("刷新设备信息", EVENT_REFRESH_DEVICE, busy))
                .child(secondary_button("读取阅读设置", EVENT_LOAD_SETTINGS, busy)),
        );

    let manage_card = card()
        .child(section_title("手环书库管理（V26.5.4+）"))
        .child(
            row()
                .child(secondary_button("删除书籍", EVENT_MGMT_DELETE_BOOK, busy))
                .child(secondary_button("删除章节", EVENT_MGMT_DELETE_CHAPTERS, busy)),
        )
        .child(
            row()
                .child(secondary_button("更新书籍信息", EVENT_MGMT_UPDATE_INFO, busy))
                .child(secondary_button("查阅读进度", EVENT_MGMT_READING_DATA, busy)),
        )
        .child(row().child(secondary_button("传输插图", EVENT_MGMT_ILLUSTRATION, busy)));

    let mut settings_card = card().child(section_title("手环阅读设置（点按即下发）"));
    if !s.band.settings_loaded {
        settings_card = settings_card
            .child(text_el("尚未读取，点上方「读取阅读设置」获取当前值", 11, C_MUTED));
    }

    for def in BAND_SETTINGS {
        let raw = read_setting_text(s, def.key);
        let shown = def_value_text(def, &raw);
        let mut r = row().child(
            text_el(def.label, 12, C_SUB).margin_right(8),
        );
        match &def.kind {
            SettingKind::Int { .. } => {
                let dec = format!("{}{}", SETTING_DEC_PREFIX, def.key);
                let inc = format!("{}{}", SETTING_INC_PREFIX, def.key);
                r = r
                    .child(tiny_button("−", &dec))
                    .child(text_el(&shown, 12, C_TEXT).margin_right(6))
                    .child(tiny_button("＋", &inc));
            }
            _ => {
                let ev = format!("{}{}", SETTING_CLICK_PREFIX, def.key);
                r = r.child(chip(&shown, false, &ev, false));
            }
        }
        settings_card = settings_card.child(r);
    }

    let mut panel = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .width_full();
    panel = panel.child(info_card);
    panel = panel.child(manage_card);
    panel = panel.child(settings_card);
    panel
}

fn build_log_panel(s: &UiState) -> ui::Element {
    let mut card_el = card()
        .child(section_title(&format!("传输日志（{} 条）", s.log.len())))
        .child(row().child(ghost_button("清空日志", EVENT_CLEAR_LOG)));

    if s.log.is_empty() {
        card_el = card_el.child(text_el("暂无日志", 11, C_MUTED));
    } else {
        let start = s.log.len().saturating_sub(40);
        for line in &s.log[start..] {
            card_el = card_el.child(text_el(line, 11, "#4B5563").margin_bottom(3));
        }
    }

    ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .width_full()
        .child(card_el)
}

/// 「关于」面板：插件来源、可用性说明与联系方式。
fn build_about_panel() -> ui::Element {
    let mut card_el = card().child(section_title("关于"));

    for line in ABOUT_LINES {
        card_el = card_el.child(text_el(line, 11, "#4B5563").margin_bottom(6));
    }

    ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .width_full()
        .child(card_el)
}

fn build_status_bar(s: &UiState) -> ui::Element {
    let (text, color) = match (&s.status_message, s.status_tone) {
        (Some(msg), Tone::Success) => (msg.clone(), C_SUCCESS),
        (Some(msg), Tone::Error) => (msg.clone(), C_ERROR),
        (Some(msg), Tone::Neutral) => (msg.clone(), C_SUB),
        (None, _) => (
            if s.is_sending {
                "传输中...".to_string()
            } else if s.is_querying {
                "读取手环信息...".to_string()
            } else {
                "就绪".to_string()
            },
            C_MUTED,
        ),
    };

    let warn = if s.band.storage_known && s.band.storage_avail < STORAGE_WARN_BYTES {
        Some(text_el("手环剩余空间偏低，建议先清理旧书", 11, C_WARN).margin_bottom(6))
    } else {
        None
    };

    let mut el = ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .padding(12)
        .radius(10)
        .bg(C_FIELD)
        .width_full();
    if let Some(warn) = warn {
        el = el.child(warn);
    }
    el.child(text_el(&text, 12, color))
}

pub fn render_main_ui(element_id: &str) {
    let ui = {
        let mut s = state();
        s.root_element_id = Some(element_id.to_string());
        if s.started_at.is_none() {
            s.started_at = Some(SystemTime::now());
        }
        build_main_ui(&s)
    };
    psys_host::ui::render(element_id, ui);
}

fn render_from_state() {
    let (root_id, ui) = {
        let s = state();
        (s.root_element_id.clone(), build_main_ui(&s))
    };
    if let Some(root_id) = root_id {
        psys_host::ui::render(&root_id, ui);
    }
}
