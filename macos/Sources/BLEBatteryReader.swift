import CoreBluetooth
import Foundation

enum BLEError: Error, CustomStringConvertible {
    case unavailable(CBManagerState)

    var description: String {
        switch self {
        case .unavailable(.unauthorized): return "Bluetooth permission denied. Allow Pips in System Settings > Privacy & Security > Bluetooth."
        case .unavailable(.poweredOff): return "Bluetooth is off."
        case .unavailable(let state): return "Bluetooth unavailable (state \(state.rawValue))."
        }
    }
}

/// Reads Battery Level (0x2A19) from connected BLE peripherals exposing Battery Service (0x180F).
/// Result maps peripheral name to percentage. Peripherals stay connected after a read so devices
/// that support it can push battery changes, delivered through `onUpdate`.
final class BLEBatteryReader: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    typealias Completion = (Result<[String: Int], BLEError>) -> Void

    /// Called on the main queue when a device pushes a new level outside a read.
    var onUpdate: ((String, Int) -> Void)?

    private let batteryService = CBUUID(string: "180F")
    private let batteryLevel = CBUUID(string: "2A19")
    private let timeout: TimeInterval = 10

    private var central: CBCentralManager!
    private var peripherals: [CBPeripheral] = []  // must be retained or CoreBluetooth drops them
    private var pending: Set<UUID> = []  // peripherals the current read still waits for
    private var levels: [String: Int] = [:]
    private var completion: Completion?
    private var generation = 0  // ties a timeout to the read that scheduled it

    override init() {
        super.init()
        central = CBCentralManager(delegate: self, queue: nil)
    }

    /// Calls `completion` once on the main queue. Ignored while a read is in progress.
    func read(completion: @escaping Completion) {
        guard self.completion == nil else { return }
        self.completion = completion
        levels = [:]
        generation += 1
        let current = generation
        DispatchQueue.main.asyncAfter(deadline: .now() + timeout) { [weak self] in
            guard let self, self.generation == current else { return }
            self.finish(.success(self.levels))
        }
        if central.state == .poweredOn { start() }
        // otherwise centralManagerDidUpdateState starts or fails the read
    }

    private func start() {
        for p in peripherals { central.cancelPeripheralConnection(p) }  // drops only our app's link
        peripherals = central.retrieveConnectedPeripherals(withServices: [batteryService])
        pending = Set(peripherals.map(\.identifier))
        if pending.isEmpty { finish(.success([:])); return }
        for p in peripherals {
            p.delegate = self
            central.connect(p)
        }
    }

    private func finish(_ result: Result<[String: Int], BLEError>) {
        guard let completion else { return }
        self.completion = nil
        pending = []
        generation += 1
        completion(result)
    }

    private func done(_ peripheral: CBPeripheral) {
        guard pending.remove(peripheral.identifier) != nil else { return }
        if pending.isEmpty { finish(.success(levels)) }
    }

    // MARK: CBCentralManagerDelegate

    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        guard completion != nil else { return }
        switch central.state {
        case .poweredOn: start()
        case .unknown, .resetting: break  // wait for a final state
        default: finish(.failure(.unavailable(central.state)))
        }
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        peripheral.discoverServices([batteryService])
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
        NSLog("Pips: connect failed for \(peripheral.name ?? "unknown"): \(String(describing: error))")
        done(peripheral)
    }

    // MARK: CBPeripheralDelegate

    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard error == nil, let service = peripheral.services?.first(where: { $0.uuid == batteryService }) else {
            NSLog("Pips: no battery service on \(peripheral.name ?? "unknown"): \(String(describing: error))")
            done(peripheral); return
        }
        peripheral.discoverCharacteristics([batteryLevel], for: service)
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard error == nil, let c = service.characteristics?.first(where: { $0.uuid == batteryLevel }) else {
            NSLog("Pips: no battery level on \(peripheral.name ?? "unknown"): \(String(describing: error))")
            done(peripheral); return
        }
        peripheral.readValue(for: c)
        if c.properties.contains(.notify) { peripheral.setNotifyValue(true, for: c) }
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        let name = peripheral.name ?? "Unknown"
        guard error == nil, let level = characteristic.value?.first, level > 0 else {
            NSLog("Pips: battery read failed on \(name): \(String(describing: error))")
            done(peripheral); return
        }
        if pending.contains(peripheral.identifier) {
            levels[name] = Int(level)
            done(peripheral)
        } else {
            NSLog("Pips: battery changed on \(name): \(level)%")
            onUpdate?(name, Int(level))
        }
    }
}
