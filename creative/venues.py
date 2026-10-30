"""Venues and the Map of performances.

    python -m creative venues [--geocode] [--json]

Venue entities come from set metadata (the graph's set node `venue`, else meta.json
`venue_guess`). Pins for sets without GPS are geocoded from the venue name with OpenStreetMap
Nominatim: at most 1 request per second, User-Agent "Punchable/0.1 (personal)", every answer
cached in comedy/kb/creative/geocode_cache.json (a cached query is never re-sent). Every
geocoded pin is `status: "inferred"` and editable; ambiguous ones are listed with the reason.
"""
from __future__ import annotations

import json
import re
import time
import urllib.parse
import urllib.request
from collections import defaultdict
from pathlib import Path

from .data import Corpus, creative_dir, venue_short
from .util import mean, now_iso, r2, slug, write_json

NOMINATIM = "https://nominatim.openstreetmap.org/search"
USER_AGENT = "Punchable/0.1 (personal)"
MIN_INTERVAL_S = 1.0
STATES = {"NYC": "New York, NY", "NY": "New York", "CT": "Connecticut", "NJ": "New Jersey"}
UNCERTAIN = ("unknown", "likely", "maybe", "(show)", "?")


def parse_venue(raw: str) -> dict:
    """'Mabel's Comedy Club, Springfield' -> name + Nominatim query; flags names that are guesses."""
    v = (raw or "").strip()
    low = v.lower()
    reasons = []
    if any(u in low for u in UNCERTAIN):
        reasons.append(f"venue name is a guess in the set metadata: '{v}'")
    m = re.search(r"maybe ([^)'\"]+)", v, re.I)
    if m:
        name, query = m.group(1).strip(), m.group(1).strip()
    else:
        base = v.split("(")[0].strip()
        parts = [p.strip() for p in base.split(",") if p.strip()]
        name = parts[0] if parts else v
        place = parts[1] if len(parts) > 1 else ""
        if not place:
            likely = re.search(r"likely ([A-Za-z ]+)", v)
            place = likely.group(1).strip() if likely else ""
        place = STATES.get(place, place)
        query = f"{name}, {place}" if place else name
    if name.lower().startswith("comix at "):
        name = name[9:]
        query = query[9:]
    queries = [query]
    if "," in query:
        place = query.split(",", 1)[1].strip()
        queries.append(f"{name}, {place.split(',')[0].strip()}")
    queries.append(name)
    return {"name": name, "query": query, "queries": list(dict.fromkeys(queries)), "uncertain_reasons": reasons}


class Geocoder:
    def __init__(self, cache_path: Path | None = None, fetch=None, sleep=time.sleep, clock=time.monotonic):
        self.cache_path = Path(cache_path or creative_dir() / "geocode_cache.json")
        self.cache = json.load(open(self.cache_path, encoding="utf-8")) if self.cache_path.exists() else {}
        self._fetch = fetch or self._http
        self._sleep, self._clock = sleep, clock
        self._last = None
        self.requests = 0

    @staticmethod
    def _http(url: str):
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept-Language": "en"})
        with urllib.request.urlopen(req, timeout=20) as r:
            return json.loads(r.read().decode("utf-8"))

    def search(self, query: str, offline: bool = False) -> list[dict] | None:
        if query in self.cache:
            return self.cache[query]["results"]
        if offline:
            return None
        if self._last is not None:
            wait = MIN_INTERVAL_S - (self._clock() - self._last)
            if wait > 0:
                self._sleep(wait)
        url = NOMINATIM + "?" + urllib.parse.urlencode({"q": query, "format": "jsonv2", "limit": 5})
        try:
            raw = self._fetch(url)
        finally:
            self._last = self._clock()
            self.requests += 1
        res = [{"lat": float(x["lat"]), "lon": float(x["lon"]), "display_name": x.get("display_name"),
                "category": x.get("category") or x.get("class"), "type": x.get("type"),
                "importance": x.get("importance"), "osm_type": x.get("osm_type"), "osm_id": x.get("osm_id")}
               for x in raw or []]
        self.cache[query] = {"fetched_at": now_iso(), "source": "nominatim", "results": res}
        write_json(self.cache_path, self.cache)
        return res


def _km(a, b):
    import math
    la1, lo1, la2, lo2 = map(math.radians, (a["lat"], a["lon"], b["lat"], b["lon"]))
    h = math.sin((la2 - la1) / 2) ** 2 + math.cos(la1) * math.cos(la2) * math.sin((lo2 - lo1) / 2) ** 2
    return 6371 * 2 * math.asin(math.sqrt(h))


VENUE_TYPES = {"theatre", "arts_centre", "nightclub", "bar", "pub", "restaurant", "cafe", "casino", "events_venue",
               "community_centre", "music_venue", "comedy_club", "hotel", "resort"}


def venue_addresses() -> dict:
    """Street addresses the comic supplied or asked us to look up, keyed by lower-case venue name:
    comedy/kb/creative/venue_addresses.json {"mabel's": {"address": "...", "note": "..."}}. Tried before the name."""
    f = creative_dir() / "venue_addresses.json"
    return json.load(open(f, encoding="utf-8")) if f.exists() else {}


def geocode_venue(geo: Geocoder, raw: str, offline: bool = False) -> dict:
    pv = parse_venue(raw)
    addr = venue_addresses().get(pv["name"].lower())
    if addr:
        pv["queries"] = list(dict.fromkeys([addr["address"]] + pv["queries"]))
        pv["query"] = addr["address"]
    res, used, tried = None, pv["query"], []
    for q in pv["queries"]:  # most specific first; each answer is cached, so a miss is never re-sent
        res, used = geo.search(q, offline=offline), q
        tried.append(q)
        if res:
            break
    pv["query"] = used
    out = {"raw": raw, "name": pv["name"], "query": used, "queries_tried": tried, "status": "inferred", "source": "nominatim",
           "lat": None, "lon": None, "display_name": None, "ambiguous": False, "ambiguous_reasons": list(pv["uncertain_reasons"])}
    if res is None:
        out["ambiguous"] = True
        out["ambiguous_reasons"].append("not geocoded yet (offline)")
        return out
    if not res:
        out["ambiguous"] = True
        out["ambiguous_reasons"].append(f"Nominatim found nothing for {' / '.join(repr(q) for q in tried)}")
        return out
    if used != tried[0] or used == pv["name"]:
        if used == pv["name"] and len(pv["queries"]) > 1:
            out["ambiguous_reasons"].append(f"matched only on the bare name '{used}'")
    top = res[0]
    out.update({"lat": top["lat"], "lon": top["lon"], "display_name": top["display_name"],
                "osm": f"{top['osm_type']}/{top['osm_id']}", "osm_type": top["type"]})
    far = [r for r in res[1:] if _km(top, r) > 2.0]
    if far:
        out["ambiguous_reasons"].append(f"{len(far)} other match(es) more than 2 km away "
                                        f"(e.g. {far[0]['display_name'][:80]})")
    if top.get("type") not in VENUE_TYPES and top.get("category") not in ("amenity", "leisure", "tourism"):
        out["ambiguous_reasons"].append(f"top match is a {top.get('category')}/{top.get('type')}, not a venue")
    out["ambiguous"] = bool(out["ambiguous_reasons"])
    return out


def venue_stats(c: Corpus) -> list[dict]:
    by = defaultdict(list)
    for k in c.set_keys():
        by[c.sets[k]["venue"]].append(k)
    # per joke: vs_room per venue
    jv = defaultdict(lambda: defaultdict(list))
    for jid, ps in c.perfs.items():
        for p in ps:
            if p["vs_room"] is not None:
                jv[jid][p["venue"]].append(p)
    out = []
    for venue, keys in sorted(by.items()):
        sets = []
        for k in keys:
            s = c.sets[k]["summary"]
            sets.append({"set": k, "date": c.sets[k]["date"], "title": c.sets[k]["title"],
                         "laugh_s_per_min": s.get("laugh_s_per_min"), "room_heat": s.get("room_heat"),
                         "pct_laughing": s.get("pct_laughing"), "material_s": s.get("material_s"),
                         "best": s.get("best") and c.map_joke(s.get("best")),
                         "weakest": s.get("weakest") and c.map_joke(s.get("weakest"))})
        here = [(jid, p) for jid, vs in jv.items() for p in vs.get(venue, [])]
        here.sort(key=lambda x: (-(x[1]["vs_room"] or 0), x[0]))
        rank = lambda x: {"joke_id": x[0], "title": c.title(x[0]), "date": x[1]["date"], "vs_room": x[1]["vs_room"],
                          "laugh_s_per_min": x[1]["laugh_s_per_min"], "ai_rating": x[1]["ai_rating"],
                          "evidence": [e for e in [c.seg_evidence(x[1]["main_seg"])] if e]}
        diff = []
        for jid, vs in jv.items():
            if venue in vs and len(vs) > 1:
                # vs_room clipped at 3.0: a near-silent room inflates it (a casino room '17: 7.47x)
                h = mean([min(3.0, p["vs_room"]) for p in vs[venue]])
                el = mean([min(3.0, p["vs_room"]) for v, ps in vs.items() if v != venue for p in ps])
                n_else = sum(len(ps) for v, ps in vs.items() if v != venue)
                diff.append({"joke_id": jid, "title": c.title(jid), "here_vs_room": r2(h), "elsewhere_vs_room": r2(el),
                             "delta": r2(h - el), "n_here": len(vs[venue]), "n_elsewhere": n_else,
                             "low_n": len(vs[venue]) < 2 or n_else < 2})
        diff.sort(key=lambda d: (-d["delta"], d["joke_id"]))
        out.append({
            "venue_id": f"venue:{slug(parse_venue(venue)['name'], 40)}", "raw": venue, "name": parse_venue(venue)["name"],
            "sets": sets, "dates": [s["date"] for s in sets], "n_sets": len(sets),
            "avg_laugh_s_per_min": r2(mean([s["laugh_s_per_min"] for s in sets])),
            "avg_room_heat": r2(mean([s["room_heat"] for s in sets])),
            "best_jokes": [rank(x) for x in here[:3]],
            "worst_jokes": [rank(x) for x in sorted(here, key=lambda x: ((x[1]["vs_room"] or 0), x[0]))[:3]],
            "works_here": [d for d in diff if d["delta"] > 0.2][:5],
            "works_elsewhere": [d for d in sorted(diff, key=lambda d: (d["delta"], d["joke_id"])) if d["delta"] < -0.2][:5],
            "note": "vs_room compares a joke with the rest of that night's set; one set per room is a small sample.",
        })
    return out


def heat_color(h) -> str:
    if h is None:
        return "#9ca3af"
    return "#22c55e" if h >= 1.2 else "#f5b544" if h >= 0.8 else "#fca5a5"


def build(c: Corpus, geo: Geocoder | None = None, offline: bool = False) -> dict:
    geo = geo or Geocoder()
    stats = venue_stats(c)
    features, ambiguous = [], []
    for v in stats:
        g = geocode_venue(geo, v["raw"], offline=offline)
        v["geo"] = g
        if g["ambiguous"]:
            ambiguous.append({"venue_id": v["venue_id"], "raw": v["raw"], "reasons": g["ambiguous_reasons"],
                              "picked": g["display_name"]})
        features.append({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [g["lon"], g["lat"]]} if g["lat"] is not None else None,
            "properties": {"venue_id": v["venue_id"], "name": v["name"], "raw": v["raw"], "status": g["status"],
                           "ambiguous": g["ambiguous"], "n_sets": v["n_sets"], "dates": v["dates"],
                           "avg_laugh_s_per_min": v["avg_laugh_s_per_min"], "avg_room_heat": v["avg_room_heat"],
                           "color": heat_color(v["avg_room_heat"]),
                           "label": venue_short(v["raw"], None)}})
    return {"generated_at": now_iso(), "venues": stats, "ambiguous": ambiguous,
            "geojson": {"type": "FeatureCollection", "features": features},
            "geocoder": {"service": "OpenStreetMap Nominatim", "user_agent": USER_AGENT,
                         "requests_this_run": geo.requests, "cache": str(geo.cache_path),
                         "attribution": "Data © OpenStreetMap contributors, ODbL"}}
