#!/usr/bin/env python3
# Famulus – App-Icon-Generator.
# Zeichnet das Famulus-„F" exakt im Stil des Famulus-Games-Icons:
# Braun→Schwarz-Verlauf, Orange #F86E27 in Menlo Bold, abgerundete Ecken.
# Der Verlauf wird 1:1 aus dem vorhandenen FG-Icon gesampelt, damit beide
# Icons garantiert identisch aussehen. Alles deterministisch, keine Quellen
# von außen außer Menlo.ttc und das FG-Referenz-Icon.

from PIL import Image, ImageDraw, ImageFont, ImageFilter
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(ROOT, "gui", "icons")
ANDROID = os.path.join(ICONS, "android")
IOS_SET = os.path.join(ROOT, "gui", "gen", "apple", "Assets.xcassets", "AppIcon.appiconset")
FG_REF = "/tmp/famulus-games-icon-1024.png"

S = 1024
ORANGE = (248, 110, 39)


def menlo(size):
    # index 1 = Menlo Bold
    return ImageFont.truetype("/System/Library/Fonts/Menlo.ttc", size, index=1)


def fg_gradient_lut():
    """Zeilenweise Farbe des FG-Icons samplen -> Lookup[y] = (r,g,b).
    Sampling-Spalten so gewählt, dass sie nie den FG-Schriftzug treffen
    (Text-BBox x 225..820, y 290..713)."""
    fg = Image.open(FG_REF).convert("RGB")
    lut = []
    for y in range(S):
        # Innerhalb der Text-Zeilen rechts daneben samplen, sonst Mitte.
        x = 940 if 280 <= y <= 723 else 512
        lut.append(fg.getpixel((x, y)))
    return lut


def verlauf_flach():
    """1024er-Verlauf exakt wie FG (als RGB)."""
    lut = fg_gradient_lut()
    img = Image.new("RGB", (S, S))
    px = img.load()
    for y in range(S):
        for x in range(S):
            px[x, y] = lut[y]
    return img


def buchstaben_maske(text, ziel_hoehe):
    """Buchstaben groß rendern, BBox ausschneiden, auf ziel_hoehe skalieren
    (Breite folgt proportional) und in 1024er-Koordinaten zentrieren."""
    # Fontgröße iterativ so bestimmen, dass die BBox-Höhe ziel_hoehe trifft.
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
    if not bb:
        return Image.new("L", (S, S), 0)
    aus = big.crop(bb)
    bw, bh = aus.size
    fakt = ziel_hoehe / bh
    neu_b = max(1, int(round(bw * fakt)))
    aus = aus.resize((neu_b, ziel_hoehe), Image.LANCZOS)
    ganz = Image.new("L", (S, S), 0)
    ganz.paste(aus, (S // 2 - neu_b // 2, S // 2 - ziel_hoehe // 2), aus)
    return ganz


def icon_macos():
    """Wie FG: abgerundete Ecken (Radius ≈ 214/1024), transparente Ecken."""
    radius = 214
    maske = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(maske)
    d.rounded_rectangle([0, 0, S - 1, S - 1], radius=radius, fill=255)
    maske = maske.filter(ImageFilter.GaussianBlur(1.2))
    f_maske = buchstaben_maske("F", 423)   # gleiche Höhe wie FG-Schriftzug
    img = verlauf_flach().convert("RGBA")
    img.paste(ORANGE, (0, 0), f_maske)
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(img, (0, 0), maske)
    return out


def icon_flach():
    """iOS/Android: volle Fläche, kein Alpha (Apple maskiert selbst)."""
    img = verlauf_flach().convert("RGBA")
    f_maske = buchstaben_maske("F", 423)
    img.paste(ORANGE, (0, 0), f_maske)
    return img


def speichern(img, pfad):
    os.makedirs(os.path.dirname(pfad), exist_ok=True)
    img.save(pfad)


def main():
    mac = icon_macos()
    flach = icon_flach()

    # ── Tauri/macOS-Icons (transparente Ecken wie FG) ──
    speichern(mac, os.path.join(ICONS, "icon.png"))
    for name, g in [("32x32.png", 32), ("64x64.png", 64),
                    ("128x128.png", 128), ("128x128@2x.png", 256),
                    ("256x256.png", 256)]:
        speichern(mac.resize((g, g), Image.LANCZOS), os.path.join(ICONS, name))

    # .icns über Apples iconutil
    iconset = os.path.join("/tmp", "famulus.iconset")
    os.makedirs(iconset, exist_ok=True)
    for name, g in [("icon_16x16.png", 16), ("icon_16x16@2x.png", 32),
                    ("icon_32x32.png", 32), ("icon_32x32@2x.png", 64),
                    ("icon_128x128.png", 128), ("icon_128x128@2x.png", 256),
                    ("icon_256x256.png", 256), ("icon_256x256@2x.png", 512),
                    ("icon_512x512.png", 512), ("icon_512x512@2x.png", 1024)]:
        speichern(mac.resize((g, g), Image.LANCZOS), os.path.join(iconset, name))
    os.system(f'iconutil -c icns "{iconset}" -o "{os.path.join(ICONS, "icon.icns")}"')

    # .ico (Windows) aus flacher Variante
    tmp_ico = "/tmp/famulus_ico.png"
    flach.convert("RGB").resize((256, 256)).save(tmp_ico)
    Image.open(tmp_ico).save(os.path.join(ICONS, "icon.ico"), format="ICO",
                             sizes=[(16, 16), (24, 24), (32, 32), (48, 48),
                                    (64, 64), (256, 256)])

    # ── Windows-Store-Kacheln (flach) ──
    for name, g in [("Square30x30Logo.png", 30), ("Square44x44Logo.png", 44),
                    ("Square71x71Logo.png", 71), ("Square89x89Logo.png", 89),
                    ("Square107x107Logo.png", 107), ("Square142x142Logo.png", 142),
                    ("Square150x150Logo.png", 150), ("Square284x284Logo.png", 284),
                    ("Square310x310Logo.png", 310), ("StoreLogo.png", 50)]:
        speichern(flach.resize((g, g), Image.LANCZOS), os.path.join(ICONS, name))

    # ── iOS-AppIcon (flach, Apple maskiert selbst) ──
    if os.path.isdir(IOS_SET):
        for datei in sorted(os.listdir(IOS_SET)):
            if not datei.endswith(".png"):
                continue
            pfad = os.path.join(IOS_SET, datei)
            ziel = Image.open(pfad).size
            speichern(flach.resize(ziel, Image.LANCZOS).convert("RGB").convert("RGBA"), pfad)

    # ── Android Adaptive Icon (flach, Braun-Hintergrund) ──
    for dichte, lanc in [("mdpi", 108), ("hdpi", 162), ("xhdpi", 216),
                         ("xxhdpi", 324), ("xxxhdpi", 432)]:
        ordner = os.path.join(ANDROID, f"mipmap-{dichte}")
        os.makedirs(ordner, exist_ok=True)
        for name in ["ic_launcher.png", "ic_launcher_round.png", "ic_launcher_foreground.png"]:
            speichern(flach.resize((lanc, lanc), Image.LANCZOS), os.path.join(ordner, name))
    with open(os.path.join(ANDROID, "values", "ic_launcher_background.xml"), "w") as f:
        f.write('<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
                '  <color name="ic_launcher_background">#211B16</color>\n</resources>\n')

    print("Icon-Generator fertig: macOS (.png/.icns/.ico + Kacheln), iOS, Android.")


if __name__ == "__main__":
    main()
