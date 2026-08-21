use super::CharTemplate;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DictStore {
    pub name: String,
    templates: HashMap<String, Vec<CharTemplate>>,
    path: PathBuf,
}

impl DictStore {
    pub fn new(name: &str, dir: &std::path::Path) -> Self {
        let path = dir.join(format!("{}.dict", name));
        Self {
            name: name.to_string(),
            templates: HashMap::new(),
            path,
        }
    }

    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let data = std::fs::read(path)?;
        let dicts: HashMap<String, Vec<CharTemplate>> = bincode::deserialize(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(Self {
            name,
            templates: dicts,
            path: path.to_path_buf(),
        })
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = bincode::serialize(&self.templates).map_err(std::io::Error::other)?;
        std::fs::write(&self.path, data)
    }

    pub fn add(&mut self, char: &str, templates: Vec<CharTemplate>) {
        self.templates.insert(char.to_string(), templates);
    }

    pub fn get(&self, char: &str) -> Option<&Vec<CharTemplate>> {
        self.templates.get(char)
    }

    pub fn all(&self) -> &HashMap<String, Vec<CharTemplate>> {
        &self.templates
    }

    pub fn remove(&mut self, char: &str) {
        self.templates.remove(char);
    }

    pub fn clear(&mut self) {
        self.templates.clear();
    }

    pub fn len(&self) -> usize {
        self.templates.len()
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

pub fn list_dicts(dir: &std::path::Path) -> std::io::Result<Vec<String>> {
    let mut dicts = Vec::new();
    if !dir.exists() {
        return Ok(dicts);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("dict") {
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                dicts.push(name.to_string());
            }
        }
    }
    Ok(dicts)
}

pub fn export_dict_text(dict: &DictStore) -> String {
    let mut lines = Vec::new();
    for (ch, templates) in dict.all() {
        for t in templates {
            lines.push(format!(
                "{} {}x{} {}",
                ch,
                t.width,
                t.height,
                encode_data_text(&t.data, t.width, t.height)
            ));
        }
    }
    lines.join("\n")
}

pub fn parse_dict_text(text: &str) -> HashMap<String, Vec<CharTemplate>> {
    let mut map: HashMap<String, Vec<CharTemplate>> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        if parts.len() < 4 {
            continue;
        }
        let ch = parts[0].to_string();
        let dims: Vec<&str> = parts[1].split('x').collect();
        if dims.len() != 2 {
            continue;
        }
        let Ok(w) = dims[0].parse::<u8>() else {
            continue;
        };
        let Ok(h) = dims[1].parse::<u8>() else {
            continue;
        };
        let data = decode_data_text(parts[3], w, h);
        let t = CharTemplate {
            char: ch.clone(),
            width: w,
            height: h,
            data,
            grayscale: vec![],
        };
        map.entry(ch).or_default().push(t);
    }
    map
}

fn encode_data_text(data: &[u8], w: u8, h: u8) -> String {
    let cols = w.div_ceil(8);
    let mut s = String::with_capacity((cols * h * 2) as usize);
    for i in 0..(cols as usize * h as usize) {
        if i < data.len() {
            s.push_str(&format!("{:02X}", data[i]));
        }
    }
    s
}

/// 解析字典格式单行：件：0807FF00...$件$0.0.41$12
pub fn parse_compact_line(line: &str) -> Option<CharTemplate> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // 分隔符：支持全角：或半角:
    let sep_idx = line.find(['：', ':'])?;
    let ch = line[..sep_idx].trim().to_string();
    if ch.is_empty() {
        return None;
    }

    let rest = &line[sep_idx + 1..];

    // 提取 $ 之前的 hex 数据
    let hex_end = rest.find('$').unwrap_or(rest.len());
    let hex_data = &rest[..hex_end];
    if hex_data.len() < 4 {
        return None;
    }

    // 前两个 hex byte = width, height
    let w = u8::from_str_radix(&hex_data[..2], 16).ok()?;
    let h = u8::from_str_radix(&hex_data[2..4], 16).ok()?;
    if w == 0 || h == 0 {
        return None;
    }

    // 剩余 hex = dot matrix data
    let data_hex = &hex_data[4..];
    let data = decode_data_text(data_hex, w, h);

    Some(CharTemplate {
        char: ch,
        width: w,
        height: h,
        data,
        grayscale: vec![],
    })
}

/// 导出为字典格式文本
pub fn export_compact_text(dict: &DictStore) -> String {
    let mut lines = Vec::new();
    for (ch, templates) in dict.all() {
        for t in templates {
            let cols = (t.width as u32).div_ceil(8) as usize;
            let hex = (0..cols * t.height as usize)
                .map(|i| {
                    if i < t.data.len() {
                        format!("{:02X}", t.data[i])
                    } else {
                        "00".to_string()
                    }
                })
                .collect::<String>();
            lines.push(format!(
                "{}：{:02X}{:02X}{}${}${}",
                ch, t.width, t.height, hex, ch, t.width
            ));
        }
    }
    lines.join("\n")
}

/// 批量解析字典格式文本
pub fn parse_compact_text(text: &str) -> HashMap<String, Vec<CharTemplate>> {
    let mut map: HashMap<String, Vec<CharTemplate>> = HashMap::new();
    for line in text.lines() {
        if let Some(t) = parse_compact_line(line) {
            map.entry(t.char.clone()).or_default().push(t);
        }
    }
    map
}

fn decode_data_text(s: &str, w: u8, h: u8) -> Vec<u8> {
    let cols = (w as u32).div_ceil(8) as usize;
    let expect = cols * h as usize;
    let bytes: Vec<u8> = (0..s.len().saturating_sub(1))
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    if bytes.len() >= expect {
        bytes[..expect].to_vec()
    } else {
        let mut padded = vec![0u8; expect];
        padded[..bytes.len()].copy_from_slice(&bytes);
        padded
    }
}
