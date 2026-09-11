//! 章节切分与 UTF-8 字节分块。
//!
//! 弦电子书(plus) 端按「章节」接收书籍：发送端负责把整本 txt 切成章节，
//! 每章再按 UTF-8 字节预算切成若干分块逐块发送。
//!
//! 分章方式对齐安卓端 `ChapterSplitter`（默认 / 宽松 / 英文 / 中文数字 / 阿拉伯数字 /
//! 按字数 / 自定义），并额外保留一套更聪明的「智能识别」作为默认值。
//! 判定规则不引入 regex 依赖，手写字符判定即可。

pub struct Chapter {
    pub index: usize,
    pub name: String,
    pub word_count: usize,
    pub content: String,
}

const MAX_HEADING_LEN: usize = 50;

/// 标题后允许跟随的字符数（对齐安卓端正则里的 `.{0,30}`）。
const TAIL_LIMIT: usize = 30;

// ---------------------------------------------------------------- 分章方式

/// 分章方式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitMode {
    /// 智能识别：综合「第X章」「Chapter N」「序章/番外」「1、」等，误判防护最全。
    Auto,
    /// 默认：`第X章/卷/节/部/篇/回/本` 或 `番外N`。
    Default,
    /// 默认-宽松：允许缺「第」、允许行首空格。
    Loose,
    /// 英文：`Chapter N` / `CHAPTER N`。
    English,
    /// 中文数字：`一、` `二.` …
    ZhNumDot,
    /// 阿拉伯数字：`1.` `2、` …
    DigitDot,
    /// 按字数分章。
    ByWordCount,
    /// 行首关键字（多个关键字用 `|` 分隔，命中行首即视为标题）。
    Custom,
}

impl SplitMode {
    pub fn label(self) -> &'static str {
        match self {
            SplitMode::Auto => "智能识别",
            SplitMode::Default => "第X章/回",
            SplitMode::Loose => "宽松匹配",
            SplitMode::English => "Chapter N",
            SplitMode::ZhNumDot => "中文数字 一、",
            SplitMode::DigitDot => "数字 1. 2、",
            SplitMode::ByWordCount => "按字数分章",
            SplitMode::Custom => "行首关键字",
        }
    }

    /// 是否使用「行首关键字」输入框。
    pub fn uses_keyword(self) -> bool {
        self == SplitMode::Custom
    }

    /// 是否使用「每章字数」设置。
    pub fn uses_words(self) -> bool {
        self == SplitMode::ByWordCount
    }

    pub fn next(self) -> Self {
        match self {
            SplitMode::Auto => SplitMode::Default,
            SplitMode::Default => SplitMode::Loose,
            SplitMode::Loose => SplitMode::English,
            SplitMode::English => SplitMode::ZhNumDot,
            SplitMode::ZhNumDot => SplitMode::DigitDot,
            SplitMode::DigitDot => SplitMode::ByWordCount,
            SplitMode::ByWordCount => SplitMode::Custom,
            SplitMode::Custom => SplitMode::Auto,
        }
    }
}

/// 切分参数。
#[derive(Clone)]
pub struct SplitOptions {
    pub mode: SplitMode,
    /// 按字数分章时每章的字数。
    pub words_per_chapter: usize,
    /// 行首关键字（`|` 分隔）。
    pub keyword: String,
}

impl Default for SplitOptions {
    fn default() -> Self {
        SplitOptions {
            mode: SplitMode::Auto,
            words_per_chapter: 5000,
            keyword: String::new(),
        }
    }
}

// ---------------------------------------------------------------- 字符判定

fn is_cn_num(c: char) -> bool {
    c.is_ascii_digit()
        || matches!(
            c,
            '零' | '一'
                | '二'
                | '三'
                | '四'
                | '五'
                | '六'
                | '七'
                | '八'
                | '九'
                | '十'
                | '百'
                | '千'
                | '万'
                | '两'
                | '〇'
        )
}

fn is_unit(c: char) -> bool {
    matches!(c, '章' | '卷' | '节' | '部' | '篇' | '回' | '本' | '集')
}

/// 吃掉至多 `n` 个空白字符。
fn eat_spaces(s: &str, n: usize) -> &str {
    let mut rest = s;
    for _ in 0..n {
        match rest.chars().next() {
            Some(c) if c.is_whitespace() => rest = &rest[c.len_utf8()..],
            _ => break,
        }
    }
    rest
}

/// 吃掉任意个空白字符。
fn eat_spaces_all(s: &str) -> &str {
    eat_spaces(s, usize::MAX)
}

/// 返回 (数字个数, 剩余)。最多吃 20 位。
fn eat_number(s: &str) -> (usize, &str) {
    let mut rest = s;
    let mut n = 0usize;
    while n < 20 {
        match rest.chars().next() {
            Some(c) if is_cn_num(c) => {
                rest = &rest[c.len_utf8()..];
                n += 1;
            }
            _ => break,
        }
    }
    (n, rest)
}

// ---------------------------------------------------------------- 标题判定

/// 智能识别：第X章 / Chapter N / 序章等特殊标题。
fn is_numbered_heading(t: &str) -> bool {
    if let Some(rest) = t.strip_prefix('第') {
        let rest = eat_spaces_all(rest);
        let (n, rest) = eat_number(rest);
        if n >= 1 {
            let tail = eat_spaces_all(rest);
            if let Some(c) = tail.chars().next() {
                if is_unit(c) {
                    return true;
                }
            }
        }
    }

    for prefix in ["chapter", "CHAPTER", "Chapter"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let rest = eat_spaces_all(rest);
            let mut n = 0usize;
            for c in rest.chars() {
                let roman = matches!(
                    c,
                    'I' | 'V'
                        | 'X'
                        | 'L'
                        | 'C'
                        | 'D'
                        | 'M'
                        | 'i'
                        | 'v'
                        | 'x'
                        | 'l'
                        | 'c'
                        | 'd'
                        | 'm'
                );
                if c.is_ascii_digit() || roman {
                    n += 1;
                } else {
                    break;
                }
            }
            if (1..=8).contains(&n) {
                return true;
            }
        }
    }

    false
}

/// 序章 / 前言 / 楔子 等特殊章节名。
fn is_special_heading(t: &str) -> bool {
    const SPECIALS: [&str; 9] = [
        "序章", "序言", "前言", "引子", "楔子", "后记", "尾声", "终章", "番外",
    ];
    const SEPS: [char; 9] = [' ', '\t', '：', ':', '、', '.', '．', '-', '—'];

    for s in SPECIALS {
        let Some(rest) = t.strip_prefix(s) else {
            continue;
        };
        if rest.is_empty() {
            return true;
        }
        let after = rest.trim_start_matches(|c| SEPS.contains(&c));
        if after.len() == rest.len() {
            // 后面不是分隔符，属于正文里的普通句子（如「前言不搭后语」）。
            continue;
        }
        if after.chars().count() > 20 || after.chars().any(|c| c.is_whitespace()) {
            continue;
        }
        return true;
    }

    false
}

/// 纯数字编号的弱标题，如「1、xxx」。仅在没有强标题时才启用。
fn is_weak_heading(t: &str) -> bool {
    let s = t.trim_start();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() > 4 {
        return false;
    }
    let rest = s[digits.len()..].trim_start();
    let mut chars = rest.chars();
    match chars.next() {
        Some('、') | Some('.') | Some('．') => {}
        _ => return false,
    }
    chars.as_str().trim_start().chars().next().is_some()
}

fn is_strong_heading(t: &str) -> bool {
    is_numbered_heading(t) || is_special_heading(t)
}

fn heading_ok(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.chars().count() <= MAX_HEADING_LEN
}

// ---------------------------------------------------------------- 各模式判定

/// 默认：`^(第(num+)(单位)|番外\s{0,2}(num)*)(.{0,30})$`
fn match_default(t: &str) -> bool {
    if let Some(rest) = t.strip_prefix('第') {
        let rest = eat_spaces(rest, 1);
        let (n, rest) = eat_number(rest);
        if n >= 1 {
            let rest = eat_spaces(rest, 1);
            if let Some(c) = rest.chars().next() {
                if is_unit(c) {
                    let tail = &rest[c.len_utf8()..];
                    return tail.chars().count() <= TAIL_LIMIT;
                }
            }
        }
        return false;
    }

    if let Some(rest) = t.strip_prefix("番外") {
        let rest = eat_spaces(rest, 2);
        let (_, rest) = eat_number(rest);
        return rest.chars().count() <= TAIL_LIMIT;
    }

    false
}

/// 宽松：`^(\s*第?(\s*num+\s*)(单位)|番外\s*num*)(.{0,30})$`
fn match_loose(t: &str) -> bool {
    let t = t.trim_start();
    let body = t.strip_prefix('第').unwrap_or(t);

    let rest = eat_spaces_all(body);
    let (n, rest) = eat_number(rest);
    if n >= 1 {
        let rest = eat_spaces_all(rest);
        if let Some(c) = rest.chars().next() {
            if is_unit(c) {
                let tail = &rest[c.len_utf8()..];
                return tail.chars().count() <= TAIL_LIMIT;
            }
        }
        return false;
    }

    if let Some(rest) = t.strip_prefix("番外") {
        let rest = eat_spaces_all(rest);
        let (_, rest) = eat_number(rest);
        return rest.chars().count() <= TAIL_LIMIT;
    }

    false
}

/// 英文：`^\s*(Chapter|CHAPTER)\s+\d+`
fn match_english(t: &str) -> bool {
    let t = t.trim_start();
    for prefix in ["Chapter", "CHAPTER"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let mut chars = rest.chars();
            let mut spaces = 0;
            while let Some(c) = chars.next() {
                if c.is_whitespace() {
                    spaces += 1;
                } else if c.is_ascii_digit() {
                    if spaces >= 1 {
                        return true;
                    }
                    break;
                } else {
                    break;
                }
            }
        }
    }
    false
}

/// 中文数字（不含阿拉伯数字、不含「两」）：`^\s*[一二三四五六七八九十百千万零〇]+[、.\s]+`
fn match_zh_num_dot(t: &str) -> bool {
    let t = t.trim_start();
    let mut rest = t;
    let mut n = 0usize;
    while let Some(c) = rest.chars().next() {
        if matches!(
            c,
            '零' | '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百'
                | '千' | '万' | '〇'
        ) {
            rest = &rest[c.len_utf8()..];
            n += 1;
        } else {
            break;
        }
    }
    if n == 0 {
        return false;
    }
    match rest.chars().next() {
        Some('、') | Some('.') => true,
        Some(c) if c.is_whitespace() => true,
        _ => false,
    }
}

/// 阿拉伯数字：`^\s*\d+[、.\s]+`
fn match_digit_dot(t: &str) -> bool {
    let t = t.trim_start();
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    let rest = &t[digits.len()..];
    match rest.chars().next() {
        Some('、') | Some('.') => true,
        Some(c) if c.is_whitespace() => true,
        _ => false,
    }
}

/// 行首关键字：命中任一关键字（`|` 分隔）即视为标题。
fn match_keyword(t: &str, keyword: &str) -> bool {
    let t = t.trim_start();
    if t.is_empty() {
        return false;
    }
    keyword
        .split('|')
        .map(|k| k.trim())
        .filter(|k| !k.is_empty())
        .any(|k| t.starts_with(k))
}

fn line_is_heading(mode: SplitMode, line: &str, opts: &SplitOptions) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    match mode {
        SplitMode::Auto => heading_ok(line) && is_strong_heading(t),
        SplitMode::Default => match_default(t) && t.chars().count() <= MAX_HEADING_LEN,
        SplitMode::Loose => match_loose(t) && t.chars().count() <= MAX_HEADING_LEN,
        SplitMode::English => match_english(t),
        SplitMode::ZhNumDot => match_zh_num_dot(t),
        SplitMode::DigitDot => match_digit_dot(t),
        SplitMode::Custom => match_keyword(t, &opts.keyword),
        SplitMode::ByWordCount => false,
    }
}

// ---------------------------------------------------------------- 对外接口

pub fn count_words(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// 把整本文本切分为章节（使用默认参数）。
pub fn split_chapters(raw_text: &str, book_name: &str) -> Vec<Chapter> {
    split_chapters_with(raw_text, book_name, &SplitOptions::default())
}

/// 把整本文本切分为章节。
///
/// - 按字数分章：每章 `words_per_chapter` 个字，章名为「第 N 章」。
/// - 其余模式：按标题行切分；正文前的零散内容作为「前言」；
///   一个标题都识别不到时整本作为一章（章名为书名）。
pub fn split_chapters_with(
    raw_text: &str,
    book_name: &str,
    opts: &SplitOptions,
) -> Vec<Chapter> {
    let text = raw_text.replace("\r\n", "\n").replace('\r', "\n");

    if opts.mode == SplitMode::ByWordCount {
        return split_by_word_count(&text, opts.words_per_chapter.max(1));
    }

    let lines: Vec<&str> = text.split('\n').collect();

    // 智能识别：优先强标题；没有强标题时才纳入弱标题（1、2、）。
    let heading_idx: Vec<usize> = if opts.mode == SplitMode::Auto {
        let strong: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| line_is_heading(SplitMode::Auto, l, opts))
            .map(|(i, _)| i)
            .collect();
        if !strong.is_empty() {
            strong
        } else {
            lines
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    heading_ok(l)
                        && (is_strong_heading(l.trim()) || is_weak_heading(l.trim()))
                })
                .map(|(i, _)| i)
                .collect()
        }
    } else {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| line_is_heading(opts.mode, l, opts))
            .map(|(i, _)| i)
            .collect()
    };

    let fallback_name = if opts.mode == SplitMode::Auto {
        book_name.to_string()
    } else {
        "全文".to_string()
    };

    if heading_idx.is_empty() {
        return vec![Chapter {
            index: 0,
            name: fallback_name,
            word_count: count_words(&text),
            content: text,
        }];
    }

    let preface_name = if opts.mode == SplitMode::Auto {
        book_name.to_string()
    } else {
        "前言".to_string()
    };

    let mut parts: Vec<(String, String)> = Vec::new();

    if heading_idx[0] > 0 {
        let pre = lines[..heading_idx[0]].join("\n");
        if !pre.trim().is_empty() {
            parts.push((preface_name, pre));
        }
    }

    for (h, &start) in heading_idx.iter().enumerate() {
        let end = if h + 1 < heading_idx.len() {
            heading_idx[h + 1]
        } else {
            lines.len()
        };
        let raw_title = lines[start].trim();
        let title: String = if raw_title.is_empty() {
            format!("第 {} 章", h + 1)
        } else {
            raw_title.chars().take(MAX_HEADING_LEN).collect()
        };
        parts.push((title, lines[start..end].join("\n")));
    }

    parts
        .into_iter()
        .enumerate()
        .map(|(i, (name, content))| Chapter {
            index: i,
            name,
            word_count: count_words(&content),
            content,
        })
        .collect()
}

fn split_by_word_count(text: &str, words_per_chapter: usize) -> Vec<Chapter> {
    let chars: Vec<char> = text.chars().collect();
    let mut chapters = Vec::new();
    let mut start = 0usize;

    while start < chars.len() {
        let end = (start + words_per_chapter).min(chars.len());
        let content: String = chars[start..end].iter().collect();
        let content = content.trim().to_string();
        if !content.is_empty() {
            chapters.push(Chapter {
                index: chapters.len(),
                name: format!("第 {} 章", chapters.len() + 1),
                word_count: content.chars().count(),
                content,
            });
        }
        start = end;
    }

    if chapters.is_empty() {
        chapters.push(Chapter {
            index: 0,
            name: "全文".to_string(),
            word_count: 0,
            content: text.to_string(),
        });
    }

    chapters
}

/// 按 UTF-8 字节预算切块，保证不切断多字节字符。空内容返回单个空块。
pub fn split_into_chunks(content: &str, max_bytes: usize) -> Vec<String> {
    if content.is_empty() {
        return vec![String::new()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_bytes = 0usize;

    for ch in content.chars() {
        let b = ch.len_utf8();
        if cur_bytes + b > max_bytes && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
            cur_bytes = 0;
        }
        cur.push(ch);
        cur_bytes += b;
    }
    chunks.push(cur);
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_book_without_headings() {
        let ch = split_chapters("hello\nworld", "book.txt");
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].name, "book.txt");
    }

    #[test]
    fn chinese_numbered_headings() {
        let text = "第一章 开始\n内容一\n第二章 继续\n内容二";
        let ch = split_chapters(text, "b.txt");
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[0].name, "第一章 开始");
        assert_eq!(ch[1].index, 1);
    }

    #[test]
    fn preface_is_separate_chapter() {
        let text = "前言部分\n\n序章\n正文";
        let ch = split_chapters(text, "b.txt");
        assert_eq!(ch[0].name, "b.txt");
        assert_eq!(ch[1].name, "序章");
    }

    #[test]
    fn special_word_inside_sentence_not_heading() {
        let text = "第一章\n他说前言不搭后语，然后继续说话";
        let ch = split_chapters(text, "b.txt");
        assert_eq!(ch.len(), 1);
    }

    #[test]
    fn chapter_prefixed_heading() {
        let text = "Chapter 1\nbody\nChapter 2\nbody2";
        let ch = split_chapters(text, "b.txt");
        assert_eq!(ch.len(), 2);
    }

    #[test]
    fn weak_numeric_heading_used_only_without_strong() {
        let text = "1、开始\n内容\n2、继续\n内容";
        let ch = split_chapters(text, "b.txt");
        assert_eq!(ch.len(), 2);
    }

    #[test]
    fn chunk_never_splits_multibyte() {
        let s = "中".repeat(100);
        let chunks = split_into_chunks(&s, 10);
        for c in &chunks {
            assert!(c.len() <= 10);
        }
        assert_eq!(chunks.concat(), s);
    }

    #[test]
    fn chunk_preserves_emoji() {
        let s = "😀".repeat(50);
        let chunks = split_into_chunks(&s, 7);
        assert_eq!(chunks.concat(), s);
        for c in &chunks {
            assert!(c.len() <= 7);
        }
    }

    #[test]
    fn empty_content_yields_one_empty_chunk() {
        assert_eq!(split_into_chunks("", 16), vec![String::new()]);
    }

    #[test]
    fn indexes_are_sequential() {
        let text = "第一章 a\n1\n第二章 b\n2\n第三章 c\n3";
        let ch = split_chapters(text, "b.txt");
        for (i, c) in ch.iter().enumerate() {
            assert_eq!(c.index, i);
        }
    }

    // ---------------- 新增：各分章方式 ----------------

    fn opts(mode: SplitMode) -> SplitOptions {
        SplitOptions {
            mode,
            ..Default::default()
        }
    }

    #[test]
    fn mode_default_matches_app_regex() {
        let o = opts(SplitMode::Default);
        for line in ["第一章 开始", "第1章", "第 3 卷 风起", "第2回 归途", "番外 1", "番外"] {
            assert!(match_default(line.trim()), "应命中: {line}");
        }
        for line in ["他说第一章很好看", "前言不搭后语", "第x章"] {
            assert!(!match_default(line.trim()), "不应命中: {line}");
        }
        let long_tail = format!("第1章 {}", "字".repeat(31));
        assert!(!match_default(&long_tail), "超长尾部不应命中");
        let ch = split_chapters_with("第一章 a\nx\n第二章 b\ny", "b.txt", &o);
        assert_eq!(ch.len(), 2);
    }

    #[test]
    fn mode_loose_allows_missing_di() {
        let o = opts(SplitMode::Loose);
        for line in ["1 章 开始", "  2章", "第三章 尾声", "番外 3"] {
            assert!(match_loose(line.trim()), "应命中: {line}");
        }
        let ch = split_chapters_with("1 章 a\nx\n2章 b\ny", "b.txt", &o);
        assert_eq!(ch.len(), 2);
    }

    #[test]
    fn mode_english_requires_space_then_digits() {
        assert!(match_english("Chapter 1"));
        assert!(match_english("CHAPTER 42"));
        assert!(!match_english("ChapterOne"));
        assert!(!match_english("chapter"));
        let ch = split_chapters_with("Chapter 1\na\nChapter 2\nb", "b.txt", &opts(SplitMode::English));
        assert_eq!(ch.len(), 2);
    }

    #[test]
    fn mode_zh_num_dot() {
        assert!(match_zh_num_dot("一、开始"));
        assert!(match_zh_num_dot("三. 继续"));
        assert!(match_zh_num_dot("  十二、终"));
        assert!(!match_zh_num_dot("一开始就很好"));
        let ch = split_chapters_with("一、开始\nx\n二、继续\ny", "b.txt", &opts(SplitMode::ZhNumDot));
        assert_eq!(ch.len(), 2);
    }

    #[test]
    fn mode_digit_dot() {
        assert!(match_digit_dot("1、开始"));
        assert!(match_digit_dot("12. 继续"));
        assert!(!match_digit_dot("2024年"));
        let ch = split_chapters_with("1. a\nx\n2. b\ny", "b.txt", &opts(SplitMode::DigitDot));
        assert_eq!(ch.len(), 2);
    }

    #[test]
    fn mode_by_word_count() {
        let text = "字".repeat(25);
        let o = SplitOptions {
            mode: SplitMode::ByWordCount,
            words_per_chapter: 10,
            keyword: String::new(),
        };
        let ch = split_chapters_with(&text, "b.txt", &o);
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[0].name, "第 1 章");
        assert_eq!(ch[0].word_count, 10);
        assert_eq!(ch[2].word_count, 5);
    }

    #[test]
    fn mode_custom_keyword_prefix() {
        let o = SplitOptions {
            mode: SplitMode::Custom,
            words_per_chapter: 5000,
            keyword: "【卷|###".to_string(),
        };
        let text = "【卷一】起\n内容\n### 转折\n内容2";
        let ch = split_chapters_with(text, "b.txt", &o);
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[0].name, "【卷一】起");
    }

    #[test]
    fn mode_non_auto_uses_preface_and_fulltext_names() {
        let o = opts(SplitMode::DigitDot);
        let ch = split_chapters_with("引子内容\n1. 开始\n正文", "b.txt", &o);
        assert_eq!(ch[0].name, "前言");

        let ch2 = split_chapters_with("没有任何标题的正文", "b.txt", &o);
        assert_eq!(ch2.len(), 1);
        assert_eq!(ch2[0].name, "全文");
    }

    #[test]
    fn split_mode_cycles_through_all() {
        let mut m = SplitMode::Auto;
        let mut seen = vec![m];
        for _ in 0..7 {
            m = m.next();
            seen.push(m);
        }
        assert_eq!(m.next(), SplitMode::Auto);
        assert_eq!(seen.len(), 8);
    }
}
