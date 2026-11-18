import IOBluetooth

/// Calls `onChange` whenever a Bluetooth device connects or disconnects.
/// IOBluetooth also reports devices already connected when registration starts.
final class ConnectionWatcher: NSObject {
    private let onChange: () -> Void
    private var connectNotification: IOBluetoothUserNotification?

    init(onChange: @escaping () -> Void) {
        self.onChange = onChange
        super.init()
        connectNotification = IOBluetoothDevice.register(
            forConnectNotifications: self, selector: #selector(connected(_:fromDevice:)))
    }

    @objc private func connected(_ note: IOBluetoothUserNotification, fromDevice device: IOBluetoothDevice) {
        NSLog("Pips: connected \(device.name ?? "unknown")")
        device.register(forDisconnectNotification: self, selector: #selector(disconnected(_:fromDevice:)))
        onChange()
    }

    @objc private func disconnected(_ note: IOBluetoothUserNotification, fromDevice device: IOBluetoothDevice) {
        NSLog("Pips: disconnected \(device.name ?? "unknown")")
        note.unregister()
        onChange()
    }
}
