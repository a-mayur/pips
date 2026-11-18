//! Battery levels Windows stores for present Bluetooth devices (the values shown in
//! Settings > Bluetooth & devices). Same data as:
//! `Get-PnpDevice -PresentOnly | Get-PnpDeviceProperty -KeyName '{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2'`

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_ALLCLASSES, DIGCF_PRESENT, HDEVINFO, SP_DEVINFO_DATA, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
    SetupDiGetClassDevsW, SetupDiGetDevicePropertyW,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_ContainerId, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_GUID, DEVPROPTYPE,
};
use windows::Win32::Foundation::DEVPROPKEY;
use std::collections::{HashMap, HashSet};

use windows::Devices::Enumeration::{DeviceInformationKind, DeviceWatcher};
use windows::core::{GUID, PCWSTR, Result};

use crate::connected;

const BATTERY_PROPERTY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2"; // same key, WinRT name
const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

// device setup classes that tell what a physical device is
const CLASS_MOUSE: GUID = GUID::from_u128(0x4d36e96f_e325_11ce_bfc1_08002be10318);
const CLASS_KEYBOARD: GUID = GUID::from_u128(0x4d36e96b_e325_11ce_bfc1_08002be10318);
const CLASS_MEDIA: GUID = GUID::from_u128(0x4d36e96c_e325_11ce_bfc1_08002be10318);
const CLASS_AUDIO_ENDPOINT: GUID = GUID::from_u128(0xc166523c_fe0c_4a94_a586_f1a80cfbbf3e);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Mouse,
    Keyboard,
    Headphones,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub name: String,
    pub level: u8,
    pub kind: Kind,
}

/// Calls `changed` (on a background thread) when Windows updates a device's battery level.
pub fn watch(changed: impl Fn() + Send + Sync + Clone + 'static) -> Result<DeviceWatcher> {
    connected::watch(
        "System.Devices.Present:=System.StructuredQueryType.Boolean#True",
        &[BATTERY_PROPERTY],
        DeviceInformationKind::Device,
        changed,
    )
}

/// Battery levels of connected Bluetooth devices, named as in Settings.
pub fn read() -> Result<Vec<Reading>> {
    // paired-but-disconnected devices keep their last battery value, so join on connected containers
    let connected = connected::connected()?;
    let mut levels: Vec<(GUID, u8)> = Vec::new();
    let mut classes: HashMap<GUID, HashSet<GUID>> = HashMap::new();
    each_node(|set, info| {
        let Some(c) = (unsafe { container(set, info) }) else { return };
        if !connected.contains_key(&c) {
            return;
        }
        classes.entry(c).or_default().insert(info.ClassGuid);
        if let Some(level) = unsafe { property(set, info, &DEVPKEY_BLUETOOTH_BATTERY, DEVPROP_TYPE_BYTE) } {
            levels.push((c, level[0]));
        }
    })?;
    let readings = levels
        .into_iter()
        .map(|(c, level)| Reading { name: connected[&c].clone(), level, kind: kind(&classes[&c]) })
        .collect();
    Ok(clean(readings))
}

/// Mouse wins over keyboard: mice often also expose a keyboard interface for their extra buttons.
fn kind(classes: &HashSet<GUID>) -> Kind {
    if classes.contains(&CLASS_MOUSE) {
        Kind::Mouse
    } else if classes.contains(&CLASS_KEYBOARD) {
        Kind::Keyboard
    } else if classes.contains(&CLASS_MEDIA) || classes.contains(&CLASS_AUDIO_ENDPOINT) {
        Kind::Headphones
    } else {
        Kind::Other
    }
}

/// Calls `f` for every present device node of every class.
fn each_node(mut f: impl FnMut(HDEVINFO, &SP_DEVINFO_DATA)) -> Result<()> {
    unsafe {
        // all classes: headphones keep their battery on an audio/hands-free node, not the Bluetooth one
        let set = SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_ALLCLASSES)?;
        let mut index = 0;
        loop {
            let mut info = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInfo(set, index, &mut info).is_err() {
                break; // ERROR_NO_MORE_ITEMS
            }
            index += 1;
            f(set, &info);
        }
        SetupDiDestroyDeviceInfoList(set)
    }
}

/// Container id: groups every device node of one physical device.
unsafe fn container(set: HDEVINFO, info: &SP_DEVINFO_DATA) -> Option<GUID> {
    let b = unsafe { property(set, info, &DEVPKEY_Device_ContainerId, DEVPROP_TYPE_GUID) }?;
    Some(GUID::from_values(
        u32::from_le_bytes(b[0..4].try_into().ok()?),
        u16::from_le_bytes(b[4..6].try_into().ok()?),
        u16::from_le_bytes(b[6..8].try_into().ok()?),
        b[8..16].try_into().ok()?,
    ))
}

/// Raw property bytes, or None when the device lacks the property or it has another type.
unsafe fn property(set: HDEVINFO, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY, want: DEVPROPTYPE) -> Option<Vec<u8>> {
    let mut kind = DEVPROPTYPE::default();
    let mut size = 0u32;
    // first call only reports the size; it fails with ERROR_INSUFFICIENT_BUFFER by design
    let _ = unsafe { SetupDiGetDevicePropertyW(set, info, key, &mut kind, None, Some(&mut size), 0) };
    if size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    unsafe { SetupDiGetDevicePropertyW(set, info, key, &mut kind, Some(&mut buffer), None, 0) }.ok()?;
    (kind == want).then_some(buffer)
}

/// Drops "no reading" (0) and keeps one entry per name: a device can have several nodes with battery.
fn clean(mut readings: Vec<Reading>) -> Vec<Reading> {
    readings.retain(|r| r.level > 0);
    readings.sort_by(|a, b| a.name.cmp(&b.name).then(b.level.cmp(&a.level)));
    readings.dedup_by(|a, b| a.name == b.name);
    readings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str, level: u8) -> Reading {
        Reading { name: name.into(), level, kind: Kind::Other }
    }

    #[test]
    fn clean_drops_zero_and_duplicates() {
        let out = clean(vec![r("Mouse", 80), r("Buds", 0), r("Mouse", 80), r("Keys", 40)]);
        assert_eq!(out, vec![r("Keys", 40), r("Mouse", 80)]);
    }

    #[test]
    fn mouse_wins_over_keyboard() {
        assert_eq!(kind(&HashSet::from([CLASS_KEYBOARD, CLASS_MOUSE])), Kind::Mouse);
        assert_eq!(kind(&HashSet::from([CLASS_KEYBOARD])), Kind::Keyboard);
        assert_eq!(kind(&HashSet::from([CLASS_AUDIO_ENDPOINT])), Kind::Headphones);
        assert_eq!(kind(&HashSet::new()), Kind::Other);
    }
}
