"""Nuphus icon generator 鈥?exact match of NuphusLogo.tsx 'n' letter path"""

from PIL import Image, ImageDraw
import os, struct, io

BASE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(BASE, "src-tauri", "icons")
PUBLIC = os.path.join(BASE, "frontend", "public")

BG = (18, 18, 26)
BORDER_RGBA = (255, 255, 255, 20)

# SVG 48x48 viewBox path: M12 34 L12 14 C12 6 36 6 36 14 L36 34
VB = 48
P0 = (12, 34)   # bottom-left
P1 = (12, 14)   # top-left
P2 = (12, 6)    # control left
P3 = (36, 6)    # control right
P4 = (36, 14)   # top-right
P5 = (36, 34)   # bottom-right


def cubic_bezier(p0, p1, p2, p3, steps=24):
    pts = []
    for i in range(steps + 1):
        t = i / steps
        u = 1 - t
        x = u**3 * p0[0] + 3 * u**2 * t * p1[0] + 3 * u * t**2 * p2[0] + t**3 * p3[0]
        y = u**3 * p0[1] + 3 * u**2 * t * p1[1] + 3 * u * t**2 * p2[1] + t**3 * p3[1]
        pts.append((x, y))
    return pts


def scale_pt(pt, size):
    return (pt[0] * size / VB, pt[1] * size / VB)


def draw_icon(size):
    """Draw the Nuphus 'n' logo — exact path from NuphusLogo.tsx, rendered directly"""
    from PIL import ImageDraw
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    s = size
    sw = max(5, round(s / 6))
    r = max(4, round(s * 0.22))

    # background rounded rect
    draw.rounded_rectangle((0, 0, s - 1, s - 1), radius=r, fill=(18, 18, 26))
    draw.rounded_rectangle((1, 1, s - 2, s - 2), radius=r - 1,
                           outline=BORDER_RGBA, width=max(1, s // 256))

    # "n" path: bottom-left → top-left → curve → top-right → bottom-right
    # draw as two straight segments + curve, without joint='curve'
    a = (P0[0] * s / VB, P0[1] * s / VB)
    b = (P1[0] * s / VB, P1[1] * s / VB)
    c = (P4[0] * s / VB, P4[1] * s / VB)
    d = (P5[0] * s / VB, P5[1] * s / VB)

    draw.line([a, b], fill=(245, 245, 250), width=sw)
    draw.line([c, d], fill=(245, 245, 250), width=sw)

    # top curve: cubic bezier P1 → P4
    curve = cubic_bezier(P1, P2, P3, P4, steps=max(32, s))
    curve_scaled = [(p[0] * s / VB, p[1] * s / VB) for p in curve]
    draw.line(curve_scaled, fill=(245, 245, 250), width=sw)

    return img

    return img


# 鈹€鈹€ ICO writer 鈹€鈹€
def make_ico(png_buffers):
    n = len(png_buffers)
    hdr = 6 + n * 16
    buf = io.BytesIO()
    buf.write(struct.pack('<HHH', 0, 1, n))
    off = hdr
    for png in png_buffers:
        w = min(png[16], 255)
        h = min(png[20], 255)
        buf.write(struct.pack('<BBBBHHII',
                              0 if w >= 256 else w,
                              0 if h >= 256 else h,
                              0, 0, 1, 32, len(png), off))
        off += len(png)
    for png in png_buffers:
        buf.write(png)
    return buf.getvalue()


# 鈹€鈹€ ICNS writer (macOS) 鈹€鈹€
def png_bytes(img):
    b = io.BytesIO()
    img.save(b, "PNG")
    return b.getvalue()

def make_icns(imgs):
    # imgs: dict {size: Image}
    entries = []
    type_map = {16: b'ic05', 32: b'ic06', 64: b'ic04',
                128: b'ic07', 256: b'ic08', 512: b'ic09',
                1024: b'ic10'}
    for sz, icon_type in sorted(type_map.items()):
        if sz in imgs:
            data = png_bytes(imgs[sz])
            entries.append(icon_type + struct.pack('>I', len(data) + 8) + data)
    body = b''.join(entries)
    return struct.pack('>I', 4 + 4 + len(body)) + b'icns' + body


# 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺?#  Generate
# 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺?
os.makedirs(ICONS, exist_ok=True)
os.makedirs(PUBLIC, exist_ok=True)

# Tauri PNGs + high-res source
print("Rendering icons...")
for s, name in [(32, "32x32"), (64, "64x64"), (128, "128x128"),
                (256, ""), (512, "icon")]:
    img = draw_icon(s)
    fn = f"{name}.png" if name else f"{s}x{s}.png"
    img.save(os.path.join(ICONS, fn), "PNG")
    print(f"  {s}x{s}")
    if s == 128:
        img256 = draw_icon(256)
        img256.save(os.path.join(ICONS, "128x128@2x.png"), "PNG")

# public
draw_icon(256).save(os.path.join(PUBLIC, "nuphus_256.png"), "PNG")
print("  public/nuphus_256.png")

# Windows Store
store = [(30, "Square30x30Logo"), (44, "Square44x44Logo"),
         (71, "Square71x71Logo"), (89, "Square89x89Logo"),
         (107, "Square107x107Logo"), (142, "Square142x142Logo"),
         (150, "Square150x150Logo"), (284, "Square284x284Logo"),
         (310, "Square310x310Logo")]
for s, n in store:
    draw_icon(s).save(os.path.join(ICONS, f"{n}.png"), "PNG")
print(f"  {len(store)} Store logos")

# Windows ICO (multi-size)
print("Building ICO...")
pngs = [draw_icon(s) for s in (16, 24, 32, 48, 64, 96, 128, 256)]
ico_bufs = []
for img in pngs:
    b = io.BytesIO()
    img.save(b, "PNG")
    ico_bufs.append(b.getvalue())
ico = make_ico(ico_bufs)
for path in [os.path.join(PUBLIC, "nuphus.ico"),
             os.path.join(ICONS, "icon.ico")]:
    with open(path, "wb") as f:
        f.write(ico)
print(f"  ICO: {len(ico)} bytes, {len(ico_bufs)} sizes")

# macOS ICNS
print("Building ICNS...")
icns = make_icns({16: draw_icon(16), 32: draw_icon(32), 64: draw_icon(64),
                  128: draw_icon(128), 256: draw_icon(256),
                  512: draw_icon(512), 1024: draw_icon(1024)})
with open(os.path.join(ICONS, "icon.icns"), "wb") as f:
    f.write(icns)
print(f"  ICNS: {len(icns)} bytes")

print("\n鉁?All icons regenerated")
