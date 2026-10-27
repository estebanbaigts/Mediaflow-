import StatusKit
import SwiftUI
import WidgetKit

// MARK: - Timeline

struct StatusEntry: TimelineEntry {
    let date: Date
    let state: WidgetState
    let destinations: [DestinationStatus]
}

struct StatusProvider: TimelineProvider {
    func placeholder(in context: Context) -> StatusEntry {
        StatusEntry(
            date: .now,
            state: WidgetState(kind: .verified, title: "Verified", subtitle: "Launch film", detail: "812 files · 248 GB", symbol: "checkmark.seal.fill", stale: false),
            destinations: [DestinationStatus(label: "RAID Projects", available: true, freeBytes: 3_200_000_000_000)]
        )
    }

    func getSnapshot(in context: Context, completion: @escaping (StatusEntry) -> Void) {
        completion(context.isPreview ? placeholder(in: context) : entry())
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<StatusEntry>) -> Void) {
        let e = entry()
        // Refresh often while an import runs, rarely otherwise. WidgetKit may
        // delay refreshes; a stale import is labelled as such, never as live.
        let next: TimeInterval
        if case .importing = e.state.kind, !e.state.stale { next = 60 } else { next = 15 * 60 }
        completion(Timeline(entries: [e], policy: .after(.now.addingTimeInterval(next))))
    }

    private func entry() -> StatusEntry {
        let snap = StatusLocation.load()
        return StatusEntry(date: .now, state: WidgetState(snapshot: snap), destinations: snap?.destinations ?? [])
    }
}

// MARK: - Views

private extension WidgetState {
    var tint: Color {
        switch kind {
        case .importing: return stale ? .secondary : .blue
        case .verified: return .green
        case .attention: return .orange
        case .idle, .unavailable: return .secondary
        }
    }

    var fraction: Double? {
        if case let .importing(f) = kind { return f }
        return nil
    }
}

struct StatusBadge: View {
    let state: WidgetState
    var size: CGFloat = 34

    var body: some View {
        ZStack {
            if let f = state.fraction {
                Circle().stroke(state.tint.opacity(0.18), lineWidth: 4)
                Circle()
                    .trim(from: 0, to: f)
                    .stroke(state.tint, style: StrokeStyle(lineWidth: 4, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                Image(systemName: "arrow.down")
                    .font(.system(size: size * 0.38, weight: .bold))
                    .foregroundStyle(state.tint)
            } else {
                Image(systemName: state.symbol)
                    .font(.system(size: size * 0.7, weight: .semibold))
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(state.tint)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

struct SmallView: View {
    let entry: StatusEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            StatusBadge(state: entry.state)
            Spacer(minLength: 0)
            Text(entry.state.title)
                .font(.headline)
                .lineLimit(2)
                .minimumScaleFactor(0.8)
            Text(entry.state.subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            if !entry.state.detail.isEmpty {
                Text(entry.state.detail)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .monospacedDigit()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

struct MediumView: View {
    let entry: StatusEntry

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            SmallView(entry: entry)
            Divider()
            VStack(alignment: .leading, spacing: 6) {
                Text("Destinations")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                if entry.destinations.isEmpty {
                    Text("None yet").font(.caption).foregroundStyle(.secondary)
                }
                ForEach(entry.destinations.prefix(3)) { d in
                    HStack(spacing: 6) {
                        Image(systemName: d.available ? "externaldrive.fill" : "externaldrive.badge.xmark")
                            .foregroundStyle(d.available ? Color.secondary : Color.orange)
                            .font(.caption)
                        Text(d.label).font(.caption).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(d.available ? (d.freeBytes.map { Format.bytes($0) } ?? "—") : "offline")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                    }
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct AccessoryView: View {
    let entry: StatusEntry

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: entry.state.symbol)
            VStack(alignment: .leading) {
                Text(entry.state.title).font(.headline).lineLimit(1)
                Text(entry.state.subtitle).font(.caption).lineLimit(1)
            }
        }
    }
}

struct MediaFlowWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: StatusEntry

    var body: some View {
        switch family {
        case .systemMedium, .systemLarge: MediumView(entry: entry)
        case .accessoryRectangular, .accessoryInline: AccessoryView(entry: entry)
        default: SmallView(entry: entry)
        }
    }
}

// MARK: - Widget

@main
struct MediaFlowWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "org.mediaflow.status", provider: StatusProvider()) { entry in
            MediaFlowWidgetView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("MediaFlow")
        .description("Your current import, the last verified import and free space on your destinations.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular])
    }
}
