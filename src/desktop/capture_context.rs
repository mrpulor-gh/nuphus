//! Process-local screenshot provenance and image-to-desktop coordinate conversion.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

/// The units accepted by the native pointer backend, not the image pixel grid.
pub const fn screen_coordinate_units() -> &'static str {
    if cfg!(target_os = "macos") {
        "logical_points"
    } else {
        "physical_pixels"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Bounds {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        i64::from(x) >= i64::from(self.x)
            && i64::from(y) >= i64::from(self.y)
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WindowSnapshot {
    pub hwnd: i32,
    pub process_id: u64,
    pub title: String,
    pub bounds: Bounds,
}
impl WindowSnapshot {
    pub fn from_info(hwnd: i32, value: &Value) -> Result<Self, String> {
        let value = &value["result"];
        if value["minimized"].as_bool() == Some(true) || value["visible"].as_bool() == Some(false) {
            return Err("目标窗口当前不可见，请刷新观察".into());
        }
        let bounds: Bounds =
            serde_json::from_value(value["window"].clone()).map_err(|_| "窗口几何信息无效")?;
        if bounds.width == 0 || bounds.height == 0 {
            return Err("窗口大小无效".into());
        }
        Ok(Self {
            hwnd,
            process_id: value["process_id"].as_u64().ok_or("窗口进程信息缺失")?,
            title: value["title"].as_str().unwrap_or_default().into(),
            bounds,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptureContext {
    pub capture_id: String,
    pub coordinate_space: &'static str,
    pub coordinate_units: &'static str,
    pub scope: &'static str,
    pub screen_bounds: Bounds,
    pub image_width: u32,
    pub image_height: u32,
    pub target: Option<WindowSnapshot>,
    pub captured_at: String,
    #[serde(skip)]
    path: PathBuf,
    #[serde(skip)]
    fingerprint: u64,
    #[serde(skip)]
    elements: HashMap<u32, (i32, i32)>,
    #[serde(skip)]
    consumed: bool,
}
impl CaptureContext {
    pub fn new(
        path: &Path,
        bounds: Bounds,
        dimensions: (u32, u32),
        scope: &'static str,
        target: Option<WindowSnapshot>,
    ) -> Result<Self, String> {
        if bounds.width == 0 || bounds.height == 0 || dimensions.0 == 0 || dimensions.1 == 0 {
            return Err("截图或屏幕区域大小无效".into());
        }
        Ok(Self {
            capture_id: uuid::Uuid::new_v4().to_string(),
            coordinate_space: "screen",
            coordinate_units: screen_coordinate_units(),
            scope,
            screen_bounds: bounds,
            image_width: dimensions.0,
            image_height: dimensions.1,
            target,
            captured_at: chrono::Utc::now().to_rfc3339(),
            path: canonical(path)?,
            fingerprint: fingerprint(path)?,
            elements: HashMap::new(),
            consumed: false,
        })
    }
    pub fn screen_point(&self, x: i32, y: i32) -> Result<(i32, i32), String> {
        if self.image_width == 0
            || self.image_height == 0
            || x < 0
            || y < 0
            || x as u32 >= self.image_width
            || y as u32 >= self.image_height
        {
            return Err("元素中心不在捕获图片内".into());
        }
        // Integer arithmetic preserves negative monitor origins and avoids model-side DPI math.
        let sx = i64::from(self.screen_bounds.x)
            + i64::from(x) * i64::from(self.screen_bounds.width) / i64::from(self.image_width);
        let sy = i64::from(self.screen_bounds.y)
            + i64::from(y) * i64::from(self.screen_bounds.height) / i64::from(self.image_height);
        Ok((
            i32::try_from(sx).map_err(|_| "屏幕坐标溢出")?,
            i32::try_from(sy).map_err(|_| "屏幕坐标溢出")?,
        ))
    }
    fn usable(&self) -> Result<(), String> {
        if self.consumed {
            Err("捕获已过期或已执行，请重新截图并感知".into())
        } else {
            Ok(())
        }
    }
    pub fn validate_window(&self, current: Option<&WindowSnapshot>) -> Result<(), String> {
        self.usable()?;
        if self.target.as_ref() != current {
            return Err("目标窗口身份或位置已变化，请重新截图并感知；未发送点击".into());
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct CaptureCache {
    records: VecDeque<CaptureContext>,
}
impl CaptureCache {
    pub fn validate(&self, id: &str) -> Result<(), String> {
        let context = self
            .records
            .iter()
            .find(|item| item.capture_id == id)
            .ok_or("捕获缓存已过期，请重新截图并感知")?;
        context.usable()?;
        if context.fingerprint != fingerprint(&context.path)? {
            return Err("截图文件已变更，请重新截图并感知".into());
        }
        Ok(())
    }
    pub fn invalidate(&mut self) {
        for record in &mut self.records {
            record.consumed = true;
        }
    }
    pub fn insert(&mut self, context: CaptureContext) {
        self.records.retain(|item| item.path != context.path);
        self.records.push_back(context);
        while self.records.len() > 64 {
            self.records.pop_front();
        }
    }
    pub fn for_path(&self, path: &Path) -> Result<Option<CaptureContext>, String> {
        let path = canonical(path)?;
        let Some(context) = self.records.iter().find(|item| item.path == path) else {
            return Ok(None);
        };
        context.usable()?;
        if context.fingerprint != fingerprint(&path)? {
            return Err("截图文件已变更，请重新截图并感知".into());
        }
        Ok(Some(context.clone()))
    }
    pub fn set_elements(
        &mut self,
        id: &str,
        elements: impl Iterator<Item = (u32, i32, i32)>,
    ) -> Result<(), String> {
        let item = self
            .records
            .iter_mut()
            .find(|item| item.capture_id == id)
            .ok_or("捕获缓存已过期")?;
        item.usable()?;
        item.elements = elements
            .map(|(id, x, y)| item.screen_point(x, y).map(|point| (id, point)))
            .collect::<Result<_, _>>()?;
        Ok(())
    }
    pub fn resolve(&self, id: &str, element: u32) -> Result<(CaptureContext, (i32, i32)), String> {
        let context = self
            .records
            .iter()
            .find(|item| item.capture_id == id)
            .ok_or("未知捕获，请重新截图并感知")?;
        context.usable()?;
        let point = *context
            .elements
            .get(&element)
            .ok_or("元素不属于该捕获，请使用本次感知返回的 element_id")?;
        Ok((context.clone(), point))
    }
    pub fn consume(&mut self, id: &str) -> Result<(), String> {
        let item = self
            .records
            .iter()
            .find(|item| item.capture_id == id)
            .ok_or("未知捕获")?;
        item.usable()?;
        let target = item
            .target
            .as_ref()
            .map(|target| (target.hwnd, target.process_id));
        for record in &mut self.records {
            if target.is_none()
                || record
                    .target
                    .as_ref()
                    .map(|target| (target.hwnd, target.process_id))
                    == target
            {
                record.consumed = true;
            }
        }
        Ok(())
    }
}
fn canonical(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|e| format!("无法读取截图路径: {e}"))
}
fn fingerprint(path: &Path) -> Result<u64, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("无法读取截图: {e}"))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> CaptureContext {
        CaptureContext {
            capture_id: "capture".into(),
            coordinate_space: "screen",
            coordinate_units: screen_coordinate_units(),
            scope: "window",
            screen_bounds: Bounds {
                x: -1000,
                y: 50,
                width: 800,
                height: 600,
            },
            image_width: 1600,
            image_height: 1200,
            target: None,
            captured_at: String::new(),
            path: PathBuf::new(),
            fingerprint: 0,
            elements: HashMap::new(),
            consumed: false,
        }
    }
    #[test]
    fn converts_retina_and_negative_origin_without_model_math() {
        let capture = context();
        assert_eq!(capture.screen_point(800, 400).unwrap(), (-600, 250));
        assert!(capture.screen_point(1600, 0).is_err());
        assert!(capture.screen_point(-1, 0).is_err());
    }
    #[test]
    fn moved_or_replaced_window_requires_fresh_observation() {
        let window = WindowSnapshot {
            hwnd: 3,
            process_id: 2,
            title: "A".into(),
            bounds: context().screen_bounds,
        };
        let mut capture = context();
        capture.target = Some(window.clone());
        assert!(capture.validate_window(Some(&window)).is_ok());
        let mut moved = window.clone();
        moved.bounds.x += 1;
        assert!(capture.validate_window(Some(&moved)).is_err());
        let mut replaced = window;
        replaced.process_id += 1;
        assert!(capture.validate_window(Some(&replaced)).is_err());
    }
    #[test]
    fn candidate_mapping_is_scoped_and_click_cannot_be_replayed() {
        let mut cache = CaptureCache::default();
        cache.insert(context());
        cache
            .set_elements("capture", [(4, 800, 400)].into_iter())
            .unwrap();
        assert_eq!(cache.resolve("capture", 4).unwrap().1, (-600, 250));
        assert!(cache.resolve("capture", 5).is_err());
        assert!(cache.resolve("other", 4).is_err());
        cache.consume("capture").unwrap();
        assert!(cache.resolve("capture", 4).is_err());
        assert!(cache.consume("capture").is_err());
        assert!(cache.validate("capture").is_err());
    }
    #[test]
    fn raw_input_invalidates_previously_observed_elements() {
        let mut cache = CaptureCache::default();
        cache.insert(context());
        cache
            .set_elements("capture", [(4, 800, 400)].into_iter())
            .unwrap();
        cache.invalidate();
        assert!(cache.resolve("capture", 4).is_err());
    }
    #[test]
    fn capture_metadata_names_native_screen_units() {
        let value = serde_json::to_value(context()).unwrap();
        assert_eq!(value["coordinate_space"], "screen");
        assert_eq!(value["coordinate_units"], screen_coordinate_units());
        assert!(value.get("path").is_none());
        assert!(value.get("fingerprint").is_none());
    }
    #[test]
    fn replacing_a_capture_path_retires_the_previous_capture_id() {
        let mut cache = CaptureCache::default();
        cache.insert(context());
        cache
            .set_elements("capture", [(4, 800, 400)].into_iter())
            .unwrap();
        let mut newer = context();
        newer.capture_id = "newer".into();
        cache.insert(newer);
        assert!(cache.resolve("capture", 4).is_err());
        assert!(cache.resolve("newer", 4).is_err());
        cache
            .set_elements("newer", [(4, 800, 400)].into_iter())
            .unwrap();
        assert_eq!(cache.resolve("newer", 4).unwrap().1, (-600, 250));
    }
    #[test]
    fn bounds_exclude_right_and_bottom_edge() {
        let bounds = context().screen_bounds;
        assert!(bounds.contains(-1000, 50));
        assert!(!bounds.contains(-200, 50));
        assert!(!bounds.contains(-500, 650));
    }
}
