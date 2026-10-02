"""Export GPX 1.1 avec altitude."""
from __future__ import annotations

from xml.sax.saxutils import escape


def to_gpx(lon, lat, ele, name: str, desc: str = "") -> str:
    out = ['<?xml version="1.0" encoding="UTF-8"?>',
           '<gpx version="1.1" creator="trailopt" xmlns="http://www.topografix.com/GPX/1/1">',
           f"<metadata><name>{escape(name)}</name><desc>{escape(desc)}</desc></metadata>",
           f"<trk><name>{escape(name)}</name><trkseg>"]
    out += [f'<trkpt lat="{la:.7f}" lon="{lo:.7f}"><ele>{el:.1f}</ele></trkpt>'
            for lo, la, el in zip(lon, lat, ele)]
    out.append("</trkseg></trk>\n</gpx>\n")
    return "\n".join(out)
