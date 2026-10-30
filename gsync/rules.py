"""Legal exclusion and comedy triage for Google Docs.

Court and legal documents are never exported. The generic rules below always apply; your own list (folder ids,
doc ids, extra title keywords, your Google account) goes in a private JSON file, never in the repo:
    $PUNCHABLE_HOME/legal_rules.json (default ~/.punchable/legal_rules.json), e.g.
    {"owner": "you@example.com", "legal_parents": ["<folder id>"], "legal_folders_extra": [], "keywords": ["..."],
     "content_markers": ["..."], "legal_ids": ["<doc id>"], "not_comedy_ids": [], "ingest_max_modified": null,
     "own_keywords_only_ids": []}
The local file can only exclude MORE, never less, with one narrow exception: a doc listed in own_keywords_only_ids
(a bit whose title trips a generic keyword, e.g. "illegal") is checked against YOUR keyword list instead of the
generic one. The denylist, folders, owner, filing names and the content check still apply to it. Jokes that mention a judge or a courtroom are part of an act and
are not excluded by content alone.
"""
from __future__ import annotations

import json
import os
import re
from pathlib import Path


def _local_rules() -> dict:
    p = Path(os.environ.get("PUNCHABLE_LEGAL_RULES") or
             Path(os.environ.get("PUNCHABLE_HOME", Path.home() / ".punchable")) / "legal_rules.json")
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


_LOCAL = _local_rules()

# ---- title and folder rules -------------------------------------------------------------------------------------
LEGAL_PARENTS = set(_LOCAL.get("legal_parents", []))
LEGAL_FOLDERS_EXTRA = set(_LOCAL.get("legal_folders_extra", []))
GENERIC_KW = ['petition', 'court', 'cross examination', 'exhibit', 'affidavit', 'deposition', 'lawsuit', 'hearing notes',
              'legal']
OWN_KW = [k.lower() for k in _LOCAL.get("keywords", [])]
KW = GENERIC_KW + [k for k in OWN_KW if k not in GENERIC_KW]
OWN_KW_ONLY_IDS = frozenset(_LOCAL.get("own_keywords_only_ids", []))  # only honoured when you have your own keywords
OWNER = _LOCAL.get("owner") or os.environ.get("PUNCHABLE_OWNER_EMAIL") or None  # None: owner check off

FILING_KW = ['summation', 'affirmation', 'affadavit', 'memorandum of law', 'motion to', 'order to show cause', 'otsc',
             'subpoena', 'q&a prep', 'cover letter for osc']
FILING_WORDS = re.compile(r'\bosc\b')
# Second net, on exported text only: two or more of these and the doc is dropped at once.
CONTENT_MARKERS = ['supreme court of the state of', 'court of the state of', 'appellate division', 'index no',
                   'docket no', 'file no.', 'petitioner', 'respondent', 'plaintiff', 'defendant',
                   'under penalty of perjury', 'sworn to before me', 'order to show cause', 'affirmation in']
# Your own markers (court names, your county, ...) from legal_rules.json; each counts toward the two-marker rule.
CONTENT_MARKERS += [m.lower() for m in _LOCAL.get("content_markers", []) if m.lower() not in CONTENT_MARKERS]

LEGAL_IDS = frozenset(_LOCAL.get("legal_ids", []))          # docs you excluded by hand
NOT_COMEDY_IDS = frozenset(_LOCAL.get("not_comedy_ids", []))  # docs you judged not comedy
INGEST_MAX_MODIFIED = _LOCAL.get("ingest_max_modified")


def legal_reason(meta: dict, ancestors: list[str] | None = None, extra_ids: set | frozenset = frozenset()) -> str | None:
    """Metadata-only check. Returns why a doc is legal, or None. Never needs the doc's text.
    meta: {id, name, owners:[{emailAddress}], parents:[...]}. ancestors: every folder above the doc (optional)."""
    did = meta.get('id')
    if did in LEGAL_IDS or did in extra_ids:
        return 'denylist'
    parents = list(meta.get('parents') or [])
    for p in parents + list(ancestors or []):
        if p in LEGAL_PARENTS or p in LEGAL_FOLDERS_EXTRA:
            return 'folder'
    owners = [o.get('emailAddress') for o in (meta.get('owners') or [])]
    if not owners or (OWNER and owners[0] != OWNER):
        return 'owner'
    t = (meta.get('name') or '').lower()
    hit = [k for k in (OWN_KW if did in OWN_KW_ONLY_IDS and OWN_KW else KW) if k in t]
    if hit:
        return 'kw:' + ','.join(hit)
    hit = [k for k in FILING_KW if k in t]
    if hit or FILING_WORDS.search(t):
        return 'filing:' + ','.join(hit or ['osc'])
    return None


def content_is_legal(text: str) -> bool:
    t = (text or '').lower()
    return sum(1 for m in CONTENT_MARKERS if m in t) >= 2


# ---- comedy triage (a new doc defaults to comedy unless clearly not) ---------------------------------------------
# Titles that are clearly not comedy (resumes, crypto, tech, app and business).
NOT_COMEDY_TITLE = re.compile(
    r'\b(resume|résumé|cover ?letter|coverletter|app idea|business (plan|idea|model)|start a business|blockchain|'
    r'crypto|dao|nft|kubernetes|landing page|take-home|sow|10dlc|test 1|copy of test)\b', re.I)


def triage(meta: dict) -> str:
    """'comedy' | 'not_comedy'. Run only after legal_reason() returned None."""
    if meta.get('id') in NOT_COMEDY_IDS:
        return 'not_comedy'
    if NOT_COMEDY_TITLE.search(meta.get('name') or ''):
        return 'not_comedy'
    return 'comedy'


def fingerprint() -> str:
    """SHA-256 over the legal constants in a canonical form (so a change is visible in logs)."""
    import hashlib
    canon = json.dumps({'parents': sorted(LEGAL_PARENTS), 'extra': sorted(LEGAL_FOLDERS_EXTRA), 'kw': KW,
                        'owner': OWNER, 'ids': sorted(LEGAL_IDS), 'markers': CONTENT_MARKERS,
                        'own_kw_only': sorted(OWN_KW_ONLY_IDS)}, sort_keys=True)
    return hashlib.sha256(canon.encode()).hexdigest()
