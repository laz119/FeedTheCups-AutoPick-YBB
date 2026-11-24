//! 卡名归一化与相似度打分。
//!
//! 与原版 `norm` / `score` 对齐。差别只在 `difflib.SequenceMatcher.ratio()` 这里是自己实现的
//! Ratcliff/Obershelp 简化版（卡名最长 6 个字，O(n·m·len) 完全够用）。

use unicode_normalization::UnicodeNormalization;

/// 归一化：先 NFKC（全角转半角等），再去掉标点/空白。
///
/// 去掉的字符集与原版正则
/// `[\s\u3000·・.,，。!！?？:：;；'\"“”‘’()（）\[\]【】<>《》|/\\_-]` 一致。
/// 注意 `+` **不**在去除集合里，它是卡名的一部分。
pub fn norm(s: &str) -> String {
    s.nfkc().filter(|c| !is_stripped(*c)).collect()
}

fn is_stripped(c: char) -> bool {
    if c.is_whitespace() {
        return true;
    }
    matches!(
        c,
        '\u{3000}' // 全角空格
        | '·' | '・'
        | '.' | ',' | '，' | '。'
        | '!' | '！' | '?' | '？'
        | ':' | '：' | ';' | '；'
        | '\'' | '"' | '“' | '”' | '‘' | '’'
        | '(' | ')' | '（' | '）'
        | '[' | ']' | '【' | '】'
        | '<' | '>' | '《' | '》'
        | '|' | '/' | '\\' | '_' | '-'
    )
}

/// 目标卡名与识别结果的匹配分。≥ `MATCH_THRESHOLD`(0.55) 视为同一张卡。
pub fn score(target: &str, cand: &str) -> f64 {
    if cand.is_empty() {
        return 0.0;
    }
    if target == cand {
        return 1.0;
    }
    // 「升级版」只差一个 +，是同一张卡
    if base(target) == base(cand) {
        return 0.85;
    }
    // 候选比目标多认了字：最多容忍 1 个多余字符
    if cand.contains(target) {
        let extra = cand.chars().count() - target.chars().count();
        return if extra <= 1 { 0.95 } else { 0.0 };
    }
    // 目标包含候选 = 识别少认了字；除了上面的 + 变体，一律不接受。
    // 这正是「气球」被「球」命中的原因，必须直接拒绝。
    if target.contains(cand) {
        return 0.0;
    }
    // 候选比目标短，或者长度差太大，都直接拒绝；宁可漏选也不选错。
    let tl = target.chars().count();
    let cl = cand.chars().count();
    if cl < tl || tl.min(cl) * 2 < tl.max(cl) {
        return 0.0;
    }
    ratio(target, cand)
}

fn base(s: &str) -> &str {
    s.strip_suffix('+').unwrap_or(s)
}

/// `difflib.SequenceMatcher(None, a, b).ratio()` 的等价实现：`2*M/T`，
/// M 为所有匹配块长度之和，T 为两串长度和。
pub fn ratio(a: &str, b: &str) -> f64 {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let total = av.len() + bv.len();
    if total == 0 {
        return 1.0;
    }
    let mut matched = 0usize;
    matching_total(&av, &bv, 0, av.len(), 0, bv.len(), &mut matched);
    2.0 * matched as f64 / total as f64
}

fn matching_total(
    a: &[char],
    b: &[char],
    alo: usize,
    ahi: usize,
    blo: usize,
    bhi: usize,
    acc: &mut usize,
) {
    let (i, j, size) = longest_match(a, b, alo, ahi, blo, bhi);
    if size == 0 {
        return;
    }
    *acc += size;
    matching_total(a, b, alo, i, blo, j, acc);
    matching_total(a, b, i + size, ahi, j + size, bhi, acc);
}

/// 最长公共连续子串，平局时取「更靠前的 i，再更靠前的 j」，与 difflib 的取法一致。
fn longest_match(
    a: &[char],
    b: &[char],
    alo: usize,
    ahi: usize,
    blo: usize,
    bhi: usize,
) -> (usize, usize, usize) {
    let (mut bi, mut bj, mut bs) = (alo, blo, 0usize);
    for i in alo..ahi {
        for j in blo..bhi {
            let mut k = 0usize;
            while i + k < ahi && j + k < bhi && a[i + k] == b[j + k] {
                k += 1;
            }
            if k > bs {
                bs = k;
                bi = i;
                bj = j;
            }
        }
    }
    (bi, bj, bs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_is_one() {
        assert_eq!(score("气球", "气球"), 1.0);
        assert_eq!(score("零花钱+", "零花钱+"), 1.0);
    }

    #[test]
    fn empty_candidate_is_zero() {
        assert_eq!(score("气球", ""), 0.0);
    }

    #[test]
    fn containing_gives_fixed_scores() {
        // 候选里含目标（识别多认了字）
        assert_eq!(score("气球", "气球X"), 0.95);
        // 目标里含候选（识别少认了字）
        assert_eq!(score("零花钱+", "零花钱"), 0.85);
    }

    #[test]
    fn plus_variant_still_matches() {
        // 「气球」与「气球+」必须能匹配上，否则会把升级版当成不存在
        assert!(score("气球+", "气球") >= 0.55);
        assert!(score("气球", "气球+") >= 0.55);
    }

    #[test]
    fn partial_ocr_is_rejected() {
        // 气球 少认成 球，不能靠 target.contains(cand) 混过去
        assert!(score("气球", "球") < 0.55);
        assert!(score("气球+", "球") < 0.55);
        assert!(score("零花钱", "零钱") < 0.55);
        assert!(score("内卷饭碗", "内卷") < 0.55);
    }

    #[test]
    fn different_cards_rejected() {
        assert!(score("气球", "永生花") < 0.55);
        assert!(score("内卷饭碗", "营业执照") < 0.55);
    }

    #[test]
    fn ratio_matches_difflib_semantics() {
        // 'abcd' vs 'bcde' -> 匹配块 'bcd' 长度 3，T=8 -> 0.75
        assert!((ratio("abcd", "bcde") - 0.75).abs() < 1e-12);
        assert!((ratio("abc", "abc") - 1.0).abs() < 1e-12);
        assert_eq!(ratio("abc", "xyz"), 0.0);
    }

    #[test]
    fn norm_folds_fullwidth_and_drops_punct() {
        assert_eq!(norm("气球＋"), "气球+"); // NFKC: ＋ -> +
        assert_eq!(norm("【气球】"), "气球"); // 【】 在原版 norm 的正则里
        assert_eq!(norm(" 零 花 钱 "), "零花钱");
        assert_eq!(norm("零花钱+！"), "零花钱+");
    }

    /// 保真度用例：原版 `norm` 的正则里**没有** `「」`，它是在 `parse_queue_text`
    /// 切分阶段被 strip 掉的。这里必须保持同样的分工，否则两边行为会漂移。
    #[test]
    fn norm_keeps_corner_brackets() {
        assert_eq!(norm("「气球」"), "「气球」");
    }
}
