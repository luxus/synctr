import AppKit
import SwiftUI

struct MenuBarView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Group {
            Text(model.rcloneLine)
            if let path = model.binaryPath {
                Text(path).font(.caption)
            }
            if let error = model.errorText {
                Text(error)
            }
        }
        Divider()
        profilesBlock
        Divider()
        Button("Refresh") { model.refresh() }
        Button("Choose synctr…") {
            DispatchQueue.main.async {
                chooseBinary()
            }
        }
        Divider()
        Button("Quit") {
            NSApplication.shared.terminate(nil)
        }
    }

    @ViewBuilder
    private var profilesBlock: some View {
        if model.binaryPath == nil {
            Text("Install synctr and put it on PATH, or Choose synctr…")
        } else if let profiles = model.snapshot?.profiles {
            if profiles.isEmpty {
                Text("No profiles")
            } else {
                ForEach(profiles) { profile in
                    Menu(profileTitle(profile)) {
                        if model.canStop(profile) {
                            Button("Stop") { model.stop(name: profile.name) }
                        } else if model.canStart(profile) {
                            Button("Sync") { model.start(name: profile.name) }
                        } else {
                            Text("Disabled")
                        }
                        Divider()
                        Text("\(profile.mode)  \(profile.local) → \(profile.remote)")
                        Text(LastRunLabel.short(profile.lastRun, nowUnix: Int64(Date().timeIntervalSince1970)))
                        if let progress = profile.progress {
                            Text(progressLine(progress))
                        }
                    }
                }
            }
        } else {
            Text("Waiting for synctr status --json")
        }
    }

    private func profileTitle(_ profile: ProfileStatus) -> String {
        let mark: String
        if profile.progress != nil || model.isStarted(profile.name) {
            if let pct = profile.progress?.percent {
                mark = "\(pct)%"
            } else {
                mark = "running"
            }
        } else if !profile.enabled {
            mark = "disabled"
        } else if let ok = profile.lastRun?.ok {
            mark = ok ? "ok" : "fail"
        } else {
            mark = "never"
        }
        return "\(profile.name)  \(mark)"
    }

    private func progressLine(_ progress: TransferProgress) -> String {
        var parts: [String] = []
        if let pct = progress.percent {
            parts.append("\(pct)%")
        }
        if progress.totalBytes > 0 {
            parts.append("\(progress.bytes)/\(progress.totalBytes) B")
        }
        if let file = progress.file, !file.isEmpty {
            parts.append(file)
        }
        if progress.dryRun {
            parts.append("dry-run")
        }
        if parts.isEmpty {
            return "transferring"
        }
        return parts.joined(separator: "  ")
    }

    private func chooseBinary() {
        let panel = NSOpenPanel()
        panel.title = "Choose synctr"
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.treatsFilePackagesAsDirectories = false
        if panel.runModal() == .OK, let url = panel.url {
            model.setBinary(path: url.path)
        }
    }
}
