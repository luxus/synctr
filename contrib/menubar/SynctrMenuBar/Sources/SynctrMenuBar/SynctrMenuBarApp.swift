import SwiftUI

@main
struct SynctrMenuBarApp: App {
    @StateObject private var model = AppModel()

    var body: some Scene {
        MenuBarExtra {
            MenuBarView(model: model)
        } label: {
            Label("synctr", systemImage: model.menuSymbolName)
        }
        .menuBarExtraStyle(.menu)
    }
}
