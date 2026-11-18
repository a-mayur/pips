import AppKit

/// Content of the device popover: icon, name, kind, level, and a bar per battery.
final class DeviceCard: NSViewController {
    static let low = 20

    private let device: Device

    init(device: Device) {
        self.device = device
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    override func loadView() {
        let icon = NSImageView(image: NSImage(systemSymbolName: device.kind.symbol, accessibilityDescription: device.kind.label)!
            .withSymbolConfiguration(.init(pointSize: 22, weight: .regular))!)
        let name = label(device.name, .systemFont(ofSize: 14, weight: .semibold), .labelColor)
        name.lineBreakMode = .byTruncatingTail
        let subtitle = label("\(device.kind.label) · Connected", .systemFont(ofSize: 12), .secondaryLabelColor)
        let titles = NSStackView(views: [name, subtitle])
        titles.orientation = .vertical
        titles.alignment = .leading
        titles.spacing = 1
        titles.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)  // the name truncates, the % does not
        let percent = label("\(device.barLevel)%", .systemFont(ofSize: 22, weight: .semibold),
                            device.barLevel <= Self.low ? .systemRed : .labelColor)
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let header = NSStackView(views: [icon, titles, spacer, percent])
        header.spacing = 10

        let rows: [NSView] = [header] + (device.components.isEmpty
            ? [BarView(level: device.barLevel)]
            : device.components.map(componentRow))
        let stack = NSStackView(views: rows)
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 14, bottom: 12, right: 14)
        stack.widthAnchor.constraint(equalToConstant: 260).isActive = true
        for row in rows { row.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -28).isActive = true }
        view = stack
        preferredContentSize = stack.fittingSize  // NSPopover sizes itself from this
    }

    /// "Left  [bar]  80%"
    private func componentRow(_ c: Component) -> NSView {
        let name = label(c.label, .systemFont(ofSize: 12), .labelColor)
        let value = label("\(c.level)%", .monospacedDigitSystemFont(ofSize: 12, weight: .regular), .labelColor)
        value.alignment = .right
        let row = NSStackView(views: [name, BarView(level: c.level), value])
        row.spacing = 8
        name.widthAnchor.constraint(equalToConstant: 40).isActive = true
        value.widthAnchor.constraint(equalToConstant: 36).isActive = true
        return row
    }

    private func label(_ text: String, _ font: NSFont, _ color: NSColor) -> NSTextField {
        let field = NSTextField(labelWithString: text)
        field.font = font
        field.textColor = color
        return field
    }
}

/// Rounded level bar: green, red at 20% or less.
final class BarView: NSView {
    private let level: Int

    init(level: Int) {
        self.level = level
        super.init(frame: .zero)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: 6) }

    override func draw(_ dirtyRect: NSRect) {
        let radius = bounds.height / 2
        NSColor.quaternaryLabelColor.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: radius, yRadius: radius).fill()
        var fill = bounds
        fill.size.width = max(bounds.height, bounds.width * CGFloat(min(level, 100)) / 100)
        (level <= DeviceCard.low ? NSColor.systemRed : NSColor.systemGreen).setFill()
        NSBezierPath(roundedRect: fill, xRadius: radius, yRadius: radius).fill()
    }
}
