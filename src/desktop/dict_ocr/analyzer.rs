use super::ColorSpec;

const QUANT_BITS: u8 = 5;
const BINS: usize = 1 << (QUANT_BITS * 3);

fn quantize(r: u8, g: u8, b: u8) -> usize {
    let ri = (r >> (8 - QUANT_BITS)) as usize;
    let gi = (g >> (8 - QUANT_BITS)) as usize;
    let bi = (b >> (8 - QUANT_BITS)) as usize;
    (ri << (QUANT_BITS * 2)) | (gi << QUANT_BITS) | bi
}

fn dequantize(idx: usize) -> (u8, u8, u8) {
    let step = 8u8;
    let mask = (1u8 << QUANT_BITS) - 1;
    let r = (((idx >> (QUANT_BITS * 2)) & mask as usize) as u8) * step + (step / 2);
    let g = (((idx >> QUANT_BITS) & mask as usize) as u8) * step + (step / 2);
    let b = ((idx & mask as usize) as u8) * step + (step / 2);
    (r, g, b)
}

pub struct ColorAnalysis {
    pub foreground: ColorSpec,
    pub background: ColorSpec,
    pub fg_pixels: u32,
    pub bg_pixels: u32,
}

pub fn analyze_region(pixels: &[u8], _width: u32, _height: u32) -> ColorAnalysis {
    let mut hist = vec![0u32; BINS];
    let total = pixels.len() / 3;

    for i in 0..total {
        let r = pixels[i * 3];
        let g = pixels[i * 3 + 1];
        let b = pixels[i * 3 + 2];
        let idx = quantize(r, g, b);
        hist[idx] += 1;
    }

    let mut ranked: Vec<(usize, u32)> = hist.iter().enumerate().map(|(i, &c)| (i, c)).collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.1));

    let top = ranked
        .iter()
        .filter(|(_, c)| *c as f64 > total as f64 * 0.01)
        .take(8)
        .collect::<Vec<_>>();

    let top_idx = top.first().map(|(i, _)| *i).unwrap_or(0);
    let (br, bg, bb) = dequantize(top_idx);

    let fg_idx = top
        .iter()
        .filter(|(i, _)| *i != top_idx)
        .max_by(|(_, ca), (_, cb)| ca.cmp(cb))
        .map(|(i, _)| *i)
        .unwrap_or(top_idx);

    let (fr, fg, fb) = dequantize(fg_idx);

    let bg_count = top.first().map(|(_, c)| *c).unwrap_or(0);
    let fg_count = if fg_idx == top_idx {
        0
    } else {
        top.iter()
            .find(|(i, _)| *i == fg_idx)
            .map(|(_, c)| *c)
            .unwrap_or(0)
    };

    let dr = if fr > br {
        fr.saturating_sub(br)
    } else {
        br.saturating_sub(fr)
    }
    .max(20);
    let dg = if fg > bg {
        fg.saturating_sub(bg)
    } else {
        bg.saturating_sub(fg)
    }
    .max(20);
    let db = if fb > bb {
        fb.saturating_sub(bb)
    } else {
        bb.saturating_sub(fb)
    }
    .max(20);

    ColorAnalysis {
        foreground: ColorSpec::new(fr, fg, fb, dr.min(0x77), dg.min(0x77), db.min(0x77)),
        background: ColorSpec::new(br, bg, bb, 0, 0, 0),
        fg_pixels: fg_count,
        bg_pixels: bg_count,
    }
}
