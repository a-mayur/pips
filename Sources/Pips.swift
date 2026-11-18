import AppKit

@main
enum Pips {
    @MainActor static func main() {
        if CommandLine.arguments.contains(ClassicBatteryReader.helperFlag) { ClassicBatteryReader.runHelper() }

        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)  // menu bar only, no Dock icon
        let controller = StatusController()
        withExtendedLifetime(controller) { app.run() }
    }
}
