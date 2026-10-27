import XCTest
@testable import StatusKit

final class StatusKitTests: XCTestCase {
    /// The fixture shared with the Rust tests (crates/mediaflow-core/src/status.rs).
    func fixture() throws -> Data {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // StatusKitTests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // StatusKit
            .deletingLastPathComponent() // macos-widget
            .appendingPathComponent("Fixtures/status-v1.json")
        return try Data(contentsOf: url)
    }

    func testDecodesSharedFixture() throws {
        let s = try StatusSnapshot.decode(fixture())
        XCTAssertEqual(s.version, 1)
        XCTAssertEqual(s.current?.state, "copying")
        XCTAssertEqual(s.current?.fraction, 0.42)
        XCTAssertEqual(s.last?.verifiedOk, true)
        XCTAssertEqual(s.attention, 1)
        XCTAssertEqual(s.destinations.count, 2)
        XCTAssertNil(s.destinations[1].freeBytes)
    }

    func testRustNanosecondDates() throws {
        let json = #"{"version":1,"updated_at":"2026-10-27T10:38:41.435223882Z","current":null,"last":null,"attention":0,"destinations":[]}"#
        let s = try StatusSnapshot.decode(Data(json.utf8))
        XCTAssertEqual(s.updatedAt.timeIntervalSince1970, 1_793_097_521.435, accuracy: 0.01)
    }

    func testRejectsUnknownVersion() {
        let json = #"{"version":2,"updated_at":"2026-10-27T10:00:00Z","current":null,"last":null,"attention":0,"destinations":[]}"#
        XCTAssertThrowsError(try StatusSnapshot.decode(Data(json.utf8))) { e in
            XCTAssertEqual(e as? StatusError, .unsupportedVersion(2))
        }
    }

    func testLiveImport() throws {
        let s = try StatusSnapshot.decode(fixture())
        let w = WidgetState(snapshot: s, now: s.updatedAt.addingTimeInterval(10))
        XCTAssertEqual(w.kind, .importing(fraction: 0.42))
        XCTAssertEqual(w.title, "Copying 42%")
        XCTAssertFalse(w.stale)
    }

    func testStaleImportIsNeverShownAsLive() throws {
        let s = try StatusSnapshot.decode(fixture())
        let w = WidgetState(snapshot: s, now: s.updatedAt.addingTimeInterval(600))
        XCTAssertTrue(w.stale)
        XCTAssertEqual(w.title, "Import paused?")
    }

    func testProblemsAreNeverShownAsVerified() throws {
        var s = try StatusSnapshot.decode(fixture())
        s.current = nil
        s.attention = 0
        s.last?.verifiedOk = false
        let w = WidgetState(snapshot: s, now: s.updatedAt)
        XCTAssertEqual(w.kind, .attention)
        XCTAssertNotEqual(w.title, "Verified")
    }

    func testVerified() throws {
        var s = try StatusSnapshot.decode(fixture())
        s.current = nil
        s.attention = 0
        let w = WidgetState(snapshot: s, now: s.updatedAt)
        XCTAssertEqual(w.kind, .verified)
        XCTAssertEqual(w.title, "Verified")
        XCTAssertTrue(w.detail.hasPrefix("812 files"))
    }

    func testNoSnapshot() {
        XCTAssertEqual(WidgetState(snapshot: nil).kind, .unavailable)
    }
}
