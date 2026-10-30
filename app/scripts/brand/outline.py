"""Outline text in the app's own fonts to SVG paths, so the brand SVGs need no font at all.
   python app/scripts/brand/outline.py   ->  app/scripts/brand/glyphs.json   (fontTools only)"""
import json, os
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.boundsPen import BoundsPen

HERE = os.path.dirname(os.path.abspath(__file__))
FONT = os.path.join(HERE, '..', '..', 'node_modules', '@expo-google-fonts', 'space-grotesk', '700Bold', 'SpaceGrotesk_700Bold.ttf')

def outline(text, size=100, track=-3):
    f = TTFont(FONT); gs = f.getGlyphSet(); cmap = f.getBestCmap(); s = size / f['head'].unitsPerEm
    pen = SVGPathPen(gs, ntos=lambda v: ('%.2f' % v).rstrip('0').rstrip('.')); bp = BoundsPen(gs); x = 0
    for ch in text:
        g = cmap[ord(ch)]; t = (s, 0, 0, -s, x, 0)
        gs[g].draw(TransformPen(pen, t)); gs[g].draw(TransformPen(bp, t))
        x += f['hmtx'][g][0] * s + track
    return {'d': pen.getCommands(), 'bounds': [round(v, 2) for v in bp.bounds]}

out = {'font': 'Space Grotesk Bold (SIL OFL), size 100, tracking -3', 'punchable': outline('punchable'), 'punch': outline('punch'), 'p': outline('p', track=0)}
json.dump(out, open(os.path.join(HERE, 'glyphs.json'), 'w'), indent=1)
print({k: v['bounds'] for k, v in out.items() if k != 'font'})
