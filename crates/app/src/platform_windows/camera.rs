// Enumerate the same provider objects used to create capture elements. Stable
// device-path keys keep identically named USB/virtual cameras distinguishable.
fn windows_devices() -> Vec<(String, gst::Device)> {
    if gst::init().is_err() {
        return vec![];
    }
    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Video/Source"), None);
    if monitor.start().is_err() {
        return vec![];
    }
    let devices = monitor
        .devices()
        .into_iter()
        .filter_map(|d| {
            let element = d.create_element(None).ok()?;
            let path = ["device-path", "device"].iter().find_map(|key| {
                element
                    .find_property(key)
                    .and_then(|_| element.property_value(key).get::<String>().ok())
                    .filter(|s| !s.is_empty())
            })?;
            Some((path, d))
        })
        .collect();
    monitor.stop();
    devices
}
pub fn devices() -> Vec<Device> {
    windows_devices()
        .into_iter()
        .map(|(path, d)| Device {
            name: d.display_name().to_string(),
            path,
        })
        .collect()
}
fn camera_source(path: &str) -> Result<gst::Element> {
    windows_devices()
        .into_iter()
        .find(|(p, _)| p == path)
        .context("Camera disconnected. Refresh cameras and select the device again.")?
        .1
        .create_element(None)
        .context("Cannot create Windows camera source")
}
fn loopback_mode(_: &str) -> Result<Option<DeviceMode>> {
    Ok(None)
}
