#!/usr/bin/env python3
# Famulus – App-Icon-Generator.
# Phoenix-Style, identisch mit Famulus Games (v0.2.5): dunkles Anthrazit
# #1e1e1e, weicher Orange-Fade oben (#7c2d12), Akzent #f97316 (Menlo Bold),
# abgerundete Ecken. Einziger Unterschied zu Famulus Games: „F" statt „FG".
# Deterministisch, keine externen Quellen außer Menlo.ttc.

from PIL import Image, ImageDraw, ImageFont, ImageFilter
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(ROOT, "swift-app", "Famulus", "Assets.xcassets",
                     "AppIcon.appiconset")

S = 1024
PHOENIX_ORANGE = (249, 115, 22)   # #f97316 – Akzent wie in der Hülle
GRUND = (30, 30, 30)              # #1e1e1e – Grundfläche
FADE = (124, 45, 18)              # #7c2d12 – Orange-Fade oben
FADE_ALPHA_MAX = 0.55             # Deckkraft des Fade ganz oben


def menlo(size):
    # index 1 = Menlo Bold
    return ImageFont.truetype("/System/Library/Fonts/Menlo.ttc", size, index=1)


def verlauf():
    """Zeilenweiser Verlauf: oben Orange-Fade über Anthrazit, unten Anthrazit."""
    img = Image.new("RGB", (S, S))
    px = img.load()
    for y in range(S):
        t = y / (S - 1)
        alpha = FADE_ALPHA_MAX * (1 - t)
        r = int(FADE[0] * alpha + GRUND[0] * (1 - alpha))
        g = int(FADE[1] * alpha + GRUND[1] * (1 - alpha))
        b = int(FADE[2] * alpha + GRUND[2] * (1 - alpha))
        for x in range(S):
            px[x, y] = (r, g, b)
    return img


def buchstaben_maske(text, ziel_hoehe):
    """Text groß rendern, BBox ausschneiden, auf Zielhöhe skalieren
    und in 1024er-Koordinaten zentrieren."""
    lo, hi, beste = 10, 2000, None
    for _ in range(30):
        mid = (lo + hi) // 2
        f = menlo(mid)
        d = ImageDraw.Draw(Image.new("L", (16, 16), 0))
        b = d.textbbox((0, 0), text, font=f)
        h = b[3] - b[1]
        if h < ziel_hoehe:
            lo = mid + 1
        else:
            hi = mid
        beste = (mid, h)
    size = beste[0]
    f = menlo(size)
    big = Image.new("L", (S * 2, S * 2), 0)
    d = ImageDraw.Draw(big)
    d.text((S, S), text, font=f, fill=255, anchor="mm")
    bb = big.getbbox()
    aus = big.crop(bb)
    bw, bh = aus.size
    fakt = ziel_hoehe / bh
    neu_b = max(1, int(round(bw * fakt)))
    aus = aus.resize((neu_b, ziel_hoehe), Image.LANCZOS)
    ganz = Image.new("L", (S, S), 0)
    ganz.paste(aus, (S // 2 - neu_b // 2, S // 2 - ziel_hoehe // 2), aus)
    return ganz


def icon():
    """Abgerundete Ecken (Radius ~214/1024, wie Famulus Games), transparente Ecken."""
    radius = 214
    maske = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(maske)
    d.rounded_rectangle([0, 0, S - 1, S - 1], radius=radius, fill=255)
    maske = maske.filter(ImageFilter.GaussianBlur(1.2))
    # Ziel-Höhe 423 statt 400 (wie bei "FG"): ein Solo-"F" hat bei gleicher
    # Fontgröße eine andere BBox-Höhe als "FG" - 423 gleicht das optisch an,
    # damit der Buchstabe gleich groß wirkt wie bei Famulus Games.
    f_maske = buchstaben_maske("F", 423)
    img = verlauf().convert("RGBA")
    img.paste(PHOENIX_ORANGE + (255,), (0, 0), f_maske)
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(img, (0, 0), maske)
    return out


def main():
    ico = icon()
    groessen = [("icon_16.png", 16), ("icon_16@2x.png", 32),
                ("icon_32.png", 32), ("icon_32@2x.png", 64),
                ("icon_128.png", 128), ("icon_128@2x.png", 256),
                ("icon_256.png", 256), ("icon_256@2x.png", 512),
                ("icon_512.png", 512), ("icon_512@2x.png", 1024)]
    for name, g in groessen:
        ico.resize((g, g), Image.LANCZOS).save(os.path.join(ICONS, name))
    print(f"Icon-Set geschrieben ({len(groessen)} Größen) nach {ICONS}")


if __name__ == "__main__":
    main()
