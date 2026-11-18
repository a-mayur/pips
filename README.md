<img src="docs/icon.png" width="128" alt="Pips icon">

# Pips

Battery levels of your Bluetooth mouse, keyboard and headphones, in the macOS menu bar.

> **Windows version coming soon.**

![Pips demo](docs/demo.gif)

<sub>Mockup of the menu bar item and menu. Device names are examples.</sub>

- Icon and percentage per connected device. Nothing else.
- Earbuds show the lower of left/right in the menu bar, and left, right and case in the menu.
- Updates when a device connects or disconnects, on wake, every 3, 6, 12 or 24 hours (default 3), and on demand.
- Launch at Login.
- Native Swift/AppKit, no dependencies, no Dock icon.

## Install

1. Download `Pips.dmg` from [Releases](../../releases/latest).
2. Open it and drag **Pips** to **Applications**.
3. Open Pips. macOS blocks it because it is not notarized. Go to **System Settings > Privacy & Security** and click **Open Anyway**.
   Or run: `xattr -dr com.apple.quarantine /Applications/Pips.app`
4. Allow Bluetooth access when asked.

Requires macOS 13 or later. Universal binary (Apple Silicon and Intel).

## Build from source

Needs Xcode or the Command Line Tools (`xcode-select --install`).

```bash
./build.sh
```

Output in `build/`: `Pips.app`, `Pips.dmg`, `Pips.zip`, `SHA256SUMS.txt`.

## How it works

| Devices | Source |
|---|---|
| BLE devices (most mice and keyboards) | CoreBluetooth, Battery Service `0x180F` |
| Classic Bluetooth (headphones, earbuds) | `IOBluetoothDevice` battery properties |

The headphone source uses private macOS API. If a macOS update removes it, Pips shows an error in the menu and BLE devices keep working.

Devices that report no battery level are not shown. Devices on a USB receiver (Logitech Unifying/Bolt) are not Bluetooth and are not supported.

## Release

Every push to `main` builds the app and uploads it as a workflow artifact. Pushing a `v*` tag also publishes a GitHub Release:

```bash
git tag v1.0.0 && git push origin v1.0.0
```

## License

[GPL-3.0-or-later](LICENSE). Anyone who distributes Pips or a modified version must release its source under the same license.
