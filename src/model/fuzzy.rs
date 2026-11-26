/// Result of a Unicode-aware fuzzy match, including character indices (not byte offsets)
/// for highlight rendering in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub matched_indices: Vec<usize>,
}

impl FuzzyMatch {
    /// Character indices in the target string that matched the query.
    pub fn indices(&self) -> &[usize] {
        &self.matched_indices
    }
}

const MATCH_BASE: i32 = 16;
const CONSECUTIVE_BONUS: i32 = 28;
const FIRST_CHAR_BONUS: i32 = 32;
const PATH_SEP_BONUS: i32 = 30;
const WORD_SEP_BONUS: i32 = 26;
const CAMEL_CASE_BONUS: i32 = 24;
const EXACT_CASE_BONUS: i32 = 2;
const EXACT_PREFIX_BONUS: i32 = 40;
const FULL_EXACT_BONUS: i32 = 60;
const GAP_START_PENALTY: i32 = 5;
const GAP_EXT_PENALTY: i32 = 1;
const MAX_LEADING_PENALTY: i32 = 12;
const NEG_INF: i32 = i32::MIN / 4;
const MAX_DP_CELLS: usize = 65_536;

/// Unicode-safe fzf/skim-style fuzzy matcher with smart-case and optimal alignment DP.
///
/// Scoring favors:
/// - Consecutive character runs
/// - Start-of-string and separator boundaries (`:`, `/`, `-`, `_`, `.`, space)
/// - CamelCase boundaries (`fooBar` -> `B`)
/// - Exact prefix and full exact matches
/// - Shorter candidates on ties
///
/// Smart-case behavior:
/// - All-lowercase query matches case-insensitively.
/// - Query containing any uppercase character matches case-sensitively.
pub fn fuzzy_match(query: &str, text: &str) -> Option<FuzzyMatch> {
    let q_trimmed = query.trim();
    if q_trimmed.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            matched_indices: Vec::new(),
        });
    }

    let q_orig: Vec<char> = q_trimmed.chars().collect();
    let t_orig: Vec<char> = text.chars().collect();
    let q_len = q_orig.len();
    let t_len = t_orig.len();

    if q_len > t_len {
        return None;
    }

    let case_sensitive = q_orig.iter().any(|c| c.is_uppercase());
    let q_cmp: Vec<char> = if case_sensitive {
        q_orig.clone()
    } else {
        q_orig.iter().copied().map(fold_char).collect()
    };
    let t_cmp: Vec<char> = if case_sensitive {
        t_orig.clone()
    } else {
        t_orig.iter().copied().map(fold_char).collect()
    };

    // Fast O(T) subsequence pre-check to reject non-matches with minimal work.
    if !is_subsequence(&q_cmp, &t_cmp) {
        return None;
    }

    let bonuses = compute_boundary_bonuses(&t_orig);

    let (raw_score, matched_indices) = if q_len.saturating_mul(t_len) <= MAX_DP_CELLS {
        dp_align(&q_orig, &q_cmp, &t_orig, &t_cmp, &bonuses, case_sensitive)
    } else {
        greedy_align(&q_orig, &q_cmp, &t_orig, &t_cmp, &bonuses, case_sensitive)
    };

    let mut score = raw_score;
    let is_prefix = matched_indices.first() == Some(&0)
        && matched_indices.windows(2).all(|w| w[1] == w[0] + 1);
    if is_prefix {
        score += EXACT_PREFIX_BONUS;
    }
    if matched_indices.len() == t_len {
        score += FULL_EXACT_BONUS;
        if q_orig == t_orig {
            score += 10;
        }
    }
    // Prefer shorter candidates when match quality is otherwise equal.
    score -= (t_len - q_len) as i32;

    Some(FuzzyMatch {
        score,
        matched_indices,
    })
}

#[inline]
fn fold_char(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

fn is_subsequence(q: &[char], t: &[char]) -> bool {
    let mut qi = 0usize;
    for &tc in t {
        if tc == q[qi] {
            qi += 1;
            if qi == q.len() {
                return true;
            }
        }
    }
    false
}

fn compute_boundary_bonuses(t_orig: &[char]) -> Vec<i32> {
    let mut bonuses = Vec::with_capacity(t_orig.len());
    for (i, &ch) in t_orig.iter().enumerate() {
        let bonus = if i == 0 {
            FIRST_CHAR_BONUS
        } else {
            let prev = t_orig[i - 1];
            match prev {
                ':' | '/' | '\\' => PATH_SEP_BONUS,
                '-' | '_' | '.' | ' ' | '@' => WORD_SEP_BONUS,
                _ if (prev.is_lowercase() && ch.is_uppercase())
                    || (prev.is_ascii_digit() && ch.is_alphabetic())
                    || (prev.is_alphabetic() && ch.is_ascii_digit()) =>
                {
                    CAMEL_CASE_BONUS
                }
                _ => 0,
            }
        };
        bonuses.push(bonus);
    }
    bonuses
}

/// Optimal affine-gap alignment DP (fzf/skim style) returning `(best_score, matched_indices)`.
fn dp_align(
    q_orig: &[char],
    q_cmp: &[char],
    t_orig: &[char],
    t_cmp: &[char],
    bonuses: &[i32],
    case_sensitive: bool,
) -> (i32, Vec<usize>) {
    let q_len = q_cmp.len();
    let t_len = t_cmp.len();

    // `d_score[qi][ti]`: best score where q[qi] matches t[ti].
    // `g_score[qi][ti]`: best score for q[0..=qi] ending strictly before ti (gap up to ti).
    let mut d_prev_row = vec![NEG_INF; t_len];
    let mut g_prev_row = vec![NEG_INF; t_len];
    let mut g_prev_src = vec![0usize; t_len];

    let mut backptr = vec![vec![0usize; t_len]; q_len];

    // Row 0
    for ti in 0..t_len {
        if t_cmp[ti] == q_cmp[0] {
            let case_bonus = if !case_sensitive && t_orig[ti] == q_orig[0] {
                EXACT_CASE_BONUS
            } else {
                0
            };
            let leading = (ti as i32).min(MAX_LEADING_PENALTY);
            d_prev_row[ti] = MATCH_BASE + bonuses[ti] + case_bonus - leading;
        }
        if ti > 0 {
            let from_d = d_prev_row[ti - 1] - GAP_START_PENALTY;
            let from_g = g_prev_row[ti - 1] - GAP_EXT_PENALTY;
            if from_d >= from_g {
                g_prev_row[ti] = from_d;
                g_prev_src[ti] = ti - 1;
            } else {
                g_prev_row[ti] = from_g;
                g_prev_src[ti] = g_prev_src[ti - 1];
            }
        }
    }

    let mut d_cur_row = vec![NEG_INF; t_len];
    let mut g_cur_row = vec![NEG_INF; t_len];
    let mut g_cur_src = vec![0usize; t_len];

    for qi in 1..q_len {
        d_cur_row.fill(NEG_INF);
        g_cur_row.fill(NEG_INF);
        g_cur_src.fill(0);

        for ti in qi..t_len {
            if t_cmp[ti] == q_cmp[qi] {
                let case_bonus = if !case_sensitive && t_orig[ti] == q_orig[qi] {
                    EXACT_CASE_BONUS
                } else {
                    0
                };

                let consec = if d_prev_row[ti - 1] > NEG_INF / 2 {
                    d_prev_row[ti - 1]
                        + MATCH_BASE
                        + CONSECUTIVE_BONUS.max(bonuses[ti])
                        + case_bonus
                } else {
                    NEG_INF
                };

                let gapped = if g_prev_row[ti - 1] > NEG_INF / 2 {
                    g_prev_row[ti - 1] + MATCH_BASE + bonuses[ti] + case_bonus
                } else {
                    NEG_INF
                };

                if consec >= gapped && consec > NEG_INF / 2 {
                    d_cur_row[ti] = consec;
                    backptr[qi][ti] = ti - 1;
                } else if gapped > NEG_INF / 2 {
                    d_cur_row[ti] = gapped;
                    backptr[qi][ti] = g_prev_src[ti - 1];
                }
            }

            if ti > qi {
                let from_d = d_cur_row[ti - 1] - GAP_START_PENALTY;
                let from_g = g_cur_row[ti - 1] - GAP_EXT_PENALTY;
                if from_d >= from_g {
                    g_cur_row[ti] = from_d;
                    g_cur_src[ti] = ti - 1;
                } else {
                    g_cur_row[ti] = from_g;
                    g_cur_src[ti] = g_cur_src[ti - 1];
                }
            }
        }

        std::mem::swap(&mut d_prev_row, &mut d_cur_row);
        std::mem::swap(&mut g_prev_row, &mut g_cur_row);
        std::mem::swap(&mut g_prev_src, &mut g_cur_src);
    }

    let mut best_ti = q_len - 1;
    let mut best_score = NEG_INF;
    for (ti, &score) in d_prev_row.iter().enumerate().skip(q_len - 1) {
        if score > best_score {
            best_score = score;
            best_ti = ti;
        }
    }

    let mut indices = Vec::with_capacity(q_len);
    let mut cur = best_ti;
    for qi in (0..q_len).rev() {
        indices.push(cur);
        if qi > 0 {
            cur = backptr[qi][cur];
        }
    }
    indices.reverse();

    (best_score, indices)
}

/// Linear fallback for very long texts where `q_len * t_len > MAX_DP_CELLS`.
fn greedy_align(
    q_orig: &[char],
    q_cmp: &[char],
    t_orig: &[char],
    t_cmp: &[char],
    bonuses: &[i32],
    case_sensitive: bool,
) -> (i32, Vec<usize>) {
    let mut qi = 0usize;
    let mut score = 0i32;
    let mut indices = Vec::with_capacity(q_cmp.len());
    let mut prev_match: Option<usize> = None;

    for (ti, &tc) in t_cmp.iter().enumerate() {
        if qi < q_cmp.len() && tc == q_cmp[qi] {
            let case_bonus = if !case_sensitive && t_orig[ti] == q_orig[qi] {
                EXACT_CASE_BONUS
            } else {
                0
            };
            if let Some(prev) = prev_match {
                if ti == prev + 1 {
                    score += MATCH_BASE + CONSECUTIVE_BONUS.max(bonuses[ti]) + case_bonus;
                } else {
                    let gap = (ti - prev - 1) as i32;
                    score += MATCH_BASE + bonuses[ti] + case_bonus
                        - GAP_START_PENALTY
                        - (gap - 1) * GAP_EXT_PENALTY;
                }
            } else {
                let leading = (ti as i32).min(MAX_LEADING_PENALTY);
                score += MATCH_BASE + bonuses[ti] + case_bonus - leading;
            }
            indices.push(ti);
            prev_match = Some(ti);
            qi += 1;
        }
    }

    (score, indices)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn empty_and_whitespace_query_match_with_zero_score() {
        let m = fuzzy_match("   ", "anything").unwrap();
        assert_eq!(m.score, 0);
        assert!(m.matched_indices.is_empty());
    }

    #[test]
    fn non_matching_returns_none() {
        assert_eq!(fuzzy_match("xyz", "abc"), None);
        assert_eq!(fuzzy_match("longer", "short"), None);
    }

    #[test]
    fn prefers_exact_and_prefix_and_shorter_candidates() {
        let exact = fuzzy_match("node", "node").unwrap();
        let prefix_short = fuzzy_match("node", "nodejs").unwrap();
        let prefix_long = fuzzy_match("node", "node-version-manager").unwrap();
        let middle = fuzzy_match("node", "my-node-tool").unwrap();
        let scattered = fuzzy_match("node", "n_o_d_e").unwrap();

        assert!(exact.score > prefix_short.score);
        assert!(prefix_short.score > prefix_long.score);
        assert!(prefix_long.score > middle.score);
        assert!(exact.score > scattered.score);
        assert_eq!(exact.indices(), &[0, 1, 2, 3]);
    }

    #[test]
    fn prefers_separator_boundaries_over_early_scattered_match() {
        let m = fuzzy_match("bt", "about:build-test").unwrap();
        // Should pick 'b' after ':' (6) and 't' after '-' (12), not 'b'/'t' inside "about" (1, 4).
        assert_eq!(m.matched_indices, vec![6, 12]);
    }

    #[test]
    fn prefers_consecutive_run_over_scattered_separators() {
        let m = fuzzy_match("foo", "f_o_o_foo").unwrap();
        assert_eq!(m.matched_indices, vec![6, 7, 8]);
    }

    #[test]
    fn scores_camel_case_boundaries() {
        let camel = fuzzy_match("bt", "buildTest").unwrap();
        let flat = fuzzy_match("bt", "buildtest").unwrap();
        assert_eq!(camel.matched_indices, vec![0, 5]);
        assert!(camel.score > flat.score);
    }

    #[test]
    fn smart_case_lowercase_is_insensitive_and_uppercase_is_sensitive() {
        // All lowercase -> case-insensitive
        assert!(fuzzy_match("foobar", "FooBar").is_some());
        assert!(fuzzy_match("rust", "RUST_LOG").is_some());

        // Contains uppercase -> case-sensitive
        assert!(fuzzy_match("Foo", "FooBar").is_some());
        assert_eq!(fuzzy_match("Foo", "foobar"), None);
        assert_eq!(fuzzy_match("RUST", "rust_log"), None);
        assert!(fuzzy_match("RUST", "RUST_LOG").is_some());
    }

    #[test]
    fn handles_chinese_and_emoji_unicode_safely() {
        let cjk = fuzzy_match("构建", "项目:构建测试").unwrap();
        assert_eq!(cjk.matched_indices, vec![3, 4]);

        let emoji_text = "🚀部署-测试🎉完成";
        let m = fuzzy_match("部署🎉", emoji_text).unwrap();
        assert_eq!(m.matched_indices, vec![1, 2, 6]);
        let chars: Vec<char> = emoji_text.chars().collect();
        let reconstructed: String = m.matched_indices.iter().map(|&i| chars[i]).collect();
        assert_eq!(reconstructed, "部署🎉");
    }

    #[test]
    fn handles_large_input_without_panicking() {
        let big_text = "a".repeat(5000) + "target_word";
        let query = "target_word".repeat(2); // fails fast
        assert_eq!(fuzzy_match(&query, &big_text), None);

        let long_query = "a".repeat(20) + "target";
        let m = fuzzy_match(&long_query, &big_text).unwrap();
        assert_eq!(m.matched_indices.len(), long_query.chars().count());
    }
}
