//! Small string helpers shared by the ported parsers.

/// Mirrors Swift's `String.dropFirst(_ n:)`: drops the first `n`
/// *characters* (not bytes) and, importantly, **clamps** rather than
/// panicking when the string has fewer than `n` characters (returns `""`
/// in that case). The Swift parsers rely on this clamping behavior for
/// short/malformed lines (e.g. a `"\"` diff line with nothing after it).
pub(crate) fn drop_first_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((idx, _)) => &s[idx..],
        None => "",
    }
}

/// Mirrors Swift's `token.split(separator: " ")` default (no `maxSplits`),
/// which omits empty subsequences: consecutive spaces collapse and
/// leading/trailing spaces produce no empty leading/trailing element.
pub(crate) fn split_space_omitting_empty(s: &str) -> Vec<&str> {
    s.split(' ').filter(|p| !p.is_empty()).collect()
}
