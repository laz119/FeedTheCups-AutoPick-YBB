//! 选卡队列解析。
//!
//! 与 MBB v1.1.4 的 `parse_queue_text` / `_split_plus` 行为对齐，但**分隔符的判定反过来做**：
//! 原版枚举了一串箭头（`[→⇒➡»＞>，,、;；|｜\s]+`），漏掉 `👉` 就整行变成一个假卡名；
//! 这里改成「任何不是卡名字符的字符都算分隔符」，以后新增什么表情都不会再出问题。

/// 卡名字符：汉字 / 字母 / 数字 / `+`（`+` 是「升级版」后缀，属于卡名的一部分）。
fn is_name_char(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'   // CJK 统一表意文字
        | '0'..='9'
        | 'a'..='z'
        | 'A'..='Z'
        | '+'
    )
}

/// 把用户贴进来的一坨文本切成有序卡名列表。
///
/// 先做 NFKC 归一化：中文输入法的全角 `＋`（U+FF0B）会转成半角 `+`（U+002B），
/// 否则 `is_name_char` 不认识全角加号，`气球＋` 会被拆成 `气球`，丢失升级标记。
pub fn parse_queue(text: &str) -> Vec<String> {
    use unicode_normalization::UnicodeNormalization;
    let text: String = text.nfkc().collect();
    let mut out = Vec::new();
    let mut cur = String::new();

    for ch in text.chars() {
        if is_name_char(ch) {
            cur.push(ch);
        } else if !cur.is_empty() {
            push_chunk(&mut out, &cur);
            cur.clear();
        }
    }
    if !cur.is_empty() {
        push_chunk(&mut out, &cur);
    }
    out
}

/// 单个片段入队。与原版 `_split_plus` 等价：
/// 以 `+` 结尾的不拆（那是卡名的一部分）；中间含 `+` 的按**第一个** `+` 拆成 `X+` 与 `Y`。
fn push_chunk(out: &mut Vec<String>, chunk: &str) {
    if chunk.is_empty() {
        return;
    }
    match split_plus(chunk) {
        Some((left, right)) => {
            out.push(left);
            out.push(right);
        }
        None => out.push(chunk.to_string()),
    }
}

fn split_plus(s: &str) -> Option<(String, String)> {
    if s.ends_with('+') || !s.contains('+') {
        return None;
    }
    let i = s.find('+')?;
    let left = s[..i].trim();
    let right = s[i + 1..].trim();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    Some((format!("{left}+"), right.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用户实际给的那一条（用 👉 分隔，原版无法识别）
    const USER_INPUT: &str = "内卷饭碗👉零花钱👉零花钱+ 👉延时抹布👉准时达👉永生花👉气球👉争分夺秒👉气球+ \
                              👉透心凉👉冰点连击👉快乐心情👉永生花+ 👉透心凉+ 👉飞来横财👉若有若无👉若有若无+ \
                              👉延时抹布+ 👉完美一天👉连击达人";

    const EXPECTED: [&str; 20] = [
        "内卷饭碗", "零花钱", "零花钱+", "延时抹布", "准时达",
        "永生花", "气球", "争分夺秒", "气球+", "透心凉",
        "冰点连击", "快乐心情", "永生花+", "透心凉+", "飞来横财",
        "若有若无", "若有若无+", "延时抹布+", "完美一天", "连击达人",
    ];

    /// 用户那条 👉 串必须切成 20 轮，顺序不能错
    #[test]
    fn user_arrow_input_splits_into_20() {
        assert_eq!(parse_queue(USER_INPUT), EXPECTED);
    }

    /// 各种箭头/标点都当分隔符
    #[test]
    fn any_symbol_is_separator() {
        for sep in ["→", "->", "=>", "－>", "👉", "＞", ">", "、", ";", "|", "  ", "\n", "，"] {
            let text = format!("气球{sep}永生花");
            assert_eq!(parse_queue(&text), vec!["气球", "永生花"], "分隔符 {sep:?} 失效");
        }
    }

    /// 结尾的 + 属于卡名，不拆
    #[test]
    fn trailing_plus_stays() {
        assert_eq!(parse_queue("零花钱+"), vec!["零花钱+"]);
        assert_eq!(parse_queue("气球+👉永生花+"), vec!["气球+", "永生花+"]);
    }

    /// 紧挨着的 + 按第一个拆分（对齐原版 _split_plus）
    #[test]
    fn glued_plus_splits_on_first() {
        assert_eq!(parse_queue("永生花+猪猪罐"), vec!["永生花+", "猪猪罐"]);
        assert_eq!(parse_queue("a+b+c"), vec!["a+", "b+c"]);
    }

    /// 引号/方括号在切分时就已丢弃
    #[test]
    fn quotes_are_dropped() {
        assert_eq!(parse_queue("「气球」【永生花+】"), vec!["气球", "永生花+"]);
        assert_eq!(parse_queue("\"零花钱\",'准时达'"), vec!["零花钱", "准时达"]);
    }

    #[test]
    fn empty_input_gives_empty() {
        assert!(parse_queue("").is_empty());
        assert!(parse_queue("👉👉👉 ，， ").is_empty());
    }

    #[test]
    fn digits_and_latin_allowed() {
        assert_eq!(parse_queue("2倍速👉AAA"), vec!["2倍速", "AAA"]);
    }
}

/// 从队列文件内容里解析（会跳过 `#` 开头的注释行，原版 cards.txt 第一行就是注释）
pub fn parse_queue_file(text: &str) -> Vec<String> {
    let body: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    parse_queue(&body)
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn comment_lines_are_skipped() {
        let t = "# 由界面自动保存\n气球→永生花\n# 备注\n零花钱+";
        assert_eq!(parse_queue_file(t), vec!["气球", "永生花", "零花钱+"]);
    }
}
