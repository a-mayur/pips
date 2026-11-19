import AppKit
import ServiceManagement

/// Owns the menu bar item: refresh schedule, menu bar image, device popover and options menu.
@MainActor
final class StatusController: NSObject, NSPopoverDelegate {
    /// Safety net for devices that push no battery changes (headphones on the private API).
    private static let backgroundRefresh: TimeInterval = 600
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
    /// x range of each device in the menu bar image, same order as `devices`
    private var segments: [ClosedRange<CGFloat>] = []
    private let popover = NSPopover()
    private var popoverDevice: String?  // name of the device whose popover is open
    private var popoverClosed = Date.distantPast
    private var popoverAnchor = NSRect.zero  // the device's part of the button, in button coordinates
    private var hoverTimer: Timer?
    private var pointerLeft: Date?  // when the pointer left the popover and its device

    override init() {
        super.init()
        ble.onUpdate = { [weak self] name, level in
            Task { @MainActor in self?.batteryChanged(name: name, level: level) }
        }
        NSWorkspace.shared.notificationCenter.addObserver(
            self, selector: #selector(didWake), name: NSWorkspace.didWakeNotification, object: nil)
        watcher = ConnectionWatcher { [weak self] in
            Task { @MainActor in self?.deviceChanged() }
        }
        popover.behavior = .transient  // closes on any click outside it
        popover.delegate = self
        item.button?.target = self
        item.button?.action = #selector(clicked(_:))
        item.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
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

    /// A BLE device pushed a new level: update it in place, or refresh if it is not shown yet.
    private func batteryChanged(name: String, level: Int) {
        guard let i = devices.firstIndex(where: { $0.name == name }) else { refresh(); return }
        devices[i] = Device(name: name, kind: devices[i].kind, barLevel: level, detail: "\(level)%", components: devices[i].components)
        updated = Date()
        render()
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
        timer = Timer.scheduledTimer(withTimeInterval: Self.backgroundRefresh, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    // MARK: Settings actions

    /// The standard About panel: icon, name and version from Info.plist (set from the release tag by build.sh).
    @objc private func showAbout() {
        let style = NSMutableParagraphStyle()
        style.alignment = .center
        let small: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 11), .paragraphStyle: style,
                                                    .foregroundColor: NSColor.secondaryLabelColor]
        let credits = NSMutableAttributedString(
            string: "github.com/a-mayur/pips",
            attributes: [.font: NSFont.systemFont(ofSize: 12), .paragraphStyle: style,
                         .link: URL(string: "https://github.com/a-mayur/pips")!])
        credits.append(NSAttributedString(string: "\n\nFree and open source · GPL-3.0", attributes: small))
        NSApp.activate(ignoringOtherApps: true)  // a menu bar app is never frontmost on its own
        // an empty build number shows "Version 1.0.0" instead of "Version 1.0.0 (1.0.0)"
        NSApp.orderFrontStandardAboutPanel(options: [.credits: credits, .version: ""])
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
        let (image, segments) = barImage()
        item.button?.image = image
        item.button?.toolTip = "Pips"
        self.segments = segments
        if popover.isShown { showPopover() }  // live: new levels, or close if the device left
    }

    // MARK: Clicks

    /// Left click: the clicked device's popover. Right click or Control-click: the options menu.
    @objc private func clicked(_ sender: NSStatusBarButton) {
        guard let event = NSApp.currentEvent else { return }
        if event.type == .rightMouseUp || event.modifierFlags.contains(.control) {
            showMenu(sender)
            return
        }
        // the pointer itself: newer macOS reports every menu bar click at the item's center
        guard let window = sender.window else { return }
        let point = sender.convert(window.convertPoint(fromScreen: NSEvent.mouseLocation), from: nil)
        let x = point.x - imageOffset(sender)
        // nearest device, so a click in the gap between two devices still opens one
        let distance = { (s: ClosedRange<CGFloat>) in s.contains(x) ? 0 : min(abs(x - s.lowerBound), abs(x - s.upperBound)) }
        guard let i = segments.indices.min(by: { distance(segments[$0]) < distance(segments[$1]) }), i < devices.count else {
            showMenu(sender)  // no device shown: the menu is the only useful thing
            return
        }
        let name = devices[i].name
        // a click on the same device closes it; the transient popover has already closed itself on mouse down
        if popoverDevice == name && (popover.isShown || Date().timeIntervalSince(popoverClosed) < 0.3) {
            popover.close()
            popoverDevice = nil
            return
        }
        popoverDevice = name
        showPopover()
    }

    private func showMenu(_ button: NSStatusBarButton) {
        popover.close()
        menu().popUp(positioning: nil, at: NSPoint(x: 0, y: button.bounds.height + 4), in: button)
    }

    /// Shows or updates the popover for `popoverDevice` under its part of the menu bar item.
    private func showPopover() {
        guard let name = popoverDevice, let i = devices.firstIndex(where: { $0.name == name }), i < segments.count,
              let button = item.button else {
            popover.close()
            popoverDevice = nil
            return
        }
        popover.contentViewController = DeviceCard(device: devices[i])
        let s = segments[i], offset = imageOffset(button)
        popoverAnchor = NSRect(x: offset + s.lowerBound, y: 0, width: s.upperBound - s.lowerBound, height: button.bounds.height)
        popover.show(relativeTo: popoverAnchor, of: button, preferredEdge: .minY)
        if hoverTimer == nil {
            hoverTimer = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { [weak self] _ in
                Task { @MainActor in self?.closeIfPointerLeft() }
            }
        }
    }

    /// Closes the popover once the pointer has been away from it and its device for half a second.
    private func closeIfPointerLeft() {
        guard popover.isShown, let card = popover.contentViewController?.view.window?.frame,
              let button = item.button, let window = button.window else { return }
        let anchor = window.convertToScreen(button.convert(popoverAnchor, to: nil))
        // one area covering the device, the popover and the arrow gap between them
        if card.union(anchor).insetBy(dx: -6, dy: -6).contains(NSEvent.mouseLocation) {
            pointerLeft = nil
        } else if let left = pointerLeft {
            if Date().timeIntervalSince(left) >= 0.5 { popover.close() }
        } else {
            pointerLeft = Date()
        }
    }

    /// The button draws its image centered.
    private func imageOffset(_ button: NSStatusBarButton) -> CGFloat {
        (button.bounds.width - (button.image?.size.width ?? 0)) / 2
    }

    func popoverDidClose(_ notification: Notification) {
        popoverClosed = Date()
        hoverTimer?.invalidate()
        hoverTimer = nil
        pointerLeft = nil
    }

    // MARK: Drawing

    /// One template image with an icon and percentage per device, so it follows the menu bar's light/dark tint.
    /// Also returns each device's x range, for hit-testing clicks.
    private func barImage() -> (NSImage, [ClosedRange<CGFloat>]) {
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

        var segments: [ClosedRange<CGFloat>] = []
        var start: CGFloat = 0
        for (icon, text) in parts {
            let w = icon.size.width + (text.length > 0 ? gap + text.size(withAttributes: attrs).width : 0)
            segments.append(start...(start + w))
            start += w + spacing
        }

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
        return (image, devices.isEmpty ? [] : segments)
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

        let login = action("Launch at Login", #selector(toggleLaunchAtLogin))
        login.state = SMAppService.mainApp.status == .enabled ? .on : .off
        menu.addItem(login)

        menu.addItem(.separator())
        menu.addItem(action("About Pips", #selector(showAbout)))
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
