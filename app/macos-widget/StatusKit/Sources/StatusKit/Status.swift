import Foundation

/// Mirror of `mediaflow_core::status::StatusSnapshot` (format version 1).
/// The JSON contract is pinned by `Fixtures/status-v1.json`, decoded by both
/// the Rust and the Swift test suites.
public struct StatusSnapshot: Codable, Equatable, Sendable {
    public static let supportedVersion = 1

    public var version: Int
    public var updatedAt: Date
    public var current: CurrentImport?
    public var last: LastImport?
    public var attention: Int
    public var destinations: [DestinationStatus]

    enum CodingKeys: String, CodingKey {
        case version, current, last, attention, destinations
        case updatedAt = "updated_at"
    }
}

public struct CurrentImport: Codable, Equatable, Sendable {
    public var sessionId: String
    public var project: String
    public var sourceLabel: String
    public var state: String
    public var fraction: Double
    public var bytesDone: UInt64
    public var bytesTotal: UInt64
    public var etaSeconds: Double?
    public var currentFile: String?

    enum CodingKeys: String, CodingKey {
        case project, state, fraction
        case sessionId = "session_id"
        case sourceLabel = "source_label"
        case bytesDone = "bytes_done"
        case bytesTotal = "bytes_total"
        case etaSeconds = "eta_seconds"
        case currentFile = "current_file"
    }
}

public struct LastImport: Codable, Equatable, Sendable {
    public var sessionId: String
    public var project: String
    public var sourceLabel: String
    public var state: String
    public var verifiedOk: Bool
    public var finishedAt: Date?
    public var filesDone: Int
    public var bytesDone: UInt64
    public var problems: Int

    enum CodingKeys: String, CodingKey {
        case project, state, problems
        case sessionId = "session_id"
        case sourceLabel = "source_label"
        case verifiedOk = "verified_ok"
        case finishedAt = "finished_at"
        case filesDone = "files_done"
        case bytesDone = "bytes_done"
    }
}

public struct DestinationStatus: Codable, Equatable, Sendable, Identifiable {
    public var label: String
    public var available: Bool
    public var freeBytes: UInt64?
    public var id: String { label }

    enum CodingKeys: String, CodingKey {
        case label, available
        case freeBytes = "free_bytes"
    }
}

public enum StatusError: Error, Equatable {
    case unsupportedVersion(Int)
}

extension StatusSnapshot {
    /// Decode, accepting RFC 3339 dates with or without fractional seconds
    /// (Rust's chrono writes nanoseconds).
    public static func decode(_ data: Data) throws -> StatusSnapshot {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .custom { decoder in
            let s = try decoder.singleValueContainer().decode(String.self)
            if let date = Self.parseDate(s) { return date }
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "bad date \(s)"))
        }
        let snap = try d.decode(StatusSnapshot.self, from: data)
        guard snap.version == supportedVersion else { throw StatusError.unsupportedVersion(snap.version) }
        return snap
    }

    static func parseDate(_ s: String) -> Date? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = f.date(from: s) { return d }
        // ISO8601DateFormatter handles at most milliseconds: trim longer fractions.
        if let dot = s.firstIndex(of: "."), let end = s[dot...].firstIndex(where: { $0 == "Z" || $0 == "+" || $0 == "-" }) {
            let frac = s[s.index(after: dot)..<end].prefix(3)
            if let d = f.date(from: String(s[..<dot]) + "." + String(frac) + String(s[end...])) { return d }
        }
        f.formatOptions = [.withInternetDateTime]
        return f.date(from: s)
    }
}
