"""Commit guard: block a commit (or push) that adds secrets, your private terms, or your own material.

    python tools/guard/check.py --staged                 # pre-commit: the lines this commit adds
    python tools/guard/check.py --range A..B             # those commits, one by one (pre-push uses this)
    python tools/guard/check.py --push < <pre-push stdin>
    python tools/guard/check.py --files PATH...          # whole files (no git)
    python tools/guard/check.py --rebuild                # rebuild the private index now

What it checks, on ADDED lines only (so a file that legitimately names the author can still be edited):
  * secrets: private keys, API tokens, JWTs, UUIDs, long hex, public IPs, long base64 (obvious fakes are skipped);
  * your private terms: regexes in $PUNCHABLE_HOME/guard_terms.json (names, servers, other projects, paths, ...);
  * your material: titles, aliases, ids and every 7-word run of words from your data folder (COMEDY_DIR):
    written units, the joke graph, set and voice memo transcripts, venues, creative boards, your notes. The index is
    built at runtime and cached OUTSIDE the repo ($PUNCHABLE_HOME/guard_cache), rebuilt when the data changes;
  * the Drive ids in your private gsync settings (legal_rules.json, gsync_docs.json);
  * file names that must never be committed (*.env, tokens, legal_rules.json, ...).
Nothing private is in this file or in the hooks: the term list and the index live only on your machine.
If $PUNCHABLE_HOME/guard_terms.json exists but the data folder does not, the guard refuses (fails closed).

Install: powershell -ExecutionPolicy Bypass -File tools/guard/install.ps1   (sets core.hooksPath=tools/guard/hooks)
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import pickle
import re
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path

VERSION = 3
HOME = Path(os.environ.get("PUNCHABLE_HOME", Path.home() / ".punchable"))
SHINGLE = 7


def _env_file() -> dict:
    out = {}
    try:
        for line in (HOME / "punchable.env").read_text(encoding="utf-8").splitlines():
            m = re.match(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$", line)
            if m:
                out[m[1]] = os.path.expandvars(m[2].strip('"'))
    except OSError:
        pass
    return out


def comedy_dir() -> Path:
    v = os.environ.get("COMEDY_DIR") or _env_file().get("COMEDY_DIR") or "~/punchable-data"
    return Path(os.path.expanduser(v))


def private_cfg() -> dict:
    try:
        return json.loads((HOME / "guard_terms.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


# ---- generic nets (public) -----------------------------------------------------------------------------------------
NEVER_COMMIT = re.compile(r"(^|/)(\.env(\..*)?|[^/]*\.env|secrets\.env|google_token\.json|google_client\.json|"
                          r"keycloak_[^/]*_token\.json|legal_rules\.json|gsync_docs\.json|venues\.json|guard_terms\.json|"
                          r"persona_synonyms\.json|punchable\.env|infra\.env|config\.env|id_rsa[^/]*|id_ed25519[^/]*|[^/]*\.pem|[^/]*\.key)$")
NEVER_COMMIT_OK = re.compile(r"\.example$")
FAKE = re.compile(r"sk-secret|test|fake|throwaway|example|dummy|placeholder|abcdefghijklmnop|s3cr3t|from-keycloak|xxxx|<|"
                  r"changeme|not-a-", re.I)
SECRETS = [r"-----BEGIN [A-Z ]*PRIVATE KEY", r"\bsk-[A-Za-z0-9_-]{16,}", r"AKIA[0-9A-Z]{16}", r"AIza[0-9A-Za-z_\-]{30,}",
           r"gh[pousr]_[A-Za-z0-9]{30,}", r"xox[abprs]-[A-Za-z0-9-]{10,}", r"GOCSPX-[A-Za-z0-9_-]{10,}",
           r"(?i)(secret|token|password|passwd|api[_-]?key|access[_-]?key)[\"']?\s*[:=]\s*[\"']?[A-Za-z0-9_\-+/=]{12,}",
           r"eyJ[A-Za-z0-9_\-]{20,}\.[A-Za-z0-9_\-]{20,}\.[A-Za-z0-9_\-]{10,}",
           r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
           r"(?<![0-9a-f])[0-9a-f]{32}(?![0-9a-f])", r"(?<![0-9a-f])[0-9a-f]{40,64}(?![0-9a-f])",
           r"\b(?:\d{1,3}\.){3}\d{1,3}\b", r"(?<![A-Za-z0-9+/_-])[A-Za-z0-9+/]{48,}={0,2}",
           r"docs\.google\.com/document/d/[A-Za-z0-9_-]{20,}", r"drive\.google\.com/drive/folders/[A-Za-z0-9_-]{20,}",
           r"\d{6,}-[a-z0-9]{20,}\.apps\.googleusercontent", r"[0-9a-f]{32}\.r2\.cloudflarestorage"]
SECRET_RX = [(p, re.compile(p)) for p in SECRETS]
IP_OK = re.compile(r"^(127\.|0\.0\.0\.0|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|203\.0\.113\.|198\.51\.100\.|"
                   r"192\.0\.2\.|255\.|169\.254\.|100\.64\.)")
BIN = (".png", ".ico", ".jpg", ".jpeg", ".gif", ".webp", ".m4a", ".wav", ".mp3", ".opus", ".woff", ".woff2", ".ttf",
       ".pdf", ".onnx", ".npz", ".f32", ".gz", ".zip")


def norm(s: str) -> str:
    return re.sub(r"[^a-z0-9]+", " ", s.lower().replace("’", "").replace("'", "")).strip()


# ---- the private index (built from the data folder at runtime; cached outside the repo) ----------------------------
def _sources(root: Path, cfg: dict) -> list[str]:
    pats = ["corpus/gdocs_units.json", "kb/v1/graph.json", "kb/v0/graph.json", "corpus/sets/*/transcript.json",
            "corpus/sets/*/meta.json", "corpus/memos/*/transcript.json", "kb/creative/*.json",
            "voice/*.md", "ideas/*.md", "notes/*.md", "notes/research/*.md"] + list(cfg.get("extra_text_globs", []))
    out = []
    for p in pats:
        out += glob.glob(str(root / p), recursive=True)
    out += [str(HOME / f) for f in ("guard_terms.json", "legal_rules.json", "gsync_docs.json", "venues.json")]
    out.append(str(Path(__file__).resolve()))
    return sorted(set(out))


def _signature(files: list[str], root: Path) -> str:
    h = hashlib.sha256(f"v{VERSION}|{root}".encode())
    for f in files:
        try:
            st = os.stat(f)
            h.update(f"{f}|{st.st_mtime_ns}|{st.st_size}\n".encode())
        except OSError:
            h.update(f"{f}|-\n".encode())
    h.update(str(sorted(glob.glob(str(root / "corpus" / "sets" / "*")) + glob.glob(str(root / "corpus" / "memos" / "*")))).encode())
    return h.hexdigest()


def build_index(root: Path, cfg: dict) -> dict:
    gen_ids = set(cfg.get("generic_ids", []))
    gen_phrases = set(cfg.get("generic_phrases", []))
    skip_files = set(cfg.get("skip_files", []))
    phrases, ids, texts = set(), set(), []

    def addp(s):
        if isinstance(s, str):
            n = norm(s)
            if len(n.split()) >= 2 and len(n) >= 8 and n not in gen_phrases:
                phrases.add(n)

    def load(p):
        try:
            return json.loads(Path(p).read_text(encoding="utf-8"))
        except Exception:  # noqa: BLE001
            return None

    for u in load(root / "corpus" / "gdocs_units.json") or []:
        addp(u.get("doc_title"))
        texts.append(u.get("text") or "")
        if u.get("doc_id"):
            ids.update((u["doc_id"], u["doc_id"][:8]))
    for kbv in ("v1", "v0"):
        g = load(root / "kb" / kbv / "graph.json") or {}
        for n in g.get("nodes", []):
            t = n.get("type")
            if t == "joke":
                ids.add(n["id"])
                slug = n["id"].split(":", 1)[1]
                if "-" in slug:
                    ids.add(slug)
                addp(n.get("title"))
                for a in n.get("aliases") or []:
                    addp(a)
                if isinstance(n.get("summary"), str):
                    texts.append(n["summary"])
            elif t in ("unit", "performance"):
                texts.append(n.get("text") or "")
                addp(n.get("name"))
                b = n.get("bit")
                if isinstance(b, dict):
                    addp(b.get("title"))
                    for a in b.get("aliases") or []:
                        addp(a)
            elif t == "set":
                addp(n.get("title"))
                addp(n.get("venue"))
    for d in glob.glob(str(root / "corpus" / "sets" / "*")):
        if not os.path.isdir(d):
            continue
        b = os.path.basename(d)
        ids.add(b)
        ids.update(b.split("_", 1)[1:])
        tj = load(os.path.join(d, "transcript.json")) or {}
        texts.extend(s.get("text", "") for s in tj.get("segments", []))
        m = load(os.path.join(d, "meta.json")) or {}
        addp(m.get("title"))
        addp(m.get("venue_guess"))
        addp(m.get("show_venue"))
        for line in (m.get("description") or "").splitlines():
            addp(line)
    for d in glob.glob(str(root / "corpus" / "memos" / "*")):
        if os.path.isdir(d):
            ids.update(os.path.basename(d).split("_", 1)[1:])
            tj = load(os.path.join(d, "transcript.json")) or {}
            texts.extend(s.get("text", "") for s in tj.get("segments", []))
    v = load(root / "kb" / "creative" / "venue_addresses.json") or {}
    for k, val in v.items():
        addp(k)
        if isinstance(val, dict):
            addp(val.get("address"))
    for f in glob.glob(str(root / "kb" / "creative" / "*.json")):
        d = load(f)
        items = d.get("items") if isinstance(d, dict) else None
        if isinstance(items, dict):
            for it in items.values():
                if not isinstance(it, dict):
                    continue
                for k in ("joke", "into"):
                    if isinstance(it.get(k), str):
                        ids.add(it[k])
                addp(it.get("title"))
                if isinstance(it.get("text"), str):
                    texts.append(it["text"])
                ids.update(j for j in it.get("joke_ids") or [] if isinstance(j, str))
    pats = ["voice/*.md", "ideas/*.md", "notes/*.md", "notes/research/*.md"] + list(cfg.get("extra_text_globs", []))
    for pat in pats:
        for f in glob.glob(str(root / pat), recursive=True):
            if Path(f).name in skip_files:
                continue
            try:
                texts.append(Path(f).read_text(encoding="utf-8", errors="replace"))
            except OSError:
                continue
            stem = Path(f).stem
            if "-" in stem and stem.upper() != stem:
                ids.add(stem)
                addp(stem.replace("-", " "))
    # Drive ids from your private gsync settings
    for name in ("legal_rules.json", "gsync_docs.json"):
        d = load(HOME / name) or {}
        for k, val in d.items():
            if k.startswith("_"):
                continue
            vals = val if isinstance(val, list) else list(val.keys()) + list(val.values()) if isinstance(val, dict) else [val]
            ids.update(x for x in vals if isinstance(x, str) and re.fullmatch(r"[A-Za-z0-9_-]{25,}", x))
    sh = set()
    for t in texts:
        w = norm(t).split()
        sh.update(" ".join(w[i:i + SHINGLE]) for i in range(len(w) - SHINGLE + 1))
    ids = {i for i in ids if isinstance(i, str) and len(i) >= 8 and "." not in i and i not in gen_ids}
    return {"phrases": phrases, "ids": ids, "shingles": sh}


def get_index(cfg: dict, quiet=False, force=False):
    root = comedy_dir()
    have_data = (root / "corpus").is_dir()
    if not have_data:
        if cfg:
            sys.exit(f"guard: BLOCKED. Your private guard settings exist ({HOME / 'guard_terms.json'}) but the data "
                     f"folder {root} has no corpus/ (set COMEDY_DIR). The material check cannot run; refusing.")
        return None
    cache = HOME / "guard_cache"
    files = _sources(root, cfg)
    sig = _signature(files, root)
    pkl = cache / "index.pkl"
    if not force:
        try:
            with open(pkl, "rb") as f:
                idx = pickle.load(f)
            if idx.get("sig") == sig:
                return idx
        except (OSError, pickle.PickleError, EOFError):
            pass
    t = time.time()
    idx = build_index(root, cfg)
    idx["sig"] = sig
    ids = sorted(idx["ids"], key=len, reverse=True)
    idx["id_re"] = re.compile(r"(?<![A-Za-z0-9_-])(" + "|".join(re.escape(i) for i in ids) + r")(?![A-Za-z0-9_]|-[A-Za-z])") if ids else None
    idx["phrase_words"] = max((len(p.split()) for p in idx["phrases"]), default=0)
    cache.mkdir(parents=True, exist_ok=True)
    tmp = pkl.with_suffix(".tmp")
    with open(tmp, "wb") as f:
        pickle.dump(idx, f, protocol=pickle.HIGHEST_PROTOCOL)
    os.replace(tmp, pkl)
    if not quiet:
        print(f"guard: index rebuilt in {time.time() - t:.1f}s ({len(idx['phrases'])} titles, {len(idx['ids'])} ids, "
              f"{len(idx['shingles'])} 7-word runs) -> {cache}", file=sys.stderr)
    return idx


# ---- scanning ------------------------------------------------------------------------------------------------------
class Scanner:
    def __init__(self, cfg: dict, idx):
        self.cfg, self.idx = cfg, idx
        self.terms = [(p, re.compile(p, re.I)) for p in cfg.get("terms", [])]
        self.author_ok_files = set(cfg.get("author_ok_files", []))
        self.author_ok = re.compile(cfg["author_ok_pattern"], re.I) if cfg.get("author_ok_pattern") else None
        self.term_ok = [re.compile(p) for p in cfg.get("term_ok_patterns", [])]
        # exact public identifiers that look like secrets (e.g. an EAS project id): listed one by one, never a wildcard
        self.secret_ok = [re.compile(p) for p in cfg.get("secret_ok_patterns", [])]

    def path_hits(self, path: str):
        p = path.replace("\\", "/")
        if NEVER_COMMIT.search(p) and not NEVER_COMMIT_OK.search(p):
            return [("NEVER-COMMIT FILE", p)]
        return []

    def binary_hits(self, path: str, raw: bytes):
        hits = []
        for m in re.finditer(rb"(tEXt|iTXt|zTXt|eXIf)(.{0,120})", raw, re.S):
            if not m.group(2).startswith((b"Software", b"Raw profile", b"XML:com.adobe")):
                hits.append(("IMAGE-TEXT", m.group(2)[:60].decode("latin-1")))
        if self.idx and self.idx.get("id_re"):
            for m in self.idx["id_re"].finditer(raw.decode("latin-1")):
                hits.append(("YOUR ID IN BINARY", m.group(0)))
        return hits

    def text_hits(self, path: str, block: list[tuple[int, str]]):
        """block: consecutive added lines [(lineno, text)]. Returns [(lineno, kind, match)]."""
        out = []
        rel = path.replace("\\", "/")
        for ln, line in block:
            txt = line
            if "%" in txt:
                dec = urllib.parse.unquote(txt)
                if dec != txt:
                    txt = txt + "\n" + dec
            for name, rx in SECRET_RX:
                if rel.endswith("package-lock.json") and "48" in name:
                    continue
                for m in rx.finditer(txt):
                    s = m.group(0)
                    ctx = txt[max(0, m.start() - 60):m.end() + 20]
                    if FAKE.search(s) or FAKE.search(txt[m.start():m.end() + 30]):
                        continue
                    if any(o.fullmatch(s) for o in self.secret_ok):
                        continue
                    if name.startswith("(?i)(secret"):
                        val = re.split(r"[:=]\s*[\"']?", s, maxsplit=1)[-1]
                        if re.fullmatch(r"[a-z_]+|[A-Z_]+|[A-Za-z_]+\(?", val) or val.startswith(("os.environ", "process.env")):
                            continue
                    if name.startswith(r"\b(?:\d") and (IP_OK.match(s) or re.search(r"(?i)version|v\d", ctx)):
                        continue
                    if "[0-9a-f]" in name and re.search(r"(?i)sha|digest|hash|md5|integrity|onnx|model|checksum", ctx):   # a lockfile's `checksum = "…"` is a public hash
                        continue
                    if "48" in name and (re.search(r"(?i)sha512-|sha256:|data:image|base64,|integrity", ctx)
                                         or not (re.search(r"\d", s) and re.search(r"[A-Z]", s))):
                        continue
                    out.append((ln, "SECRET", s[:80]))
            for name, rx in self.terms:
                for m in rx.finditer(txt):
                    if rel in self.author_ok_files and self.author_ok and self.author_ok.match(m.group(0)):
                        continue
                    if any(o.search(txt[max(0, m.start() - 40):m.end() + 40]) for o in self.term_ok):
                        continue
                    out.append((ln, "PRIVATE TERM", m.group(0)))
            if self.idx and self.idx.get("id_re"):
                for m in self.idx["id_re"].finditer(txt):
                    out.append((ln, "YOUR ID", m.group(0)))
        if self.idx:
            # titles and 7-word runs, across the whole block (a run may span lines); reported at its first line
            words, where = [], []
            for ln, line in block:
                w = norm(line).split()
                words += w
                where += [ln] * len(w)
            sh, ph = self.idx["shingles"], self.idx["phrases"]
            for i in range(len(words) - SHINGLE + 1):
                s = " ".join(words[i:i + SHINGLE])
                if s in sh:
                    out.append((where[i], "YOUR MATERIAL (7-word run)", s))
            maxw = self.idx.get("phrase_words", 0)
            for i in range(len(words)):
                for k in range(2, min(maxw, len(words) - i) + 1):
                    s = " ".join(words[i:i + k])
                    if s in ph:
                        out.append((where[i], "YOUR TITLE", s))
        seen, uniq = set(), []
        for h in out:
            if h not in seen:
                seen.add(h)
                uniq.append(h)
        return uniq


def git(*args, inp=None) -> bytes:
    r = subprocess.run(["git", *args], capture_output=True, input=inp)
    if r.returncode:
        sys.exit(f"guard: git {' '.join(args)} failed: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout


def parse_diff(diff: str):
    """-U0 unified diff -> {path: [block, ...]} with block = [(new_lineno, text), ...] of consecutive added lines,
    plus the set of binary paths."""
    files, binaries, path, block, ln = {}, set(), None, None, 0
    for line in diff.split("\n"):
        if line.startswith("diff --git "):
            path, block = None, None
        elif line.startswith("+++ "):
            p = line[4:]
            path = None if p == "/dev/null" else (p[2:] if p.startswith("b/") else p)
            if path is not None:
                files.setdefault(path, [])
        elif line.startswith("Binary files ") and " and b/" in line:
            binaries.add(line.split(" and b/", 1)[1].rsplit(" differ", 1)[0])
        elif line.startswith("@@"):
            m = re.match(r"@@ -\d+(?:,\d+)? \+(\d+)", line)
            ln = int(m.group(1)) if m else 0
            block = []
            if path is not None:
                files[path].append(block)
        elif line.startswith("+") and block is not None and path is not None:
            block.append((ln, line[1:]))
            ln += 1
    return files, binaries


def scan_diff(sc: Scanner, diff: str, blob_of, label: str):
    files, binaries = parse_diff(diff)
    hits = []
    for path in sorted(set(files) | binaries):
        for k, v in sc.path_hits(path):
            hits.append((path, 0, k, v))
        if path in binaries or path.lower().endswith(BIN):
            raw = blob_of(path)
            if raw is not None:
                hits += [(path, 0, k, v) for k, v in sc.binary_hits(path, raw)]
            continue
        for block in files.get(path, []):
            hits += [(path, ln, k, v) for ln, k, v in sc.text_hits(path, block)]
    return hits


def report(hits, label: str) -> int:
    if not hits:
        return 0
    print(f"\nguard: BLOCKED {label}: {len(hits)} hit(s). Nothing private may go into this public repo.", file=sys.stderr)
    for path, ln, kind, what in hits[:60]:
        print(f"  {path}:{ln}: {kind}: {what[:100]}", file=sys.stderr)
    if len(hits) > 60:
        print(f"  ... and {len(hits) - 60} more", file=sys.stderr)
    print("Remove or genericize those lines (git restore --staged / edit), then commit again. A real false positive:\n"
          "add a narrow pattern to term_ok_patterns, secret_ok_patterns (an exact public id) or generic_phrases in your private guard_terms.json.", file=sys.stderr)
    return 1


def check_staged(sc) -> int:
    diff = git("-c", "core.quotepath=off", "diff", "--cached", "-U0", "--no-color", "--no-ext-diff", "--diff-filter=ACMR",
               "--no-renames").decode("utf-8", "replace")
    hits = scan_diff(sc, diff, lambda p: _blob(":" + p), "staged")
    return report(hits, "commit")


def _blob(spec: str):
    r = subprocess.run(["git", "cat-file", "blob", spec], capture_output=True)
    return r.stdout if r.returncode == 0 else None


def check_commits(sc, shas: list[str], label: str) -> int:
    bad = 0
    for sha in shas:
        parents = git("rev-list", "--parents", "-n", "1", sha).decode().split()[1:]
        base = parents[0] if parents else git("hash-object", "-t", "tree", "--stdin", inp=b"").decode().strip()
        diff = git("-c", "core.quotepath=off", "diff", "-U0", "--no-color", "--no-ext-diff", "--no-renames",
                   "--diff-filter=ACMR", base, sha).decode("utf-8", "replace")
        hits = scan_diff(sc, diff, lambda p, s=sha: _blob(f"{s}:{p}"), sha[:10])
        bad += report(hits, f"{label} commit {sha[:10]}")
    return 1 if bad else 0


def check_push(sc, stdin: str) -> int:
    zero = "0" * 40
    shas = []
    for line in stdin.splitlines():
        parts = line.split()
        if len(parts) != 4:
            continue
        local_ref, local_sha, remote_ref, remote_sha = parts
        if local_sha == zero:  # deleting a ref
            continue
        if remote_sha == zero:
            rng = [local_sha, "--not", "--remotes"]
        else:
            rng = [f"{remote_sha}..{local_sha}"]
        shas += git("rev-list", "--reverse", *rng).decode().split()
    if not shas:
        return 0
    print(f"guard: checking {len(shas)} commit(s) about to be pushed", file=sys.stderr)
    return check_commits(sc, list(dict.fromkeys(shas)), "push")


def main():
    ap = argparse.ArgumentParser()
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--staged", action="store_true")
    g.add_argument("--range")
    g.add_argument("--push", action="store_true")
    g.add_argument("--files", nargs="+")
    g.add_argument("--rebuild", action="store_true")
    a = ap.parse_args()
    t0 = time.time()
    cfg = private_cfg()
    idx = get_index(cfg, force=a.rebuild)
    if a.rebuild:
        return 0
    if idx is None:
        print("guard: no data folder found: checking secrets and file names only", file=sys.stderr)
    sc = Scanner(cfg, idx)
    if a.staged:
        rc = check_staged(sc)
    elif a.range:
        shas = git("rev-list", "--reverse", a.range).decode().split()
        rc = check_commits(sc, shas, "range")
    elif a.push:
        rc = check_push(sc, sys.stdin.read())
    else:
        hits = []
        for f in a.files:
            raw = Path(f).read_bytes()
            hits += [(f, 0, k, v) for k, v in sc.path_hits(f)]
            if f.lower().endswith(BIN):
                hits += [(f, 0, k, v) for k, v in sc.binary_hits(f, raw)]
            else:
                block = list(enumerate(raw.decode("utf-8", "replace").splitlines(), 1))
                hits += [(f, ln, k, v) for ln, k, v in sc.text_hits(f, block)]
        rc = report(hits, "files")
    if os.environ.get("PK_GUARD_VERBOSE"):
        print(f"guard: done in {time.time() - t0:.2f}s", file=sys.stderr)
    return rc


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    sys.exit(main())
