/// Word-level diff between the original and the rewritten text, for the
/// popup's diff/side-by-side comparison views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffToken {
    Same(String),
    Removed(String),
    Added(String),
}

const TOKEN_LIMIT: usize = 4000;

/// Cached word diff, invalidated when either text changes. The immediate-mode
/// UI calls this every frame, so the O(n*m) pass must not rerun each time.
#[derive(Default)]
pub struct DiffCache {
    key: u64,
    tokens: Option<Vec<DiffToken>>,
    computed: bool,
}

impl DiffCache {
    pub fn key_for(original: &str, result: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        original.hash(&mut hasher);
        result.hash(&mut hasher);
        hasher.finish()
    }

    /// Returns the diff tokens, computing (and memoizing) them only when the
    /// key changed. `None` (text too large) is cached like any other result.
    pub fn get_or_compute(
        &mut self,
        key: u64,
        compute: impl FnOnce() -> Option<Vec<DiffToken>>,
    ) -> Option<&[DiffToken]> {
        if !self.computed || self.key != key {
            self.tokens = compute();
            self.key = key;
            self.computed = true;
        }
        self.tokens.as_deref()
    }
}

/// Split into word and whitespace runs so the diff preserves exact spacing.
fn tokenize(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut in_ws = s.chars().next().is_some_and(|c| c.is_whitespace());
    for (i, c) in s.char_indices() {
        if c.is_whitespace() != in_ws {
            out.push(&s[start..i]);
            start = i;
            in_ws = !in_ws;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// LCS diff over word tokens. Returns `None` when either text is too large
/// for the O(n*m) table (the UI then falls back to plain side-by-side).
pub fn word_diff(original: &str, result: &str) -> Option<Vec<DiffToken>> {
    if original == result {
        return Some(vec![DiffToken::Same(original.to_string())]);
    }
    let a = tokenize(original);
    let b = tokenize(result);
    if a.len() > TOKEN_LIMIT || b.len() > TOKEN_LIMIT {
        return None;
    }
    let (n, m) = (a.len(), b.len());
    // lcs[i][j] = LCS length of a[i..] and b[j..]
    let stride = m + 1;
    let mut lcs = vec![0u32; (n + 1) * stride];
    let mut i = n;
    while i > 0 {
        i -= 1;
        let mut j = m;
        while j > 0 {
            j -= 1;
            lcs[i * stride + j] = if a[i] == b[j] {
                lcs[(i + 1) * stride + (j + 1)] + 1
            } else {
                lcs[(i + 1) * stride + j].max(lcs[i * stride + (j + 1)])
            };
        }
    }

    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffToken::Same(a[i].to_string()));
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * stride + j] >= lcs[i * stride + (j + 1)] {
            out.push(DiffToken::Removed(a[i].to_string()));
            i += 1;
        } else {
            out.push(DiffToken::Added(b[j].to_string()));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|t| DiffToken::Removed(t.to_string())));
    out.extend(b[j..].iter().map(|t| DiffToken::Added(t.to_string())));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DiffToken::*;

    fn flat(tokens: &[DiffToken]) -> (String, String) {
        let mut orig = String::new();
        let mut new = String::new();
        for t in tokens {
            match t {
                Same(s) => {
                    orig.push_str(s);
                    new.push_str(s);
                }
                Removed(s) => orig.push_str(s),
                Added(s) => new.push_str(s),
            }
        }
        (orig, new)
    }

    #[test]
    fn identical_text_is_all_same() {
        let d = word_diff("hello world", "hello world").unwrap();
        assert_eq!(d, vec![Same("hello world".into())]);
    }

    #[test]
    fn diff_reconstructs_both_sides() {
        let orig = "the quick brown fox jumps over the lazy dog";
        let new = "the quick red fox leaps over a sleepy dog";
        let d = word_diff(orig, new).unwrap();
        let (o, r) = flat(&d);
        assert_eq!(o, orig);
        assert_eq!(r, new);
        assert!(d.contains(&Removed("brown".into())));
        assert!(d.contains(&Added("red".into())));
        assert!(d.contains(&Same("the".into())));
    }

    #[test]
    fn empty_side_becomes_all_added_or_removed() {
        let d = word_diff("", "added text").unwrap();
        assert!(matches!(d[0], Added(_)));
        let d = word_diff("removed text", "").unwrap();
        assert!(matches!(d[0], Removed(_)));
    }

    #[test]
    fn oversized_input_returns_none() {
        let big = "word ".repeat(TOKEN_LIMIT + 1);
        assert!(word_diff(&big, "x").is_none());
    }

    #[test]
    fn cache_computes_once_per_key() {
        use std::cell::Cell;
        let mut cache = DiffCache::default();
        let calls = Cell::new(0u8);
        let compute = |tokens: &'static str| {
            calls.set(calls.get() + 1);
            Some(vec![DiffToken::Same(tokens.to_string())])
        };
        let key = DiffCache::key_for("a", "b");
        assert!(cache.get_or_compute(key, || compute("first")).is_some());
        assert!(cache.get_or_compute(key, || compute("second")).is_some());
        assert_eq!(calls.get(), 1, "same key must reuse the cached tokens");
        let other = DiffCache::key_for("a", "c");
        assert_eq!(
            cache.get_or_compute(other, || compute("third")),
            Some(&[DiffToken::Same("third".into())][..])
        );
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn cache_stores_none_for_oversized_input() {
        let mut cache = DiffCache::default();
        let big = "word ".repeat(TOKEN_LIMIT + 1);
        let key = DiffCache::key_for(&big, "x");
        for _ in 0..2 {
            assert!(cache
                .get_or_compute(key, || word_diff(&big, "x"))
                .is_none());
        }
    }

    #[test]
    fn tokenizer_splits_words_and_whitespace() {
        assert_eq!(tokenize("a b\nc"), vec!["a", " ", "b", "\n", "c"]);
        assert_eq!(tokenize(""), Vec::<&str>::new());
    }
}
