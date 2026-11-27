import Foundation

enum DeviceKind: String, Codable {
    case mouse, keyboard, headphones, earbuds, phone, other

    /// SF Symbol shown in the menu bar.
    var symbol: String {
        switch self {
        case .mouse: return "computermouse"
        case .keyboard: return "keyboard"
        case .headphones: return "headphones"
        case .earbuds: return "airpods"
        case .phone: return "iphone"
        case .other: return "dot.radiowaves.left.and.right"
        }
    }

    /// Shown in the device popover, e.g. "Mouse · Connected".
    var label: String {
        switch self {
        case .mouse: return "Mouse"
        case .keyboard: return "Keyboard"
        case .headphones: return "Headphones"
        case .earbuds: return "Earbuds"
        case .phone: return "Phone"
        case .other: return "Bluetooth device"
        }
    }
}

/// One battery of a multi-battery device, e.g. ("Left", 80).
struct Component: Codable {
    let label: String
    let level: Int
}

struct Device: Codable {
    let name: String
    let kind: DeviceKind
    /// Percentage shown in the menu bar. For earbuds: the lower of left and right.
    let barLevel: Int
    /// Text shown in the dropdown, e.g. "85%" or "L 80% · R 75% · Case 40%".
    let detail: String
    /// Left, right and case for earbuds; empty for single-battery devices.
    var components: [Component] = []
}
