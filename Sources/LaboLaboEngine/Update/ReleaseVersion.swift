import Foundation

/// リリースバージョンの比較。GitHub タグ（"v0.3.2"）とアプリ版（"0.3.2"）の比較などに使う。
/// 数値ドット区切りのみを見て比較し、`-beta` などの非数値サフィックスは無視する。
public enum ReleaseVersion {

    /// 先頭の "v"/"V" と前後空白を除いた版文字列。
    public static func normalize(_ tag: String) -> String {
        var s = tag.trimmingCharacters(in: .whitespacesAndNewlines)
        if let first = s.first, first == "v" || first == "V" { s.removeFirst() }
        return s
    }

    /// `a` が `b` より新しければ true。
    public static func isNewer(_ a: String, than b: String) -> Bool {
        compare(a, b) > 0
    }

    /// -1（a<b）/ 0（等）/ 1（a>b）。前置の "v" は無視。
    public static func compare(_ a: String, _ b: String) -> Int {
        let pa = parts(normalize(a))
        let pb = parts(normalize(b))
        for i in 0 ..< max(pa.count, pb.count) {
            let x = i < pa.count ? pa[i] : 0
            let y = i < pb.count ? pb[i] : 0
            if x != y { return x < y ? -1 : 1 }
        }
        return 0
    }

    /// "0.3.2" → [0, 3, 2]。各セグメントの先頭数値のみ採用（"2-beta" → 2）。
    private static func parts(_ v: String) -> [Int] {
        v.split(separator: ".").map { segment in
            Int(segment.prefix(while: { $0.isNumber })) ?? 0
        }
    }
}
