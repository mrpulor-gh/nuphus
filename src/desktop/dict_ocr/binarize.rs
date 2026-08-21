use super::ColorSpec;

pub fn binarize(pixels: &[u8], width: u32, height: u32, fg: &ColorSpec) -> Vec<u8> {
    let total = (width * height) as usize;
    let mut out = vec![0u8; total];

    for i in 0..total {
        let r = pixels[i * 3];
        let g = pixels[i * 3 + 1];
        let b = pixels[i * 3 + 2];
        out[i] = if fg.matches(r, g, b) { 1 } else { 0 };
    }

    out
}

pub fn binarize_to_rgba(pixels: &[u8], width: u32, height: u32, fg: &ColorSpec) -> Vec<u8> {
    let total = (width * height) as usize;
    let mut out = vec![0u8; total * 4];

    for i in 0..total {
        let r = pixels[i * 3];
        let g = pixels[i * 3 + 1];
        let b = pixels[i * 3 + 2];
        let is_fg = fg.matches(r, g, b);
        let idx = i * 4;
        out[idx] = if is_fg { r } else { 0 };
        out[idx + 1] = if is_fg { g } else { 0 };
        out[idx + 2] = if is_fg { b } else { 0 };
        out[idx + 3] = if is_fg { 255 } else { 30 };
    }

    out
}
