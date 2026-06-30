import Foundation

/// `labolabo --hook <socket>` モードの本体。Claude hook の stdin(JSON) を読み、
/// AF_UNIX ソケットへ 1 接続で送って即 exit する。Claude を待たせないよう速やかに終了。
enum HookForwarder {
    static func forward(socketPath: String) {
        let input = FileHandle.standardInput.readDataToEndOfFile()

        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { exit(0) }
        defer { close(fd) }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: addr.sun_path) - 1
        let bytes = Array(socketPath.utf8)
        guard bytes.count <= capacity else { exit(0) }
        withUnsafeMutableBytes(of: &addr.sun_path) { $0.copyBytes(from: bytes) }

        let size = socklen_t(MemoryLayout<sockaddr_un>.size)
        let connected = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, size) }
        }
        guard connected == 0 else { exit(0) }

        input.withUnsafeBytes { raw in
            if let base = raw.baseAddress, raw.count > 0 {
                _ = write(fd, base, raw.count)
            }
        }
        exit(0)
    }
}
