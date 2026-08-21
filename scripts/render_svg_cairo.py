"""Render Nuphus icon SVG using pycairo (native cairo rendering)"""

import cairo, os, struct, io, math
from PIL import Image

BASE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(BASE, "src-tauri", "icons")
PUBLIC = os.path.join(BASE, "frontend", "public")

VB = 48  # viewBox
COLOR_BG = (18/255, 18/255, 26/255)       # #12121a
COLOR_STROKE = (245/255, 245/255, 250/255)  # #f5f5fa


def draw_icon(size, surface_cls=cairo.ImageSurface, format=cairo.FORMAT_ARGB32):
    """Render Nuphus 'n' logo at exact pixel size using cairo"""
    surface = surface_cls(format, size, size)
    ctx = cairo.Context(surface)

    # Scale to viewBox
    ctx.scale(size / VB, size / VB)

    # --- background rounded rect ---
    r = 10.5
    π = math.pi
    ctx.new_sub_path()
    ctx.arc(r, r, r, -π, -π / 2)
    ctx.arc(VB - r, r, r, -π / 2, 0)
    ctx.arc(VB - r, VB - r, r, 0, π / 2)
    ctx.arc(r, VB - r, r, π / 2, π)
    ctx.close_path()

    # fill bg
    ctx.set_source_rgb(*COLOR_BG)
    ctx.fill_preserve()
    # thin border
    ctx.set_source_rgba(1, 1, 1, 0.08)
    ctx.set_line_width(0.5)
    ctx.stroke()

    # --- "n" letter path ---
    ctx.new_path()
    ctx.move_to(12, 34)
    ctx.line_to(12, 14)
    ctx.curve_to(12, 6, 36, 6, 36, 14)
    ctx.line_to(36, 34)

    ctx.set_source_rgb(*COLOR_STROKE)
    ctx.set_line_width(8)
    ctx.set_line_cap(cairo.LINE_CAP_ROUND)
    ctx.set_line_join(cairo.LINE_JOIN_ROUND)
    ctx.stroke()

    return surface


def surface_to_png(surface, path):
    surface.write_to_png(path)


def surface_to_pil(surface):
    """Convert cairo surface to PIL Image"""
    w, h = surface.get_width(), surface.get_height()
    data = surface.get_data()
    img = Image.frombuffer("RGBA", (w, h),
                           data, "raw", "BGRA", 0, 1)
    return img


def make_ico(imgs):
    """Create multi-frame ICO from PIL Images"""
    n = len(imgs)
    hdr = 6 + n * 16
    buf = io.BytesIO()
    buf.write(struct.pack('<HHH', 0, 1, n))
    off = hdr
    bufs = []
    for img in imgs:
        b = io.BytesIO()
        img.save(b, "PNG")
        png = b.getvalue()
        bufs.append(png)
        w = min(png[16], 255)
        h = min(png[20], 255)
        buf.write(struct.pack('<BBBBHHII',
                              0 if w >= 256 else w,
                              0 if h >= 256 else h,
                              0, 0, 1, 32, len(png), off))
        off += len(png)
    for png in bufs:
        buf.write(png)
    return buf.getvalue()


def make_icns(imgs_dict):
    entries = []
    type_map = {16: b'ic05', 32: b'ic06', 64: b'ic04',
                128: b'ic07', 256: b'ic08', 512: b'ic09',
                1024: b'ic10'}
    for sz, icon_type in sorted(type_map.items()):
        if sz in imgs_dict:
            b = io.BytesIO()
            imgs_dict[sz].save(b, "PNG")
            data = b.getvalue()
            entries.append(icon_type + struct.pack('>I', len(data) + 8) + data)
    body = b''.join(entries)
    return struct.pack('>I', 4 + 4 + len(body)) + b'icns' + body


# ════════════════════════════════════════
#  Generate all icons
# ════════════════════════════════════════

os.makedirs(ICONS, exist_ok=True)
os.makedirs(PUBLIC, exist_ok=True)

print("Rendering icons with cairo...")

# Generate all sizes using cairo
rendered = {}
for s in [16, 24, 30, 32, 44, 48, 64, 71, 89, 96, 107, 128, 142, 150, 256, 284, 310, 512, 1024]:
    surface = draw_icon(s)
    rendered[s] = surface_to_pil(surface)

# Tauri PNGs
for s in [32, 64, 128, 256]:
    rendered[s].save(os.path.join(ICONS, f"{s}x{s}.png"), "PNG")
rendered[128].save(os.path.join(ICONS, "128x128@2x.png"), "PNG")  # will be overwritten if 256 exists
rendered[256].save(os.path.join(ICONS, "128x128@2x.png"), "PNG")
rendered[512].save(os.path.join(ICONS, "icon.png"), "PNG")
print("  Tauri PNGs OK")

# public
rendered[256].save(os.path.join(PUBLIC, "nuphus_256.png"), "PNG")
print("  public/nuphus_256.png OK")

# Store logos
store = [(30, "Square30x30Logo"), (44, "Square44x44Logo"),
         (71, "Square71x71Logo"), (89, "Square89x89Logo"),
         (107, "Square107x107Logo"), (142, "Square142x142Logo"),
         (150, "Square150x150Logo"), (284, "Square284x284Logo"),
         (310, "Square310x310Logo")]
for s, n in store:
    rendered[s].save(os.path.join(ICONS, f"{n}.png"), "PNG")
print(f"  {len(store)} Store logos OK")

# ICO (multi-size)
ico_imgs = [rendered[s] for s in [16, 24, 32, 48, 64, 96, 128, 256]]
ico = make_ico(ico_imgs)
for path in [os.path.join(PUBLIC, "nuphus.ico"),
             os.path.join(ICONS, "icon.ico")]:
    with open(path, "wb") as f:
        f.write(ico)
print(f"  ICO: {len(ico)} bytes ({len(ico_imgs)} sizes)")

# ICNS
icns = make_icns({s: rendered[s] for s in [16, 32, 64, 128, 256, 512, 1024]})
with open(os.path.join(ICONS, "icon.icns"), "wb") as f:
    f.write(icns)
print(f"  ICNS: {len(icns)} bytes")

print("\nDone - All icons rendered with cairo vector engine")
