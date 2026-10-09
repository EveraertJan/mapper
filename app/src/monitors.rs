use rustmap_ui::MonitorInfo;
use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;

/// Key that identifies a monitor even when several share a model name
/// (the usual case with a row of identical projectors).
pub fn monitor_key(m: &MonitorHandle) -> String {
    let name = m.name().unwrap_or_else(|| "Unknown display".to_owned());
    let p = m.position();
    format!("{name} @ {},{}", p.x, p.y)
}

pub fn list(event_loop: &ActiveEventLoop) -> Vec<MonitorInfo> {
    event_loop
        .available_monitors()
        .map(|m| {
            let size = m.size();
            let pos = m.position();
            MonitorInfo {
                key: monitor_key(&m),
                name: m.name().unwrap_or_else(|| "Unknown display".to_owned()),
                size: [size.width, size.height],
                position: [pos.x, pos.y],
                refresh_hz: m.refresh_rate_millihertz().map(|mhz| mhz as f32 / 1000.0),
            }
        })
        .collect()
}

/// Finds a monitor by key; falls back to the first monitor with the same name
/// so a saved show still lands on the right projector if displays were rearranged.
pub fn find(event_loop: &ActiveEventLoop, key: &str) -> Option<MonitorHandle> {
    let monitors: Vec<_> = event_loop.available_monitors().collect();
    if let Some(m) = monitors.iter().find(|m| monitor_key(m) == key) {
        return Some(m.clone());
    }
    let name = key.rsplit_once(" @ ").map_or(key, |(name, _)| name);
    monitors
        .into_iter()
        .find(|m| m.name().as_deref() == Some(name))
}
