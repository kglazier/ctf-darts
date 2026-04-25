"""Render Space Boosters launcher icons into the Android mipmap folders.

- ic_launcher.png  — full icon (background + ship), used pre-API 26.
- ic_launcher_foreground.png — ship + flame only, used by adaptive icons.
"""
from svglib.svglib import svg2rlg
from reportlab.graphics import renderPM
import os

PROJECT_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
RES_DIR = os.path.join(PROJECT_ROOT, "android", "app", "src", "main", "res")
SOURCE_FULL = os.path.join(PROJECT_ROOT, "assets", "icon.svg")
SOURCE_FG = os.path.join(PROJECT_ROOT, "assets", "icon-foreground.svg")

# Per-density legacy + adaptive sizes (px).
DENSITIES = {
    "mipmap-mdpi":    (48,  108),
    "mipmap-hdpi":    (72,  162),
    "mipmap-xhdpi":   (96,  216),
    "mipmap-xxhdpi":  (144, 324),
    "mipmap-xxxhdpi": (192, 432),
}


def render(svg_path: str, out_path: str, size: int) -> None:
    drawing = svg2rlg(svg_path)
    scale = size / 512.0
    drawing.width = size
    drawing.height = size
    drawing.scale(scale, scale)
    renderPM.drawToFile(drawing, out_path, fmt="PNG")
    print(f"  wrote {out_path}")


def main() -> None:
    for density, (legacy_size, fg_size) in DENSITIES.items():
        d = os.path.join(RES_DIR, density)
        os.makedirs(d, exist_ok=True)
        render(SOURCE_FULL, os.path.join(d, "ic_launcher.png"), legacy_size)
        render(SOURCE_FG, os.path.join(d, "ic_launcher_foreground.png"), fg_size)


if __name__ == "__main__":
    main()
