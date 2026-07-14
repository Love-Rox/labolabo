//! Faithful port of `Sources/LaboLaboEngine/Update/ReleaseVersion.swift`.
//!
//! Release version comparison, e.g. for comparing a GitHub tag ("v0.3.2")
//! against the app's own version ("0.3.2"). Compares only dot-separated
//! numeric segments; non-numeric suffixes like `-beta` are ignored.

/// Version string with a leading `v`/`V` and surrounding whitespace stripped.
pub fn normalize(tag: &str) -> String {
    let trimmed = tag.trim();
    match trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
    {
        Some(rest) => rest.to_string(),
        None => trimmed.to_string(),
    }
}

/// `true` if `a` is newer than `b`.
pub fn is_newer(a: &str, b: &str) -> bool {
    compare(a, b) > 0
}

/// -1 (a<b) / 0 (equal) / 1 (a>b). A leading "v" is ignored on both sides.
pub fn compare(a: &str, b: &str) -> i32 {
    let pa = parts(&normalize(a));
    let pb = parts(&normalize(b));
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    0
}

/// "0.3.2" -> [0, 3, 2]. Only the leading numeric run of each segment is used
/// ("2-beta" -> 2); a segment with no leading digits (or one that overflows
/// `i64`) becomes 0.
fn parts(v: &str) -> Vec<i64> {
    v.split('.')
        .map(|segment| {
            let digits: String = segment.chars().take_while(|c| c.is_numeric()).collect();
            digits.parse::<i64>().unwrap_or(0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported 1:1 from Tests/LaboLaboEngineTests/ReleaseVersionTests.swift.

    #[test]
    fn normalize_strips_v_prefix_and_whitespace() {
        assert_eq!(normalize("v0.3.2"), "0.3.2");
        assert_eq!(normalize("V1.0"), "1.0");
        assert_eq!(normalize("  v2.1.0  "), "2.1.0");
        assert_eq!(normalize("0.3.2"), "0.3.2");
    }

    #[test]
    fn is_newer_basic() {
        assert!(is_newer("v0.3.3", "0.3.2"));
        assert!(is_newer("0.4.0", "0.3.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("v0.3.10", "v0.3.9")); // 数値比較（辞書順でない）
    }

    #[test]
    fn not_newer_when_equal_or_older() {
        assert!(!is_newer("0.3.2", "0.3.2"));
        assert!(!is_newer("v0.3.2", "0.3.2"));
        assert!(!is_newer("0.3.1", "0.3.2"));
        assert!(!is_newer("0.9.9", "1.0.0"));
    }

    #[test]
    fn different_segment_counts_are_equal() {
        assert_eq!(compare("0.3", "0.3.0"), 0);
        assert_eq!(compare("1", "1.0.0"), 0);
        assert!(is_newer("0.3.1", "0.3"));
    }

    #[test]
    fn non_numeric_suffix_ignored() {
        assert_eq!(compare("0.3.2-beta", "0.3.2"), 0);
        assert!(is_newer("0.3.3-rc1", "0.3.2"));
    }
}
