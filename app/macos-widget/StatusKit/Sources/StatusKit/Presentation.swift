import Foundation

/// What the widget shows, derived from a snapshot. Pure logic, unit-tested.
public struct WidgetState: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case importing(fraction: Double)
        case verified
        case attention
        case idle
        case unavailable
    }

    public var kind: Kind
    public var title: String
    public var subtitle: String
    public var detail: String
    public var symbol: String
    public var stale: Bool

    /// - Parameters:
    ///   - now: current time (injected for tests).
    ///   - staleAfter: a snapshot older than this while "importing" is shown
    ///     as stale, never as live progress (the app may have quit).
    public init(snapshot: StatusSnapshot?, now: Date = Date(), staleAfter: TimeInterval = 120) {
        self = WidgetState.make(snapshot: snapshot, now: now, staleAfter: staleAfter)
    }

    public init(kind: Kind, title: String, subtitle: String, detail: String, symbol: String, stale: Bool) {
        self.kind = kind
        self.title = title
        self.subtitle = subtitle
        self.detail = detail
        self.symbol = symbol
        self.stale = stale
    }

    static func make(snapshot: StatusSnapshot?, now: Date, staleAfter: TimeInterval) -> WidgetState {
        guard let s = snapshot else {
            return WidgetState(
                kind: .unavailable, title: "MediaFlow", subtitle: "Open MediaFlow once",
                detail: "Status appears after the app has run.", symbol: "externaldrive.badge.questionmark", stale: false)
        }
        if let c = s.current {
            let stale = now.timeIntervalSince(s.updatedAt) > staleAfter
            let detail: String
            if stale {
                detail = "Last update \(Format.relative(s.updatedAt, now: now))"
            } else if let eta = c.etaSeconds {
                detail = "about \(Format.duration(eta)) left"
            } else {
                detail = "\(Format.bytes(c.bytesDone)) of \(Format.bytes(c.bytesTotal))"
            }
            return WidgetState(
                kind: .importing(fraction: min(max(c.fraction, 0), 1)),
                title: stale ? "Import paused?" : "Copying \(Int((c.fraction * 100).rounded(.down)))%",
                subtitle: c.project.isEmpty ? c.sourceLabel : c.project,
                detail: detail, symbol: "arrow.down.circle", stale: stale)
        }
        if s.attention > 0 || (s.last.map { !$0.verifiedOk } ?? false) {
            let n = max(s.attention, 1)
            return WidgetState(
                kind: .attention,
                title: n == 1 ? "1 import needs attention" : "\(n) imports need attention",
                subtitle: s.last.map { $0.project.isEmpty ? $0.sourceLabel : $0.project } ?? "",
                detail: "Don't format the card yet", symbol: "exclamationmark.triangle.fill", stale: false)
        }
        if let l = s.last {
            let when = l.finishedAt.map { Format.relative($0, now: now) } ?? ""
            return WidgetState(
                kind: .verified, title: "Verified",
                subtitle: l.project.isEmpty ? l.sourceLabel : l.project,
                detail: "\(l.filesDone) files · \(Format.bytes(l.bytesDone))" + (when.isEmpty ? "" : " · \(when)"),
                symbol: "checkmark.seal.fill", stale: false)
        }
        return WidgetState(kind: .idle, title: "No import yet", subtitle: "Connect a card to start", detail: "", symbol: "sdcard", stale: false)
    }
}

public enum Format {
    public static func bytes(_ n: UInt64) -> String {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f.string(fromByteCount: Int64(clamping: n))
    }

    public static func duration(_ s: Double) -> String {
        let secs = Int(s.rounded())
        if secs < 60 { return "\(secs) s" }
        if secs < 3600 { return "\(secs / 60) min" }
        return "\(secs / 3600) h \(String(format: "%02d", (secs % 3600) / 60))"
    }

    public static func relative(_ date: Date, now: Date) -> String {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f.localizedString(for: date, relativeTo: now)
    }
}

/// Where the app writes the snapshot: the App Group container shared with
/// the widget. Keep in sync with `APP_GROUP` in app/src-tauri/src/native.rs.
public enum StatusLocation {
    public static let appGroup = "group.org.mediaflow.app"
    public static let fileName = "status.json"

    public static func url(fileManager: FileManager = .default) -> URL? {
        fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroup)?.appendingPathComponent(fileName)
    }

    public static func load(fileManager: FileManager = .default) -> StatusSnapshot? {
        guard let url = url(fileManager: fileManager), let data = try? Data(contentsOf: url) else { return nil }
        return try? StatusSnapshot.decode(data)
    }
}
