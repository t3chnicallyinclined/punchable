"""Phone-video / audio metadata (ffprobe), venue inference and show grouping.

The main way sets arrive is videos imported from the phone's Photos library
(.mov/.mp4/.m4v, H.264 or HEVC). Only the audio is extracted; the original video
is moved, never copied. What ffprobe exposes is read into the set's meta:

  creation_time                         UTC (QuickTime/MP4 container)
  com.apple.quicktime.creationdate      iPhone: local time WITH the timezone offset
  com.apple.quicktime.location.ISO6709  iPhone GPS, e.g. +40.7295-073.9965+010.000/
  location / location-eng               Android GPS (same ISO 6709 form)
  com.apple.quicktime.make/model, com.android.manufacturer/model   device
"""
from __future__ import annotations

import json
import math
import re
import subprocess
from datetime import datetime, timedelta, timezone
from pathlib import Path

VIDEO_EXT = {".mov", ".mp4", ".m4v", ".3gp", ".mkv", ".webm"}


def parse_iso6709(s: str | None):
    if not s:
        return None
    m = re.match(r"\s*([+-]\d+(?:\.\d+)?)([+-]\d+(?:\.\d+)?)([+-]\d+(?:\.\d+)?)?(?:CRS[^/]*)?/?\s*$", s)
    if not m:
        return None
    lat, lon = float(m.group(1)), float(m.group(2))
    if not (-90 <= lat <= 90 and -180 <= lon <= 180):
        return None
    out = {"lat": lat, "lon": lon}
    if m.group(3):
        out["alt_m"] = float(m.group(3))
    return out


def _parse_time(s: str | None):
    if not s:
        return None
    s = s.strip()
    s = re.sub(r"([+-]\d{2})(\d{2})$", r"\1:\2", s)  # -0400 -> -04:00
    s = s.replace("Z", "+00:00")
    try:
        d = datetime.fromisoformat(s)
    except ValueError:
        return None
    return d


def probe(path: Path) -> dict:
    """ffprobe -> {duration_s, has_video, video_codec, audio_codec, created_utc, created_local, tz,
    date, date_source, gps, device, tags}. Missing fields are None."""
    r = subprocess.run(["ffprobe", "-v", "error", "-show_entries",
                        "format=duration:format_tags:stream=codec_type,codec_name:stream_tags=creation_time",
                        "-of", "json", str(path)], capture_output=True, text=True, encoding="utf-8")
    try:
        doc = json.loads(r.stdout or "{}")
    except ValueError:
        doc = {}
    fmt = doc.get("format", {})
    tags = {k.lower(): v for k, v in (fmt.get("tags") or {}).items()}
    streams = doc.get("streams", [])
    vid = next((s for s in streams if s.get("codec_type") == "video" and s.get("codec_name") not in ("mjpeg", "png")), None)
    aud = next((s for s in streams if s.get("codec_type") == "audio"), None)
    stream_ct = next(((s.get("tags") or {}).get("creation_time") for s in streams if (s.get("tags") or {}).get("creation_time")), None)
    utc = _parse_time(tags.get("creation_time") or stream_ct)
    local = _parse_time(tags.get("com.apple.quicktime.creationdate"))
    if utc and utc.tzinfo is None:
        utc = utc.replace(tzinfo=timezone.utc)
    if local and local.tzinfo is not None:
        date, date_source, tz = local.date().isoformat(), "com.apple.quicktime.creationdate (local, with timezone)", \
            local.strftime("%z")
        if utc is None:
            utc = local.astimezone(timezone.utc)
    elif utc:
        loc = utc.astimezone()  # no timezone in the file: this PC's timezone
        date, date_source, tz = loc.date().isoformat(), "creation_time (UTC) in this PC's timezone", loc.strftime("%z")
    else:
        date, date_source, tz = None, None, None
    gps = parse_iso6709(tags.get("com.apple.quicktime.location.iso6709") or tags.get("location")
                        or tags.get("location-eng"))
    make = tags.get("com.apple.quicktime.make") or tags.get("com.android.manufacturer") or tags.get("make")
    model = tags.get("com.apple.quicktime.model") or tags.get("com.android.model") or tags.get("model")
    try:
        dur = float(fmt.get("duration"))
    except (TypeError, ValueError):
        dur = 0.0
    keep = {k: v for k, v in tags.items() if k in ("creation_time", "com.apple.quicktime.creationdate",
            "com.apple.quicktime.location.iso6709", "location", "location-eng", "com.apple.quicktime.make",
            "com.apple.quicktime.model", "com.apple.quicktime.software", "com.android.manufacturer",
            "com.android.model", "com.android.version", "major_brand")}
    return {"duration_s": round(dur, 2), "has_video": vid is not None,
            "video_codec": vid.get("codec_name") if vid else None, "audio_codec": aud.get("codec_name") if aud else None,
            "created_utc": utc.isoformat() if utc else None, "created_local": local.isoformat() if local else None,
            "tz": tz, "date": date, "date_source": date_source, "gps": gps,
            "device": " ".join(x for x in (make, model) if x) or None, "tags": keep}


def haversine_m(a: dict, b: dict) -> float:
    R = 6371000.0
    p1, p2 = math.radians(a["lat"]), math.radians(b["lat"])
    dp, dl = p2 - p1, math.radians(b["lon"] - a["lon"])
    h = math.sin(dp / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
    return 2 * R * math.asin(math.sqrt(h))


def known_venues(dirs: list[Path]) -> list[dict]:
    """Named venues with coordinates from earlier set metas (meta.json: venue + gps)."""
    out = []
    for root in dirs:
        if not root.exists():
            continue
        for mp in root.glob("*/meta.json"):
            try:
                m = json.loads(mp.read_text(encoding="utf-8"))
            except (ValueError, OSError):
                continue
            gps = m.get("gps")
            name = m.get("venue") if m.get("venue_status") in ("confirmed", "inferred", "given") else m.get("venue")
            if gps and name and not str(name).startswith("unnamed venue"):
                out.append({"venue": name, "gps": gps, "set": mp.parent.name, "status": m.get("venue_status")})
    return out


def infer_venue(gps: dict | None, venues: list[dict], radius_m: float = 200.0) -> dict:
    if not gps:
        return {"venue": "", "venue_status": "unknown", "venue_basis": "no GPS in the file"}
    best = min(((haversine_m(gps, v["gps"]), v) for v in venues), default=None, key=lambda x: x[0])
    if best and best[0] <= radius_m:
        return {"venue": best[1]["venue"], "venue_status": "inferred",
                "venue_basis": f"GPS {best[0]:.0f} m from set {best[1]['set']} at {best[1]['venue']}"}
    return {"venue": f"unnamed venue @ {gps['lat']:.5f},{gps['lon']:.5f}", "venue_status": "inferred",
            "venue_basis": "GPS matches no known venue; coordinates stored"}


def same_show(a: dict, b: dict, hours: float = 3.0, radius_m: float = 200.0) -> bool:
    """Two recordings are one show if within `hours` and `radius_m` (both need time and GPS)."""
    if not (a.get("created_utc") and b.get("created_utc") and a.get("gps") and b.get("gps")):
        return False
    ta, tb = datetime.fromisoformat(a["created_utc"]), datetime.fromisoformat(b["created_utc"])
    ea = ta + timedelta(seconds=a.get("duration_s") or 0)
    eb = tb + timedelta(seconds=b.get("duration_s") or 0)
    gap = max(timedelta(0), max(ta, tb) - min(ea, eb))
    return gap <= timedelta(hours=hours) and haversine_m(a["gps"], b["gps"]) <= radius_m
