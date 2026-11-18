import Foundation

enum DeviceKind: String, Codable {
    case mouse, keyboard, headphones, earbuds, other

    /// SF Symbol shown in the menu bar.
    var symbol: String {
        switch self {
        case .mouse: return "computermouse"
        case .keyboard: return "keyboard"
        case .headphones: return "headphones"
        case .earbuds: return "airpods"
        case .other: return "dot.radiowaves.left.and.right"
        }
    }
}

struct Device: Codable {
    let name: String
    let kind: DeviceKind
    /// Percentage shown in the menu bar. For earbuds: the lower of left and right.
    let barLevel: Int
    /// Text shown in the dropdown, e.g. "85%" or "L 80% · R 75% · Case 40%".
    let detail: String
}
