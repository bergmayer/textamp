"""Rasterize the placeholder SVG paths with ImageMagick's vector drawing backend."""
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET

for path in Path(__file__).parent.glob('*.svg'):
    root = ET.fromstring(path.read_text())
    commands = ['translate 70.4,70.4 scale 4.8,4.8 stroke-linecap round stroke-linejoin round']
    for node in root:
        kind = node.tag.rsplit('}', 1)[-1]
        if kind == 'circle':
            x, y, radius = (float(node.attrib[key]) for key in ('cx', 'cy', 'r'))
            commands.append(f'circle {x},{y} {x + radius},{y}')
        elif kind == 'path':
            commands.append(f'path "{node.attrib["d"]}"')
        else:
            raise ValueError(f'Unsupported SVG primitive: {kind}')
    subprocess.run(['magick', '-size', '256x256', 'canvas:none', '-fill', 'none',
                    '-stroke', 'white', '-strokewidth', '2', '-draw', ' '.join(commands),
                    str(path.with_suffix('.png'))], check=True)
