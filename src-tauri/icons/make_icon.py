#!/usr/bin/env python3
"""Generates icon.svg, the source for the app icon.

Regenerate the platform icons with:
  python3 src-tauri/icons/make_icon.py src-tauri/icons/icon.svg
  rsvg-convert -w 1024 -h 1024 src-tauri/icons/icon.svg -o src-tauri/icons/app-icon.png
  cargo tauri icon src-tauri/icons/app-icon.png -o src-tauri/icons
  rsvg-convert -w 256 -h 256 src-tauri/icons/icon.svg -o src-tauri/icons/256x256.png
  rsvg-convert -w 48 -h 48 src-tauri/icons/icon.svg -o src-tauri/icons/48x48.png

Design: a 7-spoke helm (the Kubernetes motif) set in a control dial; the amber
handle is the pointer and the amber arc shows how far the dial is turned.
"""
import math, sys
C = 512
def pt(r, deg):
    a = math.radians(deg)
    return C + r*math.cos(a), C + r*math.sin(a)

START, SWEEP, LEVEL = 135.0, 270.0, 0.62      # dial: gap at the bottom, 62% turned
pointer = START + SWEEP*LEVEL
AMBER, WHITE = "#ffb21e", "#ffffff"

parts = []
# Faint rounded heptagon behind the wheel: a nod to the Kubernetes mark.
hept = [pt(300, -90 + i*360/7) for i in range(7)]
d = "M" + " L".join(f"{x:.1f},{y:.1f}" for x, y in hept) + " Z"
parts.append(f'<g opacity="0.13"><path d="{d}" fill="#ffffff" stroke="#ffffff" stroke-width="60" stroke-linejoin="round"/></g>')
# Dial track (arc) + ticks
r_track = 352
def arc(r, a0, a1):
    x0,y0 = pt(r,a0); x1,y1 = pt(r,a1)
    large = 1 if (a1-a0) > 180 else 0
    return f"M{x0:.1f},{y0:.1f} A{r},{r} 0 {large} 1 {x1:.1f},{y1:.1f}"
parts.append(f'<path d="{arc(r_track, START, START+SWEEP)}" fill="none" stroke="{WHITE}" stroke-opacity="0.18" stroke-width="22" stroke-linecap="round"/>')
parts.append(f'<path d="{arc(r_track, START, pointer)}" fill="none" stroke="{AMBER}" stroke-width="22" stroke-linecap="round"/>')

# Helm wheel: ring, 7 spokes with handles; one handle is the amber pointer
r_ring, r_hub, r_handle = 205, 64, 268
spokes = [pointer + i*360/7 for i in range(7)]
for i, a in enumerate(spokes):
    x0,y0 = pt(r_hub-10, a); x1,y1 = pt(r_handle, a)
    color = AMBER if i == 0 else WHITE
    parts.append(f'<line x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" stroke="{color}" stroke-width="{42 if i == 0 else 38}" stroke-linecap="round"/>')
parts.append(f'<circle cx="{C}" cy="{C}" r="{r_ring}" fill="none" stroke="{WHITE}" stroke-width="44"/>')
# Hub with a centre pin
parts.append(f'<circle cx="{C}" cy="{C}" r="{r_hub}" fill="{WHITE}"/>')
parts.append(f'<circle cx="{C}" cy="{C}" r="22" fill="#1d4fb8"/>')
# re-draw pointer handle tip over the ring so it reads as on top
x0,y0 = pt(r_ring-20, pointer); x1,y1 = pt(r_handle, pointer)
parts.append(f'<line x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" stroke="{AMBER}" stroke-width="42" stroke-linecap="round"/>')

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0.6" y2="1">
      <stop offset="0" stop-color="#3b82f6"/>
      <stop offset="1" stop-color="#1a3c9c"/>
    </linearGradient>
    <linearGradient id="sheen" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.18"/>
      <stop offset="0.5" stop-color="#ffffff" stop-opacity="0"/>
    </linearGradient>
    <filter id="shadow" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="12" stdDeviation="14" flood-color="#000" flood-opacity="0.28"/>
    </filter>
    <filter id="lift" x="-10%" y="-10%" width="120%" height="120%">
      <feDropShadow dx="0" dy="6" stdDeviation="8" flood-color="#0b1f5c" flood-opacity="0.35"/>
    </filter>
  </defs>
  <g filter="url(#shadow)">
    <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#bg)"/>
  </g>
  <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#sheen)"/>
  <g filter="url(#lift)">
    {chr(10).join("    "+p for p in parts)}
  </g>
</svg>
'''
open(sys.argv[1], "w").write(svg)
