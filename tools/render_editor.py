#!/usr/bin/env python3
"""Rasterize the native editor's test captures using Pillow (no DAW required).

REVELER_UI_CAPTURE_DIR=/tmp/reveler-ui cargo test -p reveler-rs --lib native_render
python3 tools/render_editor.py /tmp/reveler-ui/*.mesh

These are the editor's real egui meshes and font atlas, not a second UI implementation.
"""
import math
from pathlib import Path
import struct
import sys
from PIL import Image


def render(path):
    data = memoryview(path.read_bytes())
    offset = 0

    def take(fmt):
        nonlocal offset
        result = struct.unpack_from('<' + fmt, data, offset)
        offset += struct.calcsize('<' + fmt)
        return result

    width, height, tw, th, count = take('5I')
    texture = Image.frombytes('RGBA', (tw, th), data[offset:offset + tw * th * 4].tobytes())
    offset += tw * th * 4
    tex = texture.load()
    image = Image.new('RGBA', (width, height))
    pixels = image.load()
    for _ in range(count):
        clip = take('4f')
        nv, ni = take('2I')
        vertices = [take('4f4B') for _ in range(nv)]
        indices = take(f'{ni}I')
        for i in range(0, ni, 3):
            a, b, c = [vertices[k] for k in indices[i:i+3]]
            left = max(0, math.floor(clip[0]), math.floor(min(a[0], b[0], c[0])))
            right = min(width, math.ceil(clip[2]), math.ceil(max(a[0], b[0], c[0])))
            top = max(0, math.floor(clip[1]), math.floor(min(a[1], b[1], c[1])))
            bottom = min(height, math.ceil(clip[3]), math.ceil(max(a[1], b[1], c[1])))
            denom = (b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
            if abs(denom) < 1e-8:
                continue
            if denom < 0:
                a, b = b, a
                denom = -denom
            for y in range(top, bottom):
                for x in range(left, right):
                    p = ((b[1]-c[1])*(x+.5-c[0])+(c[0]-b[0])*(y+.5-c[1]))/denom
                    q = ((c[1]-a[1])*(x+.5-c[0])+(a[0]-c[0])*(y+.5-c[1]))/denom
                    r = 1-p-q
                    # Top-left fill rule avoids blending shared triangle edges twice.
                    if min(p, q, r) < -1e-6:
                        continue
                    if abs(r) < 1e-6 and (b[1] > a[1] or (b[1] == a[1] and b[0] < a[0])):
                        continue
                    if abs(q) < 1e-6 and (a[1] > c[1] or (a[1] == c[1] and a[0] < c[0])):
                        continue
                    if abs(p) < 1e-6 and (c[1] > b[1] or (c[1] == b[1] and c[0] < b[0])):
                        continue
                    u = min(tw-1, max(0, int((p*a[2]+q*b[2]+r*c[2])*tw)))
                    v = min(th-1, max(0, int((p*a[3]+q*b[3]+r*c[3])*th)))
                    sample = tex[u, v]
                    color = [max(0, min(255, p*a[j+4]+q*b[j+4]+r*c[j+4]))*sample[j]/255 for j in range(4)]
                    alpha = color[3]/255
                    if alpha == 0:
                        continue
                    dest = pixels[x, y]
                    pixels[x, y] = tuple(min(255, round(color[j]+dest[j]*(1-alpha))) for j in range(4))
    destination = path.with_suffix('.png')
    image.save(destination)
    print(destination, flush=True)


if __name__ == '__main__':
    for arg in sys.argv[1:]:
        render(Path(arg))
