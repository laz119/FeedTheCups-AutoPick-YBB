//! 队列预设：本地 JSON，键是预设名，值是原始队列文本。

use std::collections::BTreeMap;

pub type PresetMap = BTreeMap<String, String>;

pub fn load() -> PresetMap {
    std::fs::read_to_string("presets.json")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(map: &PresetMap) {
    if let Ok(text) = serde_json::to_string_pretty(map) {
        let _ = std::fs::write("presets.json", text);
    }
}
