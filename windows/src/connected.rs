//! Connected devices, from Windows' device containers (one container per physical device).
//! Per-device-node properties have no reliable "connected" flag for classic Bluetooth
//! devices such as headphones. Bluetooth association-endpoint queries carry the flag too,
//! but they wait out a ~30 s search timeout.

use std::collections::HashMap;

use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind, DeviceWatcher};
use windows::Foundation::TypedEventHandler;
use windows::core::{GUID, HSTRING, Result};
use windows_collections::IIterable;

const CONNECTED: &str = "System.Devices.Connected:=System.StructuredQueryType.Boolean#True";

/// Container id of each connected device, mapped to its display name.
pub fn connected() -> Result<HashMap<GUID, String>> {
    let no_extra_properties: IIterable<HSTRING> = Vec::<HSTRING>::new().into();
    let found = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
        &HSTRING::from(CONNECTED),
        &no_extra_properties,
        DeviceInformationKind::DeviceContainer,
    )?
    .join()?;
    let mut devices = HashMap::new();
    for device in &found {
        // container ids look like "{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}"
        let id = device.Id()?.to_string();
        let container = GUID::try_from(id.trim_matches(|c| c == '{' || c == '}'))?;
        devices.insert(container, device.Name()?.to_string());
    }
    Ok(devices)
}

/// Calls `changed` (on a background thread) whenever a device connects or disconnects.
pub fn watch_connected(changed: impl Fn() + Send + Sync + Clone + 'static) -> Result<DeviceWatcher> {
    watch(CONNECTED, &[], DeviceInformationKind::DeviceContainer, changed)
}

/// Calls `changed` (on a background thread) when a matching object appears, disappears,
/// or one of `properties` changes. Call `Stop` on the returned watcher at exit.
pub fn watch(
    aqs: &str,
    properties: &[&str],
    kind: DeviceInformationKind,
    changed: impl Fn() + Send + Sync + Clone + 'static,
) -> Result<DeviceWatcher> {
    let properties: IIterable<HSTRING> = properties.iter().map(|p| HSTRING::from(*p)).collect::<Vec<_>>().into();
    let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(&HSTRING::from(aqs), &properties, kind)?;
    let c = changed.clone();
    watcher.Added(&TypedEventHandler::new(move |_, _| {
        c();
        Ok(())
    }))?;
    let c = changed.clone();
    watcher.Updated(&TypedEventHandler::new(move |_, _| {
        c();
        Ok(())
    }))?;
    watcher.Removed(&TypedEventHandler::new(move |_, _| {
        changed();
        Ok(())
    }))?;
    watcher.Start()?;
    Ok(watcher)
}
