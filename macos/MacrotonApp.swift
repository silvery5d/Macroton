import AppKit
import Foundation
import SwiftUI

private struct GameEntry: Decodable, Identifiable {
    let appid: UInt32
    let name: String
    let action: String
    let stage: String
    let detail: String
    var id: UInt32 { appid }
}

private struct CommandResult {
    let code: Int32
    let output: String
}

private func runCommand(_ cli: URL, profiles: URL, arguments: [String]) -> CommandResult {
    let process = Process()
    process.executableURL = cli
    process.arguments = arguments
    process.environment = ProcessInfo.processInfo.environment.merging(
        ["MACROTON_PROFILES_DIR": profiles.path]
    ) { _, new in new }
    let pipe = Pipe()
    process.standardOutput = pipe
    process.standardError = pipe
    do {
        try process.run()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return CommandResult(code: process.terminationStatus,
                             output: String(decoding: data, as: UTF8.self))
    } catch {
        return CommandResult(code: 1, output: error.localizedDescription)
    }
}

@MainActor
private final class GameStore: ObservableObject {
    @Published var games: [GameEntry] = []
    @Published var message = "正在读取本机 Steam 游戏…"
    @Published var busy = false
    @Published var installerAppID: UInt32?
    @Published var installerSHA256 = ""

    private let cli = Bundle.main.resourceURL!.appendingPathComponent("bin/macroton")
    private let profiles = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Application Support/Macroton/profiles")

    init() {
        do {
            try FileManager.default.createDirectory(at: profiles, withIntermediateDirectories: true)
            if let bundled = Bundle.main.resourceURL?.appendingPathComponent("compatibility"),
               let files = try? FileManager.default.contentsOfDirectory(at: bundled, includingPropertiesForKeys: nil) {
                for file in files where file.pathExtension == "conf" {
                    let destination = profiles.appendingPathComponent(file.lastPathComponent)
                    if !FileManager.default.fileExists(atPath: destination.path) {
                        try FileManager.default.copyItem(at: file, to: destination)
                    }
                }
            }
        } catch {
            message = "无法准备配置目录：\(error.localizedDescription)"
        }
        refresh()
    }

    func refresh() {
        guard !busy else { return }
        busy = true
        let profilePath = profiles.path
        let cli = cli
        let profiles = profiles
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            let result = runCommand(cli, profiles: profiles, arguments: ["library", "list", "--profiles-dir", profilePath, "--json"])
            let decoded = try? JSONDecoder().decode([GameEntry].self, from: Data(result.output.utf8))
            DispatchQueue.main.async {
                self.busy = false
                if result.code == 0, let decoded {
                    self.games = decoded
                    self.message = decoded.isEmpty ? "本机 Steam 库尚无已安装游戏。" : "显示本机已安装的 Steam 游戏；购买权限仍由 Steam 确认。"
                } else {
                    self.message = "读取 Steam 库失败：\(result.output)"
                }
            }
        }
    }

    func prepare(_ game: GameEntry) {
        if game.stage == "configuration-invalid" {
            message = "配置需要修正：\(game.detail)"
            return
        }
        let profile = profiles.appendingPathComponent("\(game.appid).conf")
        if !FileManager.default.fileExists(atPath: profile.path) {
            let panel = NSOpenPanel()
            panel.allowedContentTypes = [.plainText]
            panel.canChooseDirectories = false
            panel.message = "选择贡献者提供的 \(game.appid).conf 配置"
            guard panel.runModal() == .OK, let source = panel.url else { return }
            let validation = runCommand(cli, profiles: profiles, arguments: ["profile", "validate", source.path])
            guard validation.code == 0 else {
                message = "配置校验失败：\(validation.output)"
                return
            }
            do {
                try FileManager.default.copyItem(at: source, to: profile)
            } catch {
                message = "无法导入配置：\(error.localizedDescription)"
                return
            }
        }
        if game.stage == "steam-setup" {
            installerAppID = game.appid
            return
        }
        execute(game.appid, command: "prepare")
    }

    func runGame(_ game: GameEntry) {
        execute(game.appid, command: "play")
    }

    func setupSteam() {
        guard let appID = installerAppID else { return }
        let sha = installerSHA256.trimmingCharacters(in: .whitespacesAndNewlines)
        guard sha.count == 64, sha.allSatisfy({ $0.isHexDigit }) else {
            message = "请输入独立核对过的 Valve 安装程序 SHA-256。"
            return
        }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.message = "选择从 Valve 获取的 SteamSetup.exe"
        guard panel.runModal() == .OK, let installer = panel.url else { return }
        installerAppID = nil
        installerSHA256 = ""
        execute(appID, command: "prepare", extra: ["--installer", installer.path, "--installer-sha256", sha])
    }

    private func execute(_ appID: UInt32, command: String, extra: [String] = []) {
        guard !busy else { return }
        busy = true
        message = "\(command == "play" ? "正在启动" : "正在准备") AppID \(appID)…"
        let profile = profiles.appendingPathComponent("\(appID).conf").path
        let cli = cli
        let profiles = profiles
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            let result = runCommand(cli, profiles: profiles, arguments: [command, String(appID), "--profile", profile] + extra)
            DispatchQueue.main.async {
                self.busy = false
                self.message = result.output.isEmpty ? "命令结束，状态 \(result.code)" : result.output
                self.refresh()
            }
        }
    }
}

private struct ContentView: View {
    @StateObject private var store = GameStore()

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Macroton").font(.largeTitle.bold())
                Spacer()
                Button("刷新") { store.refresh() }.disabled(store.busy)
            }
            Text("本机 Steam 游戏").font(.headline)
            List(store.games) { game in
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(game.name).font(.headline)
                        Text("AppID \(game.appid) · \(game.detail)")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button(game.action == "run" ? "运行" : "准备") {
                        if game.action == "run" { store.runGame(game) }
                        else { store.prepare(game) }
                    }.disabled(store.busy)
                }
            }
            Text("状态").font(.headline)
            ScrollView {
                Text(store.message).frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled).font(.system(.caption, design: .monospaced))
            }.frame(height: 110)
        }
        .padding(20)
        .frame(minWidth: 640, minHeight: 430)
        .sheet(item: Binding(
            get: { store.installerAppID.map { InstallerSelection(id: $0) } },
            set: { store.installerAppID = $0?.id }
        )) { _ in
            VStack(alignment: .leading, spacing: 12) {
                Text("准备 Windows Steam").font(.title2.bold())
                Text("先从 Valve 获取 SteamSetup.exe，并独立核对其 SHA-256。Macroton 会验证文件后建立独立 Steam 环境。")
                TextField("已核对的 SHA-256", text: $store.installerSHA256)
                HStack {
                    Button("取消") { store.installerAppID = nil }
                    Button("选择安装程序并继续") { store.setupSteam() }
                }
            }.padding(24).frame(width: 500)
        }
    }
}

private struct InstallerSelection: Identifiable {
    let id: UInt32
}

@main
private struct MacrotonApp: App {
    var body: some Scene {
        WindowGroup { ContentView() }
    }
}
