import AppKit
import Foundation
import SwiftUI

private struct GameEntry: Decodable, Identifiable {
    let appid: UInt32
    let name: String
    let source: String
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
    @Published var filter: LibraryFilter = .all
    @Published var selectedAppID: UInt32?
    @Published var search = ""

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

private enum LibraryFilter: String, CaseIterable, Identifiable {
    case all = "全部游戏"
    case ready = "可以运行"
    case prepare = "需要准备"

    var id: String { rawValue }
    var symbol: String {
        switch self {
        case .all: return "square.grid.2x2"
        case .ready: return "play.rectangle"
        case .prepare: return "arrow.down.to.line"
        }
    }
}

private enum Palette {
    static let deepest = Color(red: 28 / 255, green: 28 / 255, blue: 30 / 255)
    static let middle = Color(red: 44 / 255, green: 44 / 255, blue: 46 / 255)
    static let surface = Color(red: 58 / 255, green: 58 / 255, blue: 60 / 255)
    static let line = Color.white.opacity(0.10)
    static let muted = Color.white.opacity(0.68)
    static let faint = Color.white.opacity(0.42)
    static let accent = Color(red: 10 / 255, green: 132 / 255, blue: 1)
}

private struct VibrantSidebar: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .sidebar
        view.blendingMode = .withinWindow
        view.state = .active
        return view
    }
    func updateNSView(_ view: NSVisualEffectView, context: Context) {}
}

private struct ContentView: View {
    @StateObject private var store = GameStore()

    private var visibleGames: [GameEntry] {
        store.games.filter { game in
            let matchesFilter: Bool
            switch store.filter {
            case .all: matchesFilter = true
            case .ready: matchesFilter = game.action == "run"
            case .prepare: matchesFilter = game.action != "run"
            }
            return matchesFilter && (store.search.isEmpty || game.name.localizedCaseInsensitiveContains(store.search)
                                     || String(game.appid).contains(store.search))
        }
    }

    private var selectedGame: GameEntry? {
        visibleGames.first { $0.appid == store.selectedAppID } ?? visibleGames.first
    }

    var body: some View {
        HStack(spacing: 0) {
            sidebar.frame(width: 208)
            Rectangle().fill(Palette.line).frame(width: 1)
            library.frame(minWidth: 300, idealWidth: 350)
            Rectangle().fill(Palette.line).frame(width: 1)
            detail.frame(minWidth: 350, idealWidth: 440)
        }
        .background(Palette.deepest)
        .preferredColorScheme(.dark)
        .frame(minWidth: 900, minHeight: 580)
        .sheet(item: Binding(
            get: { store.installerAppID.map { InstallerSelection(id: $0) } },
            set: { store.installerAppID = $0?.id }
        )) { _ in installerSheet }
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "square.stack.3d.up.fill")
                    .font(.system(size: 17)).foregroundStyle(Palette.accent)
                    .frame(width: 34, height: 34)
                    .background(Palette.surface, in: RoundedRectangle(cornerRadius: 9))
                VStack(alignment: .leading, spacing: 2) {
                    Text("Macroton").font(.system(size: 20, weight: .semibold, design: .serif))
                    Text("STEAM LIBRARY").font(.system(size: 9, weight: .medium, design: .monospaced))
                        .tracking(1.4).foregroundStyle(Palette.faint)
                }
            }
            .padding(.horizontal, 18).padding(.top, 28).padding(.bottom, 36)

            Text("游戏库").font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Palette.faint).padding(.horizontal, 20).padding(.bottom, 10)
            ForEach(LibraryFilter.allCases) { item in sidebarItem(item) }
            Spacer()
            VStack(alignment: .leading, spacing: 6) {
                Text("本机游戏").font(.system(size: 11, weight: .medium))
                Text("仅显示已安装条目。购买权限由 Steam 确认。")
                    .font(.system(size: 11)).foregroundStyle(Palette.faint)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .foregroundStyle(Palette.muted)
            .padding(16).frame(maxWidth: .infinity, alignment: .leading)
            .background(Palette.deepest.opacity(0.55))
        }
        .background(VibrantSidebar().overlay(Palette.deepest.opacity(0.80)))
    }

    private func sidebarItem(_ item: LibraryFilter) -> some View {
        let selected = store.filter == item
        return Button {
            store.filter = item
            store.selectedAppID = nil
        } label: {
            HStack(spacing: 11) {
                Image(systemName: item.symbol).frame(width: 17)
                Text(item.rawValue)
                Spacer(minLength: 4)
                Text("\(count(for: item))").font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(Palette.faint)
            }
            .font(.system(size: 13))
            .foregroundStyle(selected ? Color.white.opacity(0.95) : Palette.muted)
            .padding(.horizontal, 12).frame(height: 37)
            .background(selected ? Color.white.opacity(0.10) : Color.clear,
                        in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain).padding(.horizontal, 10)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func count(for item: LibraryFilter) -> Int {
        switch item {
        case .all: return store.games.count
        case .ready: return store.games.filter { $0.action == "run" }.count
        case .prepare: return store.games.filter { $0.action != "run" }.count
        }
    }

    private var library: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                Text(store.filter.rawValue).font(.system(size: 27, weight: .semibold, design: .serif))
                Spacer()
                Button { store.refresh() } label: {
                    Image(systemName: "arrow.clockwise").font(.system(size: 13, weight: .semibold))
                        .frame(width: 30, height: 30)
                }
                .buttonStyle(.plain).background(Palette.surface, in: RoundedRectangle(cornerRadius: 8))
                .disabled(store.busy).help("刷新游戏库")
            }
            .padding(.horizontal, 22).padding(.top, 27).padding(.bottom, 16)
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(Palette.faint)
                TextField("搜索名称或 AppID", text: $store.search).textFieldStyle(.plain)
            }
            .font(.system(size: 12)).padding(.horizontal, 11).frame(height: 34)
            .background(Palette.deepest, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Palette.line, lineWidth: 1))
            .padding(.horizontal, 22).padding(.bottom, 18)
            Rectangle().fill(Palette.line).frame(height: 1)
            if visibleGames.isEmpty {
                VStack(spacing: 11) {
                    Image(systemName: store.busy ? "arrow.clockwise" : "shippingbox")
                        .font(.system(size: 27)).foregroundStyle(Palette.faint)
                    Text(store.busy ? "正在读取游戏库" : store.search.isEmpty ? "这里还没有游戏" : "没有匹配的游戏")
                        .font(.system(size: 14, weight: .medium))
                    Text(store.busy ? "正在检查本机 Steam 安装" : "可刷新列表或切换左侧分类。")
                        .font(.system(size: 12)).foregroundStyle(Palette.faint)
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 5) {
                        ForEach(visibleGames) { game in
                            Button { store.selectedAppID = game.appid } label: { gameRow(game) }
                                .buttonStyle(.plain)
                        }
                    }.padding(10)
                }
            }
        }
        .background(Palette.middle)
    }

    private func gameRow(_ game: GameEntry) -> some View {
        let selected = selectedGame?.appid == game.appid
        return HStack(spacing: 12) {
            Image(systemName: game.action == "run" ? "gamecontroller.fill" : "gamecontroller")
                .font(.system(size: 18)).foregroundStyle(game.action == "run" ? Palette.accent : Palette.muted)
                .frame(width: 42, height: 42).background(Palette.surface,
                    in: RoundedRectangle(cornerRadius: 9))
            VStack(alignment: .leading, spacing: 5) {
                Text(game.name).font(.system(size: 13, weight: .medium))
                    .lineLimit(1).foregroundStyle(Color.white.opacity(0.95))
                Text("AppID \(game.appid)  ·  \(game.action == "run" ? "可以运行" : "需要准备")")
                    .font(.system(size: 11)).foregroundStyle(Palette.faint)
            }
            Spacer(minLength: 0)
            Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold))
                .foregroundStyle(Palette.faint)
        }
        .padding(.horizontal, 10).frame(height: 62)
        .background(selected ? Color.white.opacity(0.10) : .clear,
                    in: RoundedRectangle(cornerRadius: 10))
        .contentShape(Rectangle())
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var detail: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let game = selectedGame {
                gameDetail(game)
            } else {
                VStack(alignment: .leading, spacing: 12) {
                    Text("游戏详情").font(.system(size: 27, weight: .semibold, design: .serif))
                    Text("从游戏库中选择一款游戏，查看状态和下一步。")
                        .foregroundStyle(Palette.muted)
                    Spacer()
                }.padding(28)
            }
            Rectangle().fill(Palette.line).frame(height: 1)
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("活动状态").font(.system(size: 12, weight: .semibold))
                    Spacer()
                    if store.busy { ProgressView().controlSize(.small) }
                }
                ScrollView {
                    Text(store.message).font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(Palette.muted).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }.padding(20).frame(height: 145).background(Palette.deepest)
        }
        .background(Palette.middle)
    }

    private func gameDetail(_ game: GameEntry) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("游戏详情").font(.system(size: 11, weight: .medium))
                    .foregroundStyle(Palette.faint).tracking(1)
                Spacer()
                Text("APPID \(game.appid)")
                    .font(.system(size: 10, design: .monospaced)).foregroundStyle(Palette.faint)
            }
            .padding(.bottom, 22)
            Image(systemName: "gamecontroller.fill")
                .font(.system(size: 32)).foregroundStyle(Palette.muted)
                .frame(width: 74, height: 74)
                .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12))
                .padding(.bottom, 18)
            Text(game.name).font(.system(size: 29, weight: .semibold, design: .serif))
                .foregroundStyle(Color.white.opacity(0.95)).lineLimit(2)
                .truncationMode(.tail).help(game.name)
            Text(game.source == "both" ? "macOS 与 Windows Steam" : game.source == "windows" ? "Windows Steam" : "macOS Steam")
                .font(.system(size: 12)).foregroundStyle(Palette.muted).padding(.top, 5)
            Rectangle().fill(Palette.line).frame(height: 1).padding(.vertical, 24)
            Text("当前状态").font(.system(size: 11, weight: .medium)).foregroundStyle(Palette.faint)
                .padding(.bottom, 8)
            HStack(spacing: 7) {
                Image(systemName: game.action == "run" ? "checkmark.circle.fill" : "circle.dotted")
                    .foregroundStyle(game.action == "run" ? Color.green : Palette.muted)
                Text(game.action == "run" ? "可以尝试运行" : "需要准备")
                    .font(.system(size: 13, weight: .medium))
            }
            Text(game.detail).font(.system(size: 12)).foregroundStyle(Palette.muted)
                .fixedSize(horizontal: false, vertical: true).padding(.top, 9)
            Spacer(minLength: 20)
            Button {
                if game.action == "run" { store.runGame(game) }
                else { store.prepare(game) }
            } label: {
                HStack(spacing: 9) {
                    Image(systemName: game.action == "run" ? "play.fill" : "arrow.down.to.line")
                    Text(game.action == "run" ? "运行游戏" : "准备游戏")
                }
                .font(.system(size: 13, weight: .semibold))
                .frame(maxWidth: .infinity).frame(height: 38)
                .background(Palette.surface, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(Palette.line, lineWidth: 1))
            }.buttonStyle(.plain).disabled(store.busy)
            Text("游戏启动与购买权限由 Steam 确认。")
                .font(.system(size: 10)).foregroundStyle(Palette.faint)
                .frame(maxWidth: .infinity).padding(.top, 12)
        }
        .padding(28).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var installerSheet: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("准备 Windows Steam").font(.system(size: 23, weight: .semibold, design: .serif))
            Text("先从 Valve 获取 SteamSetup.exe，并独立核对其 SHA-256。Macroton 会验证文件后建立独立 Steam 环境。")
                .font(.system(size: 12)).foregroundStyle(Palette.muted)
            TextField("已核对的 SHA-256", text: $store.installerSHA256)
                .textFieldStyle(.plain).font(.system(size: 12, design: .monospaced))
                .padding(10).background(Palette.deepest, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(Palette.line, lineWidth: 1))
            HStack {
                Spacer()
                Button("取消") { store.installerAppID = nil }
                Button("选择安装程序并继续") { store.setupSteam() }
            }
        }
        .padding(24).frame(width: 500).background(Palette.middle).preferredColorScheme(.dark)
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
