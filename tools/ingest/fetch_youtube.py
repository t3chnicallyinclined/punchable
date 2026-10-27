"""G2 step 1: fetch metadata, English captions and audio for YouTube sets.

Usage: py -I fetch_youtube.py <links.txt> <tapes_raw_dir>

For each URL writes into <tapes_raw_dir>/<upload_date>_<id>/:
  info.json      trimmed metadata (id, title, upload_date, duration, uploader, description, url)
  captions.en.vtt  English manual captions if present, else English auto captions
  audio.opus     compressed copy (~32 kbps)
  audio16k.wav   16 kHz mono PCM for ASR
Downloaded files are treated as data only; nothing from them is executed.
"""
import json
import shutil
import subprocess
import sys
from pathlib import Path

YTDLP = r"~\AppData\Roaming\Python\Python313\Scripts\yt-dlp"


def run(cmd):
    print(">", " ".join(cmd[:6]), "...", flush=True)
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")


def fetch(url, raw_root: Path):
    r = run([YTDLP, "--no-warnings", "--skip-download", "--dump-json", "--no-playlist", url])
    if r.returncode != 0:
        raise RuntimeError(f"metadata failed: {r.stderr[-500:]}")
    info = json.loads(r.stdout.splitlines()[-1])
    vid, date = info["id"], info.get("upload_date") or "00000000"
    out = raw_root / f"{date}_{vid}"
    out.mkdir(parents=True, exist_ok=True)
    keep = {k: info.get(k) for k in ("id", "title", "upload_date", "duration", "uploader",
                                      "channel", "description", "webpage_url")}
    keep["url"] = url
    keep["has_manual_en"] = any(k.startswith("en") for k in (info.get("subtitles") or {}))
    keep["has_auto_en"] = any(k in ("en", "en-orig") or k.startswith("en-") for k in (info.get("automatic_captions") or {}))
    (out / "info.json").write_text(json.dumps(keep, indent=2, ensure_ascii=False), encoding="utf-8")

    # Captions: prefer manual English, else auto (en-orig is the original-language ASR track).
    if not list(out.glob("captions*.vtt")):
        if keep["has_manual_en"]:
            r = run([YTDLP, "--no-warnings", "--skip-download", "--write-subs", "--sub-langs", "en.*,en",
                     "--sub-format", "vtt", "-o", str(out / "captions.%(ext)s"), url])
        elif keep["has_auto_en"]:
            r = run([YTDLP, "--no-warnings", "--skip-download", "--write-auto-subs", "--sub-langs", "en-orig,en",
                     "--sub-format", "vtt", "-o", str(out / "captions.%(ext)s"), url])
        vtts = sorted(out.glob("captions*.vtt"))
        if vtts:
            # keep one: prefer en-orig / en over translated variants
            best = next((v for v in vtts if v.name in ("captions.en-orig.vtt", "captions.en.vtt")), vtts[0])
            best_kind = "manual" if keep["has_manual_en"] else "auto"
            shutil.copyfile(best, out / "captions.en.vtt")
            keep["captions_kind"] = best_kind
            keep["captions_track"] = best.name
            (out / "info.json").write_text(json.dumps(keep, indent=2, ensure_ascii=False), encoding="utf-8")

    if not (out / "audio16k.wav").exists():
        r = run([YTDLP, "--no-warnings", "-f", "bestaudio/best", "--no-playlist",
                 "-o", str(out / "source.%(ext)s"), url])
        if r.returncode != 0:
            raise RuntimeError(f"audio failed: {r.stderr[-500:]}")
        src = next(p for p in out.glob("source.*") if p.suffix not in (".part", ".ytdl"))
        r = run(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-i", str(src), "-ac", "1", "-ar", "16000",
                 "-c:a", "pcm_s16le", str(out / "audio16k.wav")])
        if r.returncode != 0:
            raise RuntimeError(f"wav failed: {r.stderr[-500:]}")
        r = run(["ffmpeg", "-nostdin", "-y", "-loglevel", "error", "-i", str(src), "-vn", "-ac", "1",
                 "-c:a", "libopus", "-b:a", "32k", str(out / "audio.opus")])
        if r.returncode != 0:
            raise RuntimeError(f"opus failed: {r.stderr[-500:]}")
        src.unlink()
    return out


def main():
    links = [l.strip() for l in Path(sys.argv[1]).read_text().splitlines() if l.strip()]
    raw_root = Path(sys.argv[2])
    failed = []
    for u in links:
        try:
            print("OK", fetch(u, raw_root), flush=True)
        except Exception as e:  # noqa: BLE001
            print("FAIL", u, e, flush=True)
            failed.append((u, str(e)))
    print(f"done: {len(links) - len(failed)} ok, {len(failed)} failed")
    for u, e in failed:
        print("  ", u, e[:300])


if __name__ == "__main__":
    main()
