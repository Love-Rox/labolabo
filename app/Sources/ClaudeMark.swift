import SwiftUI

/// Claude を象徴するオレンジのサンバースト（放射状のマーク）をベクター描画する。
/// 公式ロゴ（商標）をそのまま埋め込む代わりの、差し替え可能なベクター表現。
/// 起動ボタンなど「Claude を起動」を示す用途に使う。
struct ClaudeMark: View {
    var color: Color = Color(red: 0.85, green: 0.47, blue: 0.34) // Claude のオレンジ ~#D97757
    var rays: Int = 11

    var body: some View {
        Canvas { context, size in
            let center = CGPoint(x: size.width / 2, y: size.height / 2)
            let outer = min(size.width, size.height) / 2
            let inner = outer * 0.28
            let halfAngle = (CGFloat.pi / CGFloat(rays)) * 0.92 // 各スパイクの根元の半幅（太め）

            // 少しだけ長さを不揃いにして手描き風の有機的な見た目に。
            let lengthFactors: [CGFloat] = [1.0, 0.82, 0.94, 0.86, 1.0, 0.9, 0.84, 0.96, 0.88, 1.0, 0.9]

            for i in 0 ..< rays {
                let angle = (CGFloat(i) / CGFloat(rays)) * 2 * .pi - .pi / 2
                let factor = lengthFactors[i % lengthFactors.count]
                let length = outer * factor

                let tip = point(center, angle, length)
                let base1 = point(center, angle - halfAngle, inner)
                let base2 = point(center, angle + halfAngle, inner)
                let ctrl1 = point(center, angle - halfAngle * 0.4, length * 0.72)
                let ctrl2 = point(center, angle + halfAngle * 0.4, length * 0.72)

                var path = Path()
                path.move(to: base1)
                path.addQuadCurve(to: tip, control: ctrl1)
                path.addQuadCurve(to: base2, control: ctrl2)
                path.closeSubpath()
                context.fill(path, with: .color(color))
            }

            // 中央を埋めて放射の根元をつなぐ。
            let dot = CGRect(x: center.x - inner, y: center.y - inner, width: inner * 2, height: inner * 2)
            context.fill(Path(ellipseIn: dot), with: .color(color))
        }
    }

    private func point(_ center: CGPoint, _ angle: CGFloat, _ radius: CGFloat) -> CGPoint {
        CGPoint(x: center.x + cos(angle) * radius, y: center.y + sin(angle) * radius)
    }
}
