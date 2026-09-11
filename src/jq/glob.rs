//! yq's wildcard key matcher, ported byte-for-byte from
//! `pkg/yqlib/matchKeyString.go` (mikefarah/yq v4.53.3) -- issue #2785.
//!
//! Real yq routes two very different operations through one matcher: a
//! mapping traversal by key (`.["a*"]`, `has("a*")`) and the scalar half of
//! `==`/`!=` (`isEquals` in `operator_equals.go` calls
//! `matchKey(lhs.Value, rhs.Value)` whenever both operands are scalars). The
//! matcher is Russ Cox's linear-time glob (<https://research.swtch.com/glob>):
//! `*` matches zero or more bytes, `?` matches exactly one *byte* -- Go
//! indexes the string by byte, so a multi-byte character needs one `?` per
//! byte -- and nothing else is special: no character classes, no escaping,
//! so `"[a]"` matches only the literal three bytes `[a]`.
//!
//! Only the **pattern** side is interpreted. `"abc" == "a*"` is `true` and
//! `"a*" == "abc"` is `false` (both captured live), which is why yq's `==`
//! is neither symmetric nor transitive and why this lives beside, not
//! inside, `owned_value_eq` -- a matcher this shape cannot serve as a
//! dedup key for `unique`/`group_by`, and real yq's own dedup keys on the
//! plain text (`operator_unique.go`, `operator_group_by.go`).

/// Whether `name` matches yq's wildcard `pattern` -- `matchKey` in
/// `matchKeyString.go`, including its two short-circuits: an empty
/// pattern matches only an empty name, and a bare `*` matches anything.
///
/// A pattern with no `*`/`?` at all is compared as plain bytes without
/// entering the matcher, so the common string-against-string `==` stays a
/// single `memcmp`.
pub(crate) fn yq_match_key(name: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return name.is_empty();
    }
    if pattern == "*" {
        return true;
    }
    if !pattern.bytes().any(|b| b == b'*' || b == b'?') {
        return name == pattern;
    }
    deep_match(name.as_bytes(), pattern.as_bytes())
}

/// `deepMatch` in `matchKeyString.go`, verbatim in shape: one pass with a
/// single restart point after the most recent `*`, so a name of `n` bytes
/// against a pattern of `p` bytes costs O(n·p) at worst and O(n + p) with
/// no backtracking, never the exponential blow-up of the naive recursion.
fn deep_match(name: &[u8], pattern: &[u8]) -> bool {
    let mut px = 0;
    let mut nx = 0;
    let mut next_px = 0;
    let mut next_nx = 0;
    while px < pattern.len() || nx < name.len() {
        if px < pattern.len() {
            match pattern[px] {
                b'?' => {
                    if nx < name.len() {
                        px += 1;
                        nx += 1;
                        continue;
                    }
                }
                b'*' => {
                    // Try to match at `nx`; if that fails, restart at
                    // `nx + 1` from this same star.
                    next_px = px;
                    next_nx = nx + 1;
                    px += 1;
                    continue;
                }
                c => {
                    if nx < name.len() && name[nx] == c {
                        px += 1;
                        nx += 1;
                        continue;
                    }
                }
            }
        }
        // Mismatch: back up to the last star, if one can still consume.
        if 0 < next_nx && next_nx <= name.len() {
            px = next_px;
            nx = next_nx;
            continue;
        }
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::yq_match_key;

    /// Every row captured live from yq v4.53.3 as `<name> == <pattern>`
    /// with `-o=json -I0`.
    #[test]
    fn matches_yq_captured_rows_2785() {
        for (name, pattern, expected) in [
            ("abc", "a*", true),
            ("a*", "abc", false),
            ("abc", "a?c", true),
            ("1", "*", true),
            ("true", "t*", true),
            ("[", "[", true),
            ("[", "[a", false),
            ("a", "[a]", false),
            ("Abc", "a*", false),
            ("a", "A", false),
            ("", "", true),
            ("", "*", true),
            ("a", "", false),
            ("nul?", "null", false),
            ("a", "a?", false),
            ("ab", "a?", true),
            ("x", "x", true),
        ] {
            assert_eq!(
                yq_match_key(name, pattern),
                expected,
                "{name:?} against {pattern:?}"
            );
        }
    }

    /// The restart-after-star path: a later literal forces the first `*`
    /// to give back bytes it consumed.
    #[test]
    fn star_backtracks_to_a_later_literal_2785() {
        assert!(yq_match_key("axxbyc", "a*b*c"));
        assert!(yq_match_key("abc", "a*b*c"));
        assert!(!yq_match_key("axxbyd", "a*b*c"));
        assert!(yq_match_key("aXb", "*b"));
        assert!(!yq_match_key("aXb", "*c"));
        assert!(yq_match_key("aaa", "a*a"));
        assert!(!yq_match_key("ab", "a?b"));
    }

    /// `?` consumes one *byte*, as Go's byte-indexed loop does: a two-byte
    /// character needs two of them.
    #[test]
    fn question_mark_is_one_byte_not_one_char_2785() {
        assert!(!yq_match_key("é", "?"));
        assert!(yq_match_key("é", "??"));
        assert!(yq_match_key("é", "*"));
    }
}
