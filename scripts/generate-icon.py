#!/usr/bin/env python3
"""Famulus (Agent) – App-Icon-Generator (Motiv: KI-Geist).
Phoenix-Stil via gemeinsamer Basis in famulus-icons/. Ersetzt den
Buchstaben "F" durch ein Geist-Symbol (passend zum Agenten).
"""
import os, sys
sys.path.insert(0, os.path.expanduser("~/KI Agenten/famulus-icons"))
from famulus_icon import icon, schreibe, S, PHOENIX_ORANGE
from motive import motiv_geist

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(ROOT, "swift-app", "Famulus", "Assets.xcassets",
                     "AppIcon.appiconset")


def main():
    ico = icon(motiv_geist)
    n = schreibe(ico, ICONS)
    print(f"Famulus-Icon (Geist) geschrieben ({n} Größen) nach {ICONS}")


if __name__ == "__main__":
    main()