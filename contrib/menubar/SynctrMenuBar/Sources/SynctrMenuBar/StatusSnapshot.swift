import Foundation

/// `synctr status --json` contract (`synctr-engine` `StatusSnapshot`).
/// Field names match the CLI JSON. Do not invent a second schema.
struct StatusSnapshot: Decodable, Equatable {
    var rclone: RcloneStatus
    var profiles: [ProfileStatus]
}

struct RcloneStatus: Decodable, Equatable {
    var found: Bool
    var path: String?
    var source: String?
    var detail: String?

    enum CodingKeys: String, CodingKey {
        case found = "found"
        case path = "path"
        case source = "source"
        case detail = "detail"
    }
}

struct LastRun: Decodable, Equatable {
    var finishedAtUnix: Int64
    var finishedAt: String
    var exitCode: Int
    var ok: Bool

    enum CodingKeys: String, CodingKey {
        case finishedAtUnix = "finished_at_unix"
        case finishedAt = "finished_at"
        case exitCode = "exit_code"
        case ok = "ok"
    }
}

struct TransferProgress: Decodable, Equatable {
    var bytes: UInt64
    var totalBytes: UInt64
    var percent: UInt8?
    var speedBps: UInt64?
    var etaSecs: UInt64?
    var transfers: UInt64
    var totalTransfers: UInt64
    var file: String?
    var pid: UInt32?
    var dryRun: Bool
    var updatedAtUnix: Int64
    var updatedAt: String

    enum CodingKeys: String, CodingKey {
        case bytes = "bytes"
        case totalBytes = "total_bytes"
        case percent = "percent"
        case speedBps = "speed_bps"
        case etaSecs = "eta_secs"
        case transfers = "transfers"
        case totalTransfers = "total_transfers"
        case file = "file"
        case pid = "pid"
        case dryRun = "dry_run"
        case updatedAtUnix = "updated_at_unix"
        case updatedAt = "updated_at"
    }
}

struct ProfileStatus: Decodable, Equatable, Identifiable {
    var name: String
    var local: String
    var remote: String
    var mode: String
    var rclone: String?
    var extraFlags: [String]
    var extraIgnore: [String]
    var lastRun: LastRun?
    var progress: TransferProgress?
    /// Omitted from JSON when true (the default).
    var enabled: Bool

    var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name = "name"
        case local = "local"
        case remote = "remote"
        case mode = "mode"
        case rclone = "rclone"
        case extraFlags = "extra_flags"
        case extraIgnore = "extra_ignore"
        case lastRun = "last_run"
        case progress = "progress"
        case enabled = "enabled"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decode(String.self, forKey: .name)
        local = try c.decode(String.self, forKey: .local)
        remote = try c.decode(String.self, forKey: .remote)
        mode = try c.decode(String.self, forKey: .mode)
        rclone = try c.decodeIfPresent(String.self, forKey: .rclone)
        extraFlags = try c.decodeIfPresent([String].self, forKey: .extraFlags) ?? []
        extraIgnore = try c.decodeIfPresent([String].self, forKey: .extraIgnore) ?? []
        lastRun = try c.decodeIfPresent(LastRun.self, forKey: .lastRun)
        progress = try c.decodeIfPresent(TransferProgress.self, forKey: .progress)
        enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
    }
}

enum StatusJSON {
    static func decode(_ data: Data) throws -> StatusSnapshot {
        try JSONDecoder().decode(StatusSnapshot.self, from: data)
    }
}

enum LastRunLabel {
    static func short(_ run: LastRun?, nowUnix: Int64) -> String {
        guard let run else { return "never" }
        let age = ageLabel(unix: run.finishedAtUnix, nowUnix: nowUnix)
        if run.ok {
            return "ok \(age)"
        }
        return "exit \(run.exitCode) \(age)"
    }

    /// Same buckets as `synctr-engine` `age_label_at`.
    static func ageLabel(unix: Int64, nowUnix: Int64) -> String {
        let d = max(0, nowUnix - unix)
        if d < 60 {
            return "\(d)s ago"
        }
        if d < 3600 {
            return "\(d / 60)m ago"
        }
        if d < 86_400 {
            return "\(d / 3600)h ago"
        }
        return "\(d / 86_400)d ago"
    }
}
