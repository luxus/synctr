import Darwin
import Foundation

/// Thin wrapper: locate `synctr` and run `status --json` / `sync <name>`.
/// Never talks to rclone itself.
struct SynctrCLI {
    var binaryURL: URL
    var configDir: String?

    enum CLIError: LocalizedError {
        case missingBinary
        case statusFailed(status: Int32, stderr: String)
        case spawnFailed(Int32)
        case invalidJSON(Error)

        var errorDescription: String? {
            switch self {
            case .missingBinary:
                return "synctr not found (PATH, SYNCTR_BIN, or Choose synctr…)"
            case let .statusFailed(status, stderr):
                let tail = stderr.trimmingCharacters(in: .whitespacesAndNewlines)
                if tail.isEmpty {
                    return "synctr status --json exited \(status)"
                }
                return "synctr status --json exited \(status): \(tail)"
            case let .spawnFailed(code):
                return "could not start synctr (posix_spawn \(code))"
            case let .invalidJSON(error):
                return "synctr status --json: \(error.localizedDescription)"
            }
        }
    }

    static func resolveBinary(override: String? = nil) -> URL? {
        if let override, !override.isEmpty {
            let url = URL(fileURLWithPath: override)
            if FileManager.default.isExecutableFile(atPath: url.path) {
                return url
            }
        }
        if let env = ProcessInfo.processInfo.environment["SYNCTR_BIN"], !env.isEmpty {
            if FileManager.default.isExecutableFile(atPath: env) {
                return URL(fileURLWithPath: env)
            }
        }
        let path = augmentedPATH()
        for dir in path.split(separator: ":") {
            let candidate = URL(fileURLWithPath: String(dir)).appendingPathComponent("synctr")
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                return candidate
            }
        }
        return nil
    }

    /// GUI apps often have a short PATH. Match synctr's well-known dirs, plus cargo.
    static func augmentedPATH() -> String {
        let env = ProcessInfo.processInfo.environment
        let user = env["USER"] ?? env["LOGNAME"] ?? NSUserName()
        let home = env["HOME"] ?? NSHomeDirectory()
        var dirs: [String] = []
        if let existing = env["PATH"] {
            dirs.append(contentsOf: existing.split(separator: ":").map(String.init))
        }
        dirs.append(contentsOf: [
            "\(home)/.cargo/bin",
            "\(home)/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/etc/profiles/per-user/\(user)/bin",
            "/run/current-system/sw/bin",
        ])
        var seen = Set<String>()
        return dirs.filter { seen.insert($0).inserted }.joined(separator: ":")
    }

    func status() throws -> StatusSnapshot {
        let proc = Process()
        proc.executableURL = binaryURL
        proc.arguments = prefixArgs() + ["status", "--json"]
        proc.environment = childEnvironment()
        let out = Pipe()
        let err = Pipe()
        proc.standardOutput = out
        proc.standardError = err
        proc.standardInput = FileHandle.nullDevice
        try proc.run()
        proc.waitUntilExit()
        let stdout = out.fileHandleForReading.readDataToEndOfFile()
        let stderr = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        if proc.terminationStatus != 0 {
            throw CLIError.statusFailed(status: proc.terminationStatus, stderr: stderr)
        }
        do {
            return try StatusJSON.decode(stdout)
        } catch {
            throw CLIError.invalidJSON(error)
        }
    }

    /// Start `synctr sync <name>` in its own process group so Stop can SIGTERM rclone too.
    func startSync(name: String) throws -> pid_t {
        let args = prefixArgs() + ["sync", name]
        return try spawnOwnGroup(
            path: binaryURL,
            arguments: args,
            environment: childEnvironment()
        )
    }

    func prefixArgs() -> [String] {
        guard let configDir, !configDir.isEmpty else { return [] }
        return ["--config-dir", configDir]
    }

    func childEnvironment() -> [String: String] {
        var env = ProcessInfo.processInfo.environment
        env["PATH"] = Self.augmentedPATH()
        return env
    }
}

enum ProcessGroup {
    static func spawnOwnGroup(
        path: URL,
        arguments: [String],
        environment: [String: String]
    ) throws -> pid_t {
        var pid: pid_t = 0
        var attr: posix_spawnattr_t?
        posix_spawnattr_init(&attr)
        defer { posix_spawnattr_destroy(&attr) }
        posix_spawnattr_setflags(&attr, Int16(POSIX_SPAWN_SETPGROUP))
        posix_spawnattr_setpgroup(&attr, 0)

        var fileActions: posix_spawn_file_actions_t?
        posix_spawn_file_actions_init(&fileActions)
        defer { posix_spawn_file_actions_destroy(&fileActions) }
        posix_spawn_file_actions_addopen(&fileActions, 0, "/dev/null", O_RDONLY, 0)
        posix_spawn_file_actions_addopen(&fileActions, 1, "/dev/null", O_WRONLY, 0)
        posix_spawn_file_actions_addopen(&fileActions, 2, "/dev/null", O_WRONLY, 0)

        var argv = ([path.path] + arguments).map { strdup($0) }
        argv.append(nil)
        var envp = environment.map { strdup("\($0.key)=\($0.value)") }
        envp.append(nil)
        defer {
            for ptr in argv {
                if let ptr {
                    free(ptr)
                }
            }
            for ptr in envp {
                if let ptr {
                    free(ptr)
                }
            }
        }

        let rc = argv.withUnsafeMutableBufferPointer { argvBuf in
            envp.withUnsafeMutableBufferPointer { envBuf in
                posix_spawn(
                    &pid,
                    path.path,
                    &fileActions,
                    &attr,
                    argvBuf.baseAddress,
                    envBuf.baseAddress
                )
            }
        }
        if rc != 0 {
            throw SynctrCLI.CLIError.spawnFailed(rc)
        }
        return pid
    }

    static func terminate(pid: pid_t) {
        if pid <= 0 { return }
        _ = killpg(pid, SIGTERM)
        _ = kill(pid, SIGTERM)
    }

    static func terminatePID(_ pid: UInt32) {
        if pid == 0 { return }
        _ = kill(pid_t(pid), SIGTERM)
    }

    static func waitNonblocking(pid: pid_t) -> Bool {
        if pid <= 0 { return true }
        var status: Int32 = 0
        let rc = waitpid(pid, &status, WNOHANG)
        return rc == pid || (rc < 0 && errno == ECHILD)
    }
}

func spawnOwnGroup(
    path: URL,
    arguments: [String],
    environment: [String: String]
) throws -> pid_t {
    try ProcessGroup.spawnOwnGroup(path: path, arguments: arguments, environment: environment)
}
