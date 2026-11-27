import Foundation
import IOBluetooth

enum ClassicError: Error, CustomStringConvertible {
    case missingAPI(String)
    case helperFailed(String)

    var description: String {
        switch self {
        case .missingAPI(let getter): return "Headphone battery unavailable: this macOS has no IOBluetoothDevice.\(getter)."
        case .helperFailed(let message): return "Headphone battery read failed: \(message)"
        }
    }
}

/// Reads battery levels macOS already knows for connected Bluetooth devices (headphones, earbuds).
/// Uses private IOBluetoothDevice getters; a macOS update can remove them, which throws `missingAPI`.
struct ClassicBatteryReader {
    static let helperFlag = "--read-classic"
    private static let getters = ["batteryPercentSingle", "batteryPercentLeft", "batteryPercentRight", "batteryPercentCase"]

    /// IOBluetooth keeps a device's battery values from the first read and does not update them after
    /// a reconnect, so every read runs in a fresh child process: `Pips --read-classic`.
    func read() throws -> [Device] {
        let process = Process()
        process.executableURL = Bundle.main.executableURL!
        process.arguments = [Self.helperFlag]
        let out = Pipe(), err = Pipe()
        process.standardOutput = out
        process.standardError = err
        try process.run()
        // a hung child would block the menu forever; stop it after 5 s
        DispatchQueue.global().asyncAfter(deadline: .now() + 5) { if process.isRunning { process.terminate() } }
        let data = out.fileHandleForReading.readDataToEndOfFile()  // read before waiting so the pipe cannot fill up
        process.waitUntilExit()
        if process.terminationReason == .uncaughtSignal { throw ClassicError.helperFailed("timed out") }
        guard process.terminationStatus == 0 else {
            let message = String(decoding: err.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            throw ClassicError.helperFailed(message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return try JSONDecoder().decode([Device].self, from: data)
    }

    /// Entry point of the child process: prints the devices as JSON and exits.
    static func runHelper() -> Never {
        do {
            FileHandle.standardOutput.write(try JSONEncoder().encode(readInProcess()))
            exit(0)
        } catch {
            FileHandle.standardError.write("\(error)".data(using: .utf8)!)
            exit(1)
        }
    }

    private static func readInProcess() throws -> [Device] {
        var devices: [Device] = []
        for d in connected() {
            for g in getters where !d.responds(to: Selector(g)) { throw ClassicError.missingAPI(g) }
            // 0 means macOS has no reading
            let level = { (key: String) in (d.value(forKey: key) as? NSNumber)?.intValue ?? 0 }
            let name = d.name ?? "Unknown"
            let left = level("batteryPercentLeft"), right = level("batteryPercentRight")

            if left > 0 || right > 0 {
                // (popover label, menu label, level)
                let parts = [("Left", "L", left), ("Right", "R", right), ("Case", "Case", level("batteryPercentCase"))]
                    .filter { $0.2 > 0 }
                let detail = parts.map { "\($0.1) \($0.2)%" }.joined(separator: " · ")
                let bar = [left, right].filter { $0 > 0 }.min()!
                let components = parts.map { Component(label: $0.0, level: $0.2) }
                devices.append(Device(name: name, kind: .earbuds, barLevel: bar, detail: detail, components: components))
            } else {
                let single = level("batteryPercentSingle")
                guard single > 0 else { continue }
                devices.append(Device(name: name, kind: kind(of: d), barLevel: single, detail: "\(single)%"))
            }
        }
        return devices
    }

    /// Device kind from the Bluetooth Class of Device. BLE devices have none; HIDRegistry covers them.
    private static func kind(of d: IOBluetoothDevice) -> DeviceKind {
        switch d.deviceClassMajor {
        case 0x05:  // peripheral; minor bits 0x10 keyboard, 0x20 pointing
            switch d.deviceClassMinor & 0x30 {
            case 0x10, 0x30: return .keyboard
            case 0x20: return .mouse
            default: return .other
            }
        case 0x04: return .headphones  // audio/video
        case 0x02: return .phone
        default: return .other
        }
    }

    /// Kind of the paired device with this name, from its Class of Device. An iPhone connected over
    /// Bluetooth LE has no HID interface, but its classic pairing record says "phone".
    static func kind(named name: String) -> DeviceKind {
        guard let d = (IOBluetoothDevice.pairedDevices() as? [IOBluetoothDevice] ?? []).first(where: { $0.name == name }) else {
            return .other
        }
        return kind(of: d)
    }

    private static func connected() -> [IOBluetoothDevice] {
        (IOBluetoothDevice.pairedDevices() as? [IOBluetoothDevice] ?? []).filter { $0.isConnected() }
    }
}
