import Combine
import Foundation

@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var snapshot: StatusSnapshot?
    @Published private(set) var errorText: String?
    @Published private(set) var binaryPath: String?
    @Published private(set) var startedPIDs: [String: pid_t] = [:]

    private var timer: AnyCancellable?
    private let defaults = UserDefaults.standard

    static let binaryDefaultsKey = "synctrBinary"
    static let configDirDefaultsKey = "synctrConfigDir"

    var configDir: String? {
        let value = defaults.string(forKey: Self.configDirDefaultsKey)
        if let value, !value.isEmpty { return value }
        return nil
    }

    var pollInterval: TimeInterval {
        if snapshot?.profiles.contains(where: { $0.progress != nil }) == true {
            return 2
        }
        if !startedPIDs.isEmpty {
            return 2
        }
        return 10
    }

    var menuSymbolName: String {
        if binaryPath == nil {
            return "questionmark.circle"
        }
        guard let snapshot else {
            return errorText == nil ? "icloud" : "exclamationmark.triangle"
        }
        if !snapshot.rclone.found {
            return "icloud.slash"
        }
        if snapshot.profiles.contains(where: { $0.progress != nil }) {
            return "arrow.triangle.2.circlepath"
        }
        if snapshot.profiles.contains(where: { $0.lastRun?.ok == false }) {
            return "exclamationmark.icloud"
        }
        if snapshot.profiles.isEmpty {
            return "icloud"
        }
        if snapshot.profiles.allSatisfy({ $0.lastRun?.ok == true }) {
            return "checkmark.icloud"
        }
        return "icloud"
    }

    var rcloneLine: String {
        guard let snapshot else {
            return errorText ?? "status unknown"
        }
        if snapshot.rclone.found {
            if let path = snapshot.rclone.path {
                return "rclone found (\(path))"
            }
            return "rclone found"
        }
        return "rclone missing"
    }

    init() {
        binaryPath = SynctrCLI.resolveBinary(
            override: defaults.string(forKey: Self.binaryDefaultsKey)
        )?.path
        startTimer()
        refresh()
    }

    func setBinary(path: String) {
        defaults.set(path, forKey: Self.binaryDefaultsKey)
        binaryPath = SynctrCLI.resolveBinary(override: path)?.path
        refresh()
    }

    func refresh() {
        reapStarted()
        guard let binary = SynctrCLI.resolveBinary(
            override: defaults.string(forKey: Self.binaryDefaultsKey)
        ) else {
            binaryPath = nil
            snapshot = nil
            errorText = SynctrCLI.CLIError.missingBinary.errorDescription
            restartTimer()
            return
        }
        binaryPath = binary.path
        let cli = SynctrCLI(binaryURL: binary, configDir: configDir)
        Task.detached { [weak self] in
            do {
                let snap = try cli.status()
                await MainActor.run {
                    self?.snapshot = snap
                    self?.errorText = nil
                    self?.reapStarted()
                    self?.restartTimer()
                }
            } catch {
                await MainActor.run {
                    self?.errorText = error.localizedDescription
                    self?.restartTimer()
                }
            }
        }
    }

    func start(name: String) {
        guard let binary = SynctrCLI.resolveBinary(
            override: defaults.string(forKey: Self.binaryDefaultsKey)
        ) else {
            errorText = SynctrCLI.CLIError.missingBinary.errorDescription
            return
        }
        if let existing = startedPIDs[name], !ProcessGroup.waitNonblocking(pid: existing) {
            return
        }
        let cli = SynctrCLI(binaryURL: binary, configDir: configDir)
        do {
            let pid = try cli.startSync(name: name)
            startedPIDs[name] = pid
            errorText = nil
        } catch {
            errorText = error.localizedDescription
        }
        refresh()
    }

    func stop(name: String) {
        if let pid = startedPIDs[name] {
            ProcessGroup.terminate(pid: pid)
        }
        if let rclonePID = snapshot?.profiles.first(where: { $0.name == name })?.progress?.pid {
            ProcessGroup.terminatePID(rclonePID)
        }
        refresh()
    }

    func isStarted(_ name: String) -> Bool {
        guard let pid = startedPIDs[name] else { return false }
        return !ProcessGroup.waitNonblocking(pid: pid)
    }

    func canStart(_ profile: ProfileStatus) -> Bool {
        profile.enabled && profile.progress == nil && !isStarted(profile.name)
    }

    func canStop(_ profile: ProfileStatus) -> Bool {
        profile.progress != nil || isStarted(profile.name)
    }

    private func reapStarted() {
        startedPIDs = startedPIDs.filter { _, pid in
            !ProcessGroup.waitNonblocking(pid: pid)
        }
    }

    private func startTimer() {
        restartTimer()
    }

    private func restartTimer() {
        timer?.cancel()
        timer = Timer.publish(every: pollInterval, on: .main, in: .common)
            .autoconnect()
            .sink { [weak self] _ in
                self?.refresh()
            }
    }
}
