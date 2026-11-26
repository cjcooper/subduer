#!/usr/bin/env python3
"""Generates the app icon sources: a light and a dark version.

  icon.svg        light, the default everywhere (Windows .exe, Linux, macOS before 26)
  icon-dark.svg   dark, for the window icon and About window when the app is dark
  AppIcon.icon/   Icon Composer document for macOS 26+, which shows the light or
                  dark version (and the clear and tinted ones) as the system asks
  icon-festive.svg, icon-festive-dark.svg
                  the holiday versions (festive.rs): the Dock shows them on macOS
                  in December, and the About window too

Regenerate the icons with:
  python3 src-tauri/icons/make_icon.py src-tauri/icons
  rsvg-convert -w 1024 -h 1024 src-tauri/icons/icon.svg -o src-tauri/icons/app-icon.png
  cargo tauri icon src-tauri/icons/app-icon.png -o src-tauri/icons
  rsvg-convert -w 256 -h 256 src-tauri/icons/icon.svg -o src-tauri/icons/256x256.png
  rsvg-convert -w 48 -h 48 src-tauri/icons/icon.svg -o src-tauri/icons/48x48.png
  rsvg-convert -w 256 -h 256 src-tauri/icons/icon-dark.svg -o src-tauri/icons/dark-256x256.png
  rsvg-convert -w 512 -h 512 src-tauri/icons/icon-festive.svg -o src-tauri/icons/festive-512.png
  rsvg-convert -w 512 -h 512 src-tauri/icons/icon-festive-dark.svg -o src-tauri/icons/festive-dark-512.png

Preview the macOS versions with Icon Composer's ictool (Xcode 26+), e.g.:
  "$(xcode-select -p)/../Applications/Icon Composer.app/Contents/Executables/ictool" \\
    src-tauri/icons/AppIcon.icon --export-image --output-file dark.png --platform macOS \\
    --rendition Dark --width 1024 --height 1024 --scale 1

Design: a 7-spoke helm (the Kubernetes motif) set in a control dial; the amber
handle is the pointer and the amber arc shows how far the dial is turned. Light
is a blue helm on a white tile, dark a light-blue helm on a charcoal tile.
The holiday versions are a green helm with a red pointer, a Santa hat on the
ring and snow on the tile.
"""
import json, math, os, sys
C = 512

START, SWEEP, LEVEL = 135.0, 270.0, 0.62      # dial: gap at the bottom, 62% turned
POINTER = START + SWEEP*LEVEL

# Per version: tile gradient (top, bottom), helm, amber, centre pin, and the
# colour and opacity of the faint heptagon and the dial track.
PALETTES = {
    "light": dict(bg=("#ffffff", "#dce5f2"), helm="#1f5bd6", amber="#f29d0c", pin="#ffffff",
                  hept=("#1f5bd6", 0.08), track=("#1f5bd6", 0.16), edge=0.10),
    "dark": dict(bg=("#2b3445", "#0d1119"), helm="#6fa8ff", amber="#ffb21e", pin="#141a25",
                 hept=("#6fa8ff", 0.10), track=("#ffffff", 0.14), edge=0.0),
}

# The holiday versions: the same shapes in green and red, plus snow and a hat.
FESTIVE = {
    "light": dict(PALETTES["light"], bg=("#ffffff", "#dbe8f0"), helm="#17804a", amber="#d62b33",
                  hept=("#17804a", 0.08), track=("#17804a", 0.16), snow=("#9fbad6", 0.55)),
    "dark": dict(PALETTES["dark"], bg=("#22344a", "#0b121c"), helm="#4cc47e", amber="#ff4f58",
                 hept=("#4cc47e", 0.10), snow=("#ffffff", 0.55)),
}

def pt(r, deg):
    a = math.radians(deg)
    return C + r*math.cos(a), C + r*math.sin(a)

def arc(r, a0, a1):
    x0, y0 = pt(r, a0); x1, y1 = pt(r, a1)
    large = 1 if (a1-a0) > 180 else 0
    return f"M{x0:.1f},{y0:.1f} A{r},{r} 0 {large} 1 {x1:.1f},{y1:.1f}"

def dial(p):
    """The back layer: faint heptagon (a nod to the Kubernetes mark) and the dial."""
    hept = [pt(300, -90 + i*360/7) for i in range(7)]
    d = "M" + " L".join(f"{x:.1f},{y:.1f}" for x, y in hept) + " Z"
    hc, ho = p["hept"]; tc, to = p["track"]
    return [
        f'<g opacity="{ho}"><path d="{d}" fill="{hc}" stroke="{hc}" stroke-width="60" stroke-linejoin="round"/></g>',
        f'<path d="{arc(352, START, START+SWEEP)}" fill="none" stroke="{tc}" stroke-opacity="{to}" stroke-width="22" stroke-linecap="round"/>',
        f'<path d="{arc(352, START, POINTER)}" fill="none" stroke="{p["amber"]}" stroke-width="22" stroke-linecap="round"/>',
    ]

def helm(p):
    """The front layer: ring, 7 spokes with handles (one is the amber pointer), hub."""
    r_ring, r_hub, r_handle = 205, 64, 268
    parts = []
    for i in range(7):
        a = POINTER + i*360/7
        x0, y0 = pt(r_hub-10, a); x1, y1 = pt(r_handle, a)
        color = p["amber"] if i == 0 else p["helm"]
        parts.append(f'<line x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" stroke="{color}" stroke-width="{42 if i == 0 else 38}" stroke-linecap="round"/>')
    parts.append(f'<circle cx="{C}" cy="{C}" r="{r_ring}" fill="none" stroke="{p["helm"]}" stroke-width="44"/>')
    parts.append(f'<circle cx="{C}" cy="{C}" r="{r_hub}" fill="{p["helm"]}"/>')
    parts.append(f'<circle cx="{C}" cy="{C}" r="22" fill="{p["pin"]}"/>')
    # Re-draw the pointer handle over the ring so it reads as on top.
    x0, y0 = pt(r_ring-20, POINTER); x1, y1 = pt(r_handle, POINTER)
    parts.append(f'<line x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" stroke="{p["amber"]}" stroke-width="42" stroke-linecap="round"/>')
    return parts

def snow(p):
    """Flakes scattered over the tile, the same every time."""
    color, opacity = p["snow"]
    flakes = [(188, 214, 9), (300, 160, 6), (842, 196, 8), (770, 140, 5), (166, 400, 6),
              (868, 380, 7), (150, 600, 8), (880, 620, 6), (210, 820, 6), (820, 834, 9),
              (402, 874, 5), (640, 880, 7), (520, 150, 5), (264, 300, 4), (760, 300, 4)]
    return [f'<g fill="{color}" opacity="{opacity}">'
            + "".join(f'<circle cx="{x}" cy="{y}" r="{r}"/>' for x, y, r in flakes) + "</g>"]

def hat(p):
    """A Santa hat on the top left of the ring, its tip flopping over to the right."""
    x, y = pt(205, 236)
    red = p["amber"]
    return [f'<g transform="translate({x:.1f},{y:.1f}) rotate(-34) scale(0.82)">'
            f'<path d="M-96,-6 C-84,-130 -20,-210 70,-214 C126,-216 168,-186 182,-128 '
            f'C150,-160 108,-168 76,-150 C88,-104 96,-52 96,-6 Z" fill="{red}"/>'
            f'<path d="M-40,-30 C-30,-110 10,-170 64,-188" fill="none" stroke="#000" stroke-opacity="0.12" stroke-width="14" stroke-linecap="round"/>'
            '<rect x="-122" y="-34" width="244" height="64" rx="32" fill="#ffffff"/>'
            '<rect x="-122" y="-34" width="244" height="64" rx="32" fill="none" stroke="#000" stroke-opacity="0.08" stroke-width="4"/>'
            '<circle cx="184" cy="-118" r="40" fill="#ffffff"/>'
            '<circle cx="184" cy="-118" r="40" fill="none" stroke="#000" stroke-opacity="0.08" stroke-width="4"/>'
            "</g>"]

def svg(body, defs=""):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">\n'
            + (f"  <defs>\n{defs}  </defs>\n" if defs else "")
            + "".join(f"  {line}\n" for line in body) + "</svg>\n")

def flat(p, festive=False):
    """The whole icon on a rounded tile, with the macOS grid's margin and shadow."""
    top, bottom = p["bg"]
    defs = f'''    <linearGradient id="bg" x1="0" y1="0" x2="0.6" y2="1">
      <stop offset="0" stop-color="{top}"/>
      <stop offset="1" stop-color="{bottom}"/>
    </linearGradient>
    <linearGradient id="sheen" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.14"/>
      <stop offset="0.5" stop-color="#ffffff" stop-opacity="0"/>
    </linearGradient>
    <filter id="shadow" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="12" stdDeviation="14" flood-color="#000" flood-opacity="0.28"/>
    </filter>
'''
    tile = 'x="100" y="100" width="824" height="824" rx="185"'
    body = [f'<g filter="url(#shadow)"><rect {tile} fill="url(#bg)"/></g>',
            f'<rect {tile} fill="url(#sheen)"/>']
    # A hairline keeps the white tile from melting into light backgrounds.
    if p["edge"]:
        body.append(f'<rect x="102" y="102" width="820" height="820" rx="183" fill="none" stroke="#0b1f5c" stroke-opacity="{p["edge"]}" stroke-width="4"/>')
    body += (snow(p) if festive else []) + dial(p) + helm(p) + (hat(p) if festive else [])
    return svg(body, defs)

def srgb(hex_color):
    r, g, b = (int(hex_color[i:i+2], 16)/255 for i in (1, 3, 5))
    return f"srgb:{r:.5f},{g:.5f},{b:.5f},1.00000"

def icon_composer(out):
    """AppIcon.icon: the system draws the tile and the glass; we give it the
    tile colours and two layers per version."""
    assets = os.path.join(out, "AppIcon.icon", "Assets")
    os.makedirs(assets, exist_ok=True)
    for mode, p in PALETTES.items():
        for name, layer in (("dial", dial), ("helm", helm)):
            with open(os.path.join(assets, f"{name}-{mode}.svg"), "w") as f:
                f.write(svg(layer(p)))
    def gradient(mode):
        return {"linear-gradient": [srgb(c) for c in PALETTES[mode]["bg"]], "orientation": {"start": {"x": 0.5, "y": 0}, "stop": {"x": 0.5, "y": 1}}}
    def group(name):
        return {"layers": [{
                    "name": name,
                    "image-name-specializations": [
                        {"value": f"{name}-light.svg"},
                        {"appearance": "dark", "value": f"{name}-dark.svg"},
                    ],
                    "glass": True,
                }],
                "shadow": {"kind": "neutral", "opacity": 0.5},
                "translucency": {"enabled": True, "value": 0.4}}
    doc = {
        "fill-specializations": [
            {"value": gradient("light")},
            {"appearance": "dark", "value": gradient("dark")},
        ],
        "groups": [group("helm"), group("dial")],   # front first
        "supported-platforms": {"squares": ["macOS"]},
    }
    with open(os.path.join(out, "AppIcon.icon", "icon.json"), "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")

out = sys.argv[1]
with open(os.path.join(out, "icon.svg"), "w") as f:
    f.write(flat(PALETTES["light"]))
with open(os.path.join(out, "icon-dark.svg"), "w") as f:
    f.write(flat(PALETTES["dark"]))
icon_composer(out)
with open(os.path.join(out, "icon-festive.svg"), "w") as f:
    f.write(flat(FESTIVE["light"], festive=True))
with open(os.path.join(out, "icon-festive-dark.svg"), "w") as f:
    f.write(flat(FESTIVE["dark"], festive=True))
