import AppKit
import ServiceManagement

/// Owns the menu bar item: refresh schedule, menu bar image and dropdown menu.
@MainActor
final class StatusController: NSObject {
    static let intervals = [3, 6, 12, 24]  // hours
    private static let intervalKey = "refreshHours"
    private static let wakeDelay: TimeInterval = 15  // devices reconnect a few seconds after wake
    private static let connectDelay: TimeInterval = 3  // battery level is not readable at the instant of connect
    private static let lateDelay: TimeInterval = 15  // headphones report battery several seconds after connecting

    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let ble = BLEBatteryReader()
    private let classic = ClassicBatteryReader()
    private var watcher: ConnectionWatcher?
    private var timer: Timer?
    private var pendingRefresh: DispatchWorkItem?
    private var refreshing = false
    private var refreshAgain = false  // a change arrived while a refresh was running
    private var devices: [Device] = []
    private var errors: [String] = []
    private var updated: Date?

    private var hours: Int {
        get { UserDefaults.standard.integer(forKey: Self.intervalKey) }
        set { UserDefaults.standard.set(newValue, forKey: Self.intervalKey) }
    }

    override init() {
        super.init()
        UserDefaults.standard.register(defaults: [Self.intervalKey: 3])
        NSWorkspace.shared.notificationCenter.addObserver(
            self, selector: #selector(didWake), name: NSWorkspace.didWakeNotification, object: nil)
        watcher = ConnectionWatcher { [weak self] in
            Task { @MainActor in self?.deviceChanged() }
        }
        render()
        schedule()
        refresh()
    }

    // MARK: Refresh

    @objc private func refresh() {
        if refreshing { refreshAgain = true; return }
        refreshing = true
        ble.read { [weak self] result in
            Task { @MainActor in self?.finishRefresh(result) }
        }
    }

    private func finishRefresh(_ result: Result<[String: Int], BLEError>) {
        apply(result)
        refreshing = false
        if refreshAgain {
            refreshAgain = false
            refresh()
        }
    }

    private func deviceChanged() {
        refreshSoon(after: Self.connectDelay)
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.lateDelay) { [weak self] in
            Task { @MainActor in self?.refresh() }
        }
    }

    /// Debounced refresh: several connect/disconnect events in a row cause one refresh.
    private func refreshSoon(after delay: TimeInterval) {
        pendingRefresh?.cancel()
        let work = DispatchWorkItem { [weak self] in
            Task { @MainActor in self?.refresh() }
        }
        pendingRefresh = work
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }

    private func apply(_ bleResult: Result<[String: Int], BLEError>) {
        var list: [Device] = []
        var errs: [String] = []

        switch bleResult {
        case .success(let levels):
            list = levels.map { name, level in
                Device(name: name, kind: HIDRegistry.kind(named: name), barLevel: level, detail: "\(level)%")
            }
        case .failure(let error):
            errs.append(error.description)
        }

        do {
            // BLE reading wins when both sources report the same device
            list += try classic.read().filter { c in !list.contains { $0.name == c.name } }
        } catch let error as ClassicError {
            errs.append(error.description)
        } catch {
            errs.append("\(error)")
        }

        devices = list.sorted { $0.name < $1.name }
        NSLog("Pips: refresh ble=\(bleResult) shown=\(devices.map(\.name)) errors=\(errs)")
        errors = errs
        updated = Date()
        render()
    }

    @objc private func didWake() {
        refreshSoon(after: Self.wakeDelay)
    }

    private func schedule() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: TimeInterval(hours * 3600), repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    // MARK: Settings actions

    @objc private func setInterval(_ sender: NSMenuItem) {
        hours = sender.tag
        schedule()
        render()
    }

    @objc private func toggleLaunchAtLogin() {
        let service = SMAppService.mainApp
        do {
            if service.status == .enabled { try service.unregister() } else { try service.register() }
        } catch {
            let alert = NSAlert(error: error)
            alert.messageText = "Could not change Launch at Login"
            alert.runModal()
        }
        render()
    }

    // MARK: Rendering

    private func render() {
        item.button?.image = barImage()
        item.button?.toolTip = "Pips"
        item.menu = menu()
    }

    /// One template image with an icon and percentage per device, so it follows the menu bar's light/dark tint.
    private func barImage() -> NSImage {
        let font = NSFont.menuBarFont(ofSize: 0)
        let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.black]
        let config = NSImage.SymbolConfiguration(pointSize: font.pointSize, weight: .regular)
        let symbol = { (name: String) in
            NSImage(systemSymbolName: name, accessibilityDescription: nil)!.withSymbolConfiguration(config)!
        }

        let parts: [(NSImage, NSString)] = devices.isEmpty
            ? [(symbol("battery.0"), "" as NSString)]
            : devices.map { (symbol($0.kind.symbol), "\($0.barLevel)%" as NSString) }

        let gap: CGFloat = 3, spacing: CGFloat = 8
        let height = NSStatusBar.system.thickness
        let width = parts.reduce(CGFloat(0)) { w, p in
            w + p.0.size.width + (p.1.length > 0 ? gap + p.1.size(withAttributes: attrs).width : 0)
        } + spacing * CGFloat(parts.count - 1)

        let image = NSImage(size: NSSize(width: ceil(width), height: height), flipped: false) { _ in
            var x: CGFloat = 0
            for (icon, text) in parts {
                icon.draw(in: NSRect(x: x, y: (height - icon.size.height) / 2, width: icon.size.width, height: icon.size.height))
                x += icon.size.width
                if text.length > 0 {
                    let size = text.size(withAttributes: attrs)
                    x += gap
                    text.draw(at: NSPoint(x: x, y: (height - size.height) / 2), withAttributes: attrs)
                    x += size.width
                }
                x += spacing
            }
            return true
        }
        image.isTemplate = true
        return image
    }

    private func menu() -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false

        let style = NSMutableParagraphStyle()
        style.tabStops = [NSTextTab(textAlignment: .right, location: 280)]
        let font = NSFont.menuFont(ofSize: 0)
        for d in devices {
            let row = NSMenuItem(title: d.name, action: nil, keyEquivalent: "")
            row.attributedTitle = NSAttributedString(
                string: "\(d.name)\t\(d.detail)", attributes: [.paragraphStyle: style, .font: font])
            menu.addItem(row)
        }
        if devices.isEmpty { menu.addItem(disabled("No Bluetooth devices reporting battery")) }
        for e in errors { menu.addItem(disabled(e)) }

        menu.addItem(.separator())
        if let updated {
            menu.addItem(disabled("Updated \(updated.formatted(date: .omitted, time: .shortened))"))
        }
        menu.addItem(action("Refresh Now", #selector(refresh), key: "r"))

        let every = NSMenuItem(title: "Refresh Every", action: nil, keyEquivalent: "")
        let sub = NSMenu()
        for h in Self.intervals {
            let option = action("\(h) hours", #selector(setInterval(_:)))
            option.tag = h
            option.state = h == hours ? .on : .off
            sub.addItem(option)
        }
        every.submenu = sub
        menu.addItem(every)

        let login = action("Launch at Login", #selector(toggleLaunchAtLogin))
        login.state = SMAppService.mainApp.status == .enabled ? .on : .off
        menu.addItem(login)

        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "Quit Pips", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
        return menu
    }

    private func action(_ title: String, _ selector: Selector, key: String = "") -> NSMenuItem {
        let item = NSMenuItem(title: title, action: selector, keyEquivalent: key)
        item.target = self
        return item
    }

    private func disabled(_ title: String) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        return item
    }
}
