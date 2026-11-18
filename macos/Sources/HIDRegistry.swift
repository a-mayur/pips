import IOKit

/// Device kind of a Bluetooth HID device, read from the IOKit registry. Needs no permission.
enum HIDRegistry {
    private static let genericDesktop = 0x01, mouse = 0x02, keyboard = 0x06

    /// Mouse wins over keyboard: mice often also expose a keyboard interface for their extra buttons.
    static func kind(named name: String) -> DeviceKind {
        let usages = genericDesktopUsages(named: name)
        if usages.contains(mouse) { return .mouse }
        if usages.contains(keyboard) { return .keyboard }
        return .other
    }

    /// Generic Desktop usages of every Bluetooth HID interface whose product name matches.
    private static func genericDesktopUsages(named name: String) -> Set<Int> {
        var iterator: io_iterator_t = 0
        guard IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("IOHIDDevice"), &iterator) == KERN_SUCCESS else {
            return []
        }
        defer { IOObjectRelease(iterator) }

        var found = Set<Int>()
        var service = IOIteratorNext(iterator)
        while service != 0 {
            found.formUnion(usages(of: service, named: name))
            IOObjectRelease(service)
            service = IOIteratorNext(iterator)
        }
        return found
    }

    private static func usages(of service: io_service_t, named name: String) -> Set<Int> {
        guard property(service, "Product") as? String == name,
              (property(service, "Transport") as? String)?.hasPrefix("Bluetooth") == true,
              let pairs = property(service, "DeviceUsagePairs") as? [[String: Any]] else { return [] }
        return Set(pairs.compactMap { p in
            p["DeviceUsagePage"] as? Int == genericDesktop ? p["DeviceUsage"] as? Int : nil
        })
    }

    private static func property(_ service: io_service_t, _ key: String) -> Any? {
        IORegistryEntryCreateCFProperty(service, key as CFString, kCFAllocatorDefault, 0)?.takeRetainedValue()
    }
}
