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
/// Result maps peripheral name to percentage.
final class BLEBatteryReader: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    typealias Completion = (Result<[String: Int], BLEError>) -> Void

    private let batteryService = CBUUID(string: "180F")
    private let batteryLevel = CBUUID(string: "2A19")
    private let timeout: TimeInterval = 10

    private var central: CBCentralManager!
    private var peripherals: [CBPeripheral] = []  // must be retained or CoreBluetooth drops them
    private var levels: [String: Int] = [:]
    private var remaining = 0
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
        peripherals = central.retrieveConnectedPeripherals(withServices: [batteryService])
        remaining = peripherals.count
        if remaining == 0 { finish(.success([:])); return }
        for p in peripherals {
            p.delegate = self
            central.connect(p)
        }
    }

    private func finish(_ result: Result<[String: Int], BLEError>) {
        guard let completion else { return }
        self.completion = nil
        generation += 1
        for p in peripherals { central.cancelPeripheralConnection(p) }  // drops only our app's link
        peripherals = []
        completion(result)
    }

    private func done() {
        remaining -= 1
        if remaining == 0 { finish(.success(levels)) }
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
        done()
    }

    // MARK: CBPeripheralDelegate

    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard error == nil, let service = peripheral.services?.first(where: { $0.uuid == batteryService }) else {
            NSLog("Pips: no battery service on \(peripheral.name ?? "unknown"): \(String(describing: error))")
            done(); return
        }
        peripheral.discoverCharacteristics([batteryLevel], for: service)
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard error == nil, let c = service.characteristics?.first(where: { $0.uuid == batteryLevel }) else {
            NSLog("Pips: no battery level on \(peripheral.name ?? "unknown"): \(String(describing: error))")
            done(); return
        }
        peripheral.readValue(for: c)
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        if error == nil, let level = characteristic.value?.first, level > 0 {
            levels[peripheral.name ?? "Unknown"] = Int(level)
        } else {
            NSLog("Pips: battery read failed on \(peripheral.name ?? "unknown"): \(String(describing: error))")
        }
        done()
    }
}
