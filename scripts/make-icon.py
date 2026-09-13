# Renders the Ebb logo into assets/: ebb.ico (16..256, embedded into the exe and
# used for the tray) and ebb.png (512, for the README).
#
#   python scripts/make-icon.py        (needs Pillow)
#
# Geometry is in a 140-unit square, matching assets/ebb.svg. Small sizes drop the
# text lines and the second wave: at 16-24 px they only turn into mush.
import math
import pathlib

from PIL import Image, ImageDraw

ROOT = pathlib.Path(__file__).resolve().parent.parent / "assets"
BG = (30, 33, 40, 255)
EDGE = (255, 255, 255, 30)
CARD = (52, 211, 153, 255)
INK = (11, 59, 43, 255)
WAVE = (45, 212, 191, 255)
WAVE_FAINT = (45, 212, 191, 115)
SS = 8  # supersampling


def wave(draw, k, y, width, color):
    pts = []
    for i in range(0, 97):
        x = 22 + i
        pts.append(((x) * k, (y - 5 * math.sin(math.pi * (x - 22) / 24)) * k))
    draw.line(pts, fill=color, width=round(width * k), joint="curve")
    r = width * k / 2
    for px, py in (pts[0], pts[-1]):
        draw.ellipse((px - r, py - r, px + r, py + r), fill=color)


def render(size):
    small = size <= 24
    n = size * SS
    k = n / 140
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((0, 0, n - 1, n - 1), radius=32 * k, fill=BG)
    if not small:
        d.rounded_rectangle((0.5 * k, 0.5 * k, n - 1 - 0.5 * k, n - 1 - 0.5 * k), radius=32 * k, outline=EDGE, width=max(1, round(1.2 * k)))
    if small:
        # Bigger card and one bold wave.
        d.rounded_rectangle((34 * k, 22 * k, 106 * k, 78 * k), radius=12 * k, fill=CARD)
        wave(d, k, 108, 13, WAVE)
    else:
        d.rounded_rectangle((42 * k, 26 * k, 98 * k, 70 * k), radius=10 * k, fill=CARD)
        d.rounded_rectangle((54 * k, 40 * k, 86 * k, 45 * k), radius=2.5 * k, fill=INK)
        d.rounded_rectangle((54 * k, 51 * k, 74 * k, 56 * k), radius=2.5 * k, fill=INK)
        wave(d, k, 90, 6, WAVE)
        wave(d, k, 110, 6, WAVE_FAINT)
    return img.resize((size, size), Image.Resampling.LANCZOS)


ROOT.mkdir(exist_ok=True)
sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
frames = [render(s) for s in sizes]
frames[-1].save(ROOT / "ebb.ico", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
render(512).save(ROOT / "ebb.png")
print("wrote", ROOT / "ebb.ico", ROOT / "ebb.png")
