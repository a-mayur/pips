import AppKit
import CryptoKit

private let releasesURL = URL(string: "https://api.github.com/repos/a-mayur/pips/releases?per_page=20")!
private let autoKey = "checkForUpdates"
private let skipKey = "skippedVersion"
private let assetName = "Pips.zip"
private let sumsName = "SHA256SUMS.txt"

/// Checks GitHub Releases for a newer Pips, shows what changed, and replaces the app in place.
@MainActor
final class Updater: NSObject {

    private var timer: Timer?
    private var download: Task<Void, Never>?

    struct Release: Decodable {
        let tag_name: String
        let body: String?
        let draft: Bool
        let prerelease: Bool
        let assets: [Asset]
        struct Asset: Decodable {
            let name: String
            let browser_download_url: URL
        }
        var version: String { tag_name.hasPrefix("v") ? String(tag_name.dropFirst()) : tag_name }
    }

    enum UpdateError: LocalizedError {
        case server(Int), missingAsset(String), checksum, unzip, notWritable(String)
        var errorDescription: String? {
            switch self {
            case .server(let code): return "GitHub answered with HTTP \(code)."
            case .missingAsset(let version): return "Release \(version) has no \(assetName) or \(sumsName)."
            case .checksum: return "The download does not match its checksum; nothing was changed."
            case .unzip: return "The downloaded app could not be unpacked."
            case .notWritable(let folder): return "Pips cannot replace itself in \(folder). Move Pips to Applications and try again."
            }
        }
    }

    private var current: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
    }

    override init() {
        super.init()
        UserDefaults.standard.register(defaults: [autoKey: true])
        DispatchQueue.main.asyncAfter(deadline: .now() + 10) { [weak self] in
            Task { @MainActor in self?.check(manual: false) }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 24 * 3600, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.check(manual: false) }
        }
    }

    /// Automatic checks stay quiet unless there is an update; manual ones always answer.
    func check(manual: Bool) {
        guard manual || UserDefaults.standard.bool(forKey: autoKey) else { return }
        Task {
            do {
                var request = URLRequest(url: releasesURL)
                request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
                let (data, response) = try await URLSession.shared.data(for: request)
                let status = (response as? HTTPURLResponse)?.statusCode ?? 0
                guard status == 200 else { throw UpdateError.server(status) }
                let newer = try JSONDecoder().decode([Release].self, from: data)
                    .filter { !$0.draft && !$0.prerelease && isNewer($0.version, than: current) }
                    .sorted { isNewer($0.version, than: $1.version) }
                guard let latest = newer.first else {
                    if manual { show("Pips is up to date", "You have the latest version, \(current).") }
                    return
                }
                if !manual && UserDefaults.standard.string(forKey: skipKey) == latest.version { return }
                offer(latest, notes: newer)
            } catch {
                NSLog("Pips: update check failed: \(error)")
                if manual { show("Could not check for updates", error.localizedDescription) }
            }
        }
    }

    private func offer(_ latest: Release, notes: [Release]) {
        let alert = NSAlert()
        alert.icon = NSApp.applicationIconImage
        alert.messageText = "A new version of Pips is available"
        alert.informativeText = "Pips \(latest.version) is available. You have \(current). Would you like to install it now?"
        alert.addButton(withTitle: "Install Update")
        alert.addButton(withTitle: "Remind Me Later")
        alert.addButton(withTitle: "Skip This Version")
        alert.showsSuppressionButton = true  // used as the "check automatically" checkbox
        alert.suppressionButton?.title = "Check for updates automatically"
        alert.suppressionButton?.state = UserDefaults.standard.bool(forKey: autoKey) ? .on : .off
        alert.accessoryView = notesView(notes)
        NSApp.activate(ignoringOtherApps: true)  // a menu bar app is never frontmost on its own
        let choice = alert.runModal()
        UserDefaults.standard.set(alert.suppressionButton?.state == .on, forKey: autoKey)
        switch choice {
        case .alertFirstButtonReturn: install(latest)
        case .alertThirdButtonReturn: UserDefaults.standard.set(latest.version, forKey: skipKey)
        default: break
        }
    }

    /// Release notes of every newer version, newest first, in a scrollable box.
    private func notesView(_ releases: [Release]) -> NSView {
        let text = NSMutableAttributedString()
        for r in releases {
            text.append(NSAttributedString(string: "What's new in \(r.version)\n", attributes: [.font: NSFont.boldSystemFont(ofSize: 12)]))
            let body = r.body ?? ""
            let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
            text.append((try? NSAttributedString(markdown: body, options: options)) ?? NSAttributedString(string: body))
            text.append(NSAttributedString(string: "\n\n"))
        }
        text.addAttribute(.foregroundColor, value: NSColor.labelColor, range: NSRange(location: 0, length: text.length))
        let scroll = NSTextView.scrollableTextView()
        scroll.frame = NSRect(x: 0, y: 0, width: 380, height: 160)
        let view = scroll.documentView as! NSTextView
        view.isEditable = false
        view.textStorage?.setAttributedString(text)
        return scroll
    }

    private func install(_ release: Release) {
        guard let zip = release.assets.first(where: { $0.name == assetName }),
              let sums = release.assets.first(where: { $0.name == sumsName }) else {
            show("Update failed", UpdateError.missingAsset(release.version).localizedDescription)
            return
        }
        let panel = progressPanel(release.version)
        download = Task {
            do {
                let folder = FileManager.default.temporaryDirectory.appendingPathComponent("PipsUpdate-\(UUID().uuidString)")
                try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                let (downloaded, _) = try await URLSession.shared.download(from: zip.browser_download_url)
                let file = folder.appendingPathComponent(assetName)
                try FileManager.default.moveItem(at: downloaded, to: file)
                let (sumData, _) = try await URLSession.shared.data(from: sums.browser_download_url)
                try Task.checkCancellation()
                try verify(file, sums: String(decoding: sumData, as: UTF8.self))
                try replaceAndRelaunch(with: file)
            } catch is CancellationError {
            } catch {
                show("Update failed", error.localizedDescription)
            }
            panel.close()
        }
    }

    private func verify(_ file: URL, sums: String) throws {
        // sha256sum output: "<hash>  <name>" per line
        let expected = sums.split(separator: "\n")
            .map { $0.split(separator: " ", omittingEmptySubsequences: true) }
            .first(where: { $0.count == 2 && $0[1] == assetName[...] })?[0]
        let actual = SHA256.hash(data: try Data(contentsOf: file)).map { String(format: "%02x", $0) }.joined()
        guard let expected, expected.lowercased() == actual else { throw UpdateError.checksum }
    }

    /// Unpacks the new app, then a small shell script waits for this process to quit,
    /// swaps the bundle and opens the new one.
    private func replaceAndRelaunch(with zip: URL) throws {
        let folder = zip.deletingLastPathComponent()
        let unzip = Process()
        unzip.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
        unzip.arguments = ["-x", "-k", zip.path, folder.path]
        try unzip.run()
        unzip.waitUntilExit()
        let new = folder.appendingPathComponent("Pips.app")
        guard unzip.terminationStatus == 0, FileManager.default.fileExists(atPath: new.path) else { throw UpdateError.unzip }

        let old = Bundle.main.bundleURL
        let parent = old.deletingLastPathComponent().path
        guard old.pathExtension == "app", FileManager.default.isWritableFile(atPath: parent) else {
            throw UpdateError.notWritable(parent)
        }
        let swap = Process()
        swap.executableURL = URL(fileURLWithPath: "/bin/sh")
        // paths go in as arguments, never into the script text
        swap.arguments = ["-c", "while kill -0 \(getpid()) 2>/dev/null; do sleep 0.2; done; rm -rf \"$1\" && mv \"$2\" \"$1\" && open \"$1\"",
                          "sh", old.path, new.path]
        try swap.run()
        NSApp.terminate(nil)
    }

    private func progressPanel(_ version: String) -> NSPanel {
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 340, height: 100), styleMask: [.titled], backing: .buffered, defer: false)
        panel.title = "Updating Pips"
        let label = NSTextField(labelWithString: "Downloading Pips \(version)…")
        let bar = NSProgressIndicator()
        bar.isIndeterminate = true
        bar.style = .bar
        bar.startAnimation(nil)
        bar.widthAnchor.constraint(equalToConstant: 308).isActive = true
        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancelDownload))
        let stack = NSStackView(views: [label, bar, cancel])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.edgeInsets = NSEdgeInsets(top: 14, left: 16, bottom: 14, right: 16)
        panel.contentView = stack
        panel.level = .floating
        panel.center()
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
        return panel
    }

    @objc private func cancelDownload() {
        download?.cancel()
    }

    private func show(_ title: String, _ text: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = text
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }
}

/// "1.0.10" is newer than "1.0.9"; missing parts count as 0.
func isNewer(_ a: String, than b: String) -> Bool {
    let x = a.split(separator: ".").map { Int($0) ?? 0 }
    let y = b.split(separator: ".").map { Int($0) ?? 0 }
    for i in 0..<max(x.count, y.count) {
        let p = i < x.count ? x[i] : 0, q = i < y.count ? y[i] : 0
        if p != q { return p > q }
    }
    return false
}
