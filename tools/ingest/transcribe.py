"""G2 step 2: transcribe 16 kHz WAVs with faster-whisper large-v3 on the GPU.

Usage: python transcribe.py <tapes_raw_dir> <corpus_sets_dir> [--device cuda|cpu] [--force]
           [--sets SET ...] [--vad whisper|silero-clips] [--gap-fill] [--gap-fill-only] [--batch-size 16]

For each <tapes_raw_dir>/<set>/audio16k.wav writes <corpus_sets_dir>/<set>/:
  transcript.json  {model, device, compute_type, audio_sec, compute_sec, rtf, segments:[{start,end,text,words:[{w,start,end,prob}]}]}
  transcript.md    one line per segment, "[mm:ss] text"
rtf = seconds of audio per second of compute (higher is faster).

VAD (tools/audio/vad.py, Silero v5 ONNX on the shared decoded PCM):
  --vad whisper       (default) faster-whisper's own Silero VAD, sequential decoding, with the
                      caption-count / long-gap fallback to VAD off. The canonical path.
  --vad silero-clips  our Silero v5 speech regions, packed into <=30 s clips, fed to
                      BatchedInferencePipeline as clip_timestamps (batched decoding).
  --gap-fill          after either pass, re-run Whisper on every stretch where Silero hears
                      >= 1 s of speech but Whisper has no word, keep what passes the
                      hallucination guards, and merge it in (segments marked "gap_fill": true).
  --gap-fill-only     gap-fill existing transcript.json files in place (no full re-run).
Coverage = share of Silero speech time with a Whisper word within 0.25 s; recorded before and
after under "vad_coverage".
"""
import json
import os
import re
import sys
import time
import wave
from pathlib import Path

SITE = Path(os.environ.get("APPDATA", "")) / "Python" / "Python313" / "site-packages" / "nvidia"
for sub in ("cublas", "cudnn", "cuda_nvrtc"):
    b = SITE / sub / "bin"
    if b.is_dir():
        os.add_dll_directory(str(b))
        os.environ["PATH"] = str(b) + os.pathsep + os.environ["PATH"]

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))  # punchable root, for tools.audio

from faster_whisper import WhisperModel  # noqa: E402

MODEL = "large-v3"
MAX_CLIP_S = 30.0

# Gap-fill hallucination guards (Whisper's own thresholds, plus phrases it invents on noise/applause)
NO_SPEECH_MAX = 0.6
AVG_LOGPROB_MIN = -1.0
COMPRESSION_MAX = 2.4
# Set from the first gap-fill pass over all 17 sets (2026-10-26): one-word gap segments at
# p < 0.5 were noise ("speech." twice at p 0.03-0.07, "lungs." 0.02, "work." 0.04, "I" 0.15);
# every multi-word segment above 0.30 read as a real line.
SINGLE_WORD_PROB_MIN = 0.50
MEAN_WORD_PROB_MIN = 0.30
HALLUCINATIONS = {"thank you.", "thank you", "thanks for watching!", "thanks for watching.", "you", "bye.",
                  "thank you very much.", "so", "okay.", "music", "[music]", "applause", "(applause)", "(laughs)",
                  "(laughter)", "subtitles by the amara.org community"}
# A gap is often the room laughing; Whisper writes that as "Ha, ha, ha." Never a word of the set.
LAUGH_TOKEN = re.compile(r"(?:h+[aeiou]+h*)+|(?:ah)+|(?:heh)+|(?:hm+)|(?:o+h+)|(?:wo+)+")


def fmt(t):
    t = int(t)
    return f"{t // 60:02d}:{t % 60:02d}"


def seg_dict(s, extra=None):
    d = {"start": round(s.start, 2), "end": round(s.end, 2), "text": s.text.strip(),
         "words": [{"w": x.word.strip(), "start": round(x.start, 2), "end": round(x.end, 2),
                    "prob": round(x.probability, 3)} for x in (s.words or [])]}
    if extra:
        d.update(extra)
    return d


def run(model, wav, vad):
    segs, _ = model.transcribe(wav, language="en", word_timestamps=True, vad_filter=vad, beam_size=5,
                               condition_on_previous_text=False)
    return [seg_dict(s) for s in segs]


def pack_clips(regions, max_s=MAX_CLIP_S, join_s=1.0):
    """Silero regions -> clips <= max_s, merging neighbours closer than join_s while they fit."""
    clips = []
    for a, b in regions:
        while b - a > max_s:  # a single region longer than a clip: split it
            clips.append([a, a + max_s])
            a += max_s
        if clips and a - clips[-1][1] < join_s and b - clips[-1][0] <= max_s:
            clips[-1][1] = b
        else:
            clips.append([a, b])
    return clips


def run_silero_clips(model, audio, regions, batch_size=16):
    from faster_whisper import BatchedInferencePipeline
    pipe = BatchedInferencePipeline(model=model)
    clips = [{"start": a, "end": b} for a, b in pack_clips(regions)]
    segs, _ = pipe.transcribe(audio, language="en", word_timestamps=True, beam_size=5, batch_size=batch_size,
                              clip_timestamps=clips, condition_on_previous_text=False, without_timestamps=True)
    return [seg_dict(s) for s in segs]


def guard(s):
    """None if the segment passes the hallucination guards, else the reason."""
    t = s.text.strip().lower()
    if not t or not s.words:
        return "empty"
    if s.no_speech_prob > NO_SPEECH_MAX and s.avg_logprob < AVG_LOGPROB_MIN:
        return f"no_speech {s.no_speech_prob:.2f} with avg_logprob {s.avg_logprob:.2f}"
    if s.avg_logprob < AVG_LOGPROB_MIN - 0.5:
        return f"avg_logprob {s.avg_logprob:.2f}"
    if s.compression_ratio > COMPRESSION_MAX:
        return f"compression {s.compression_ratio:.2f}"
    if t in HALLUCINATIONS:
        return f"known hallucination {t!r}"
    toks = re.findall(r"[a-z']+", t)
    if toks and all(LAUGH_TOKEN.fullmatch(x) for x in toks):
        return "laughter transcribed as words"
    probs = [w.probability for w in s.words]
    mean_p = sum(probs) / len(probs)
    if len(probs) == 1 and mean_p < SINGLE_WORD_PROB_MIN:
        return f"single word at p={mean_p:.2f}"
    if mean_p < MEAN_WORD_PROB_MIN:
        return f"mean word p={mean_p:.2f}"
    return None


def gap_fill(model, audio, segments, regions, audio_sec):
    """Re-run Whisper on VAD speech with no words; returns (segments, report)."""
    from tools.audio import vad as V
    words = V.word_spans({"segments": segments})
    before = V.coverage(regions, words)
    gaps = V.gaps(regions, words, audio_s=audio_sec)
    added, rejected = [], []
    if gaps:
        flat = [round(x, 2) for g in gaps for x in g]
        segs, _ = model.transcribe(audio, language="en", word_timestamps=True, vad_filter=False, beam_size=5,
                                   condition_on_previous_text=False, clip_timestamps=flat)
        for s in segs:
            why = guard(s)
            # a gap segment must land inside a gap (Whisper can drift a word into the neighbour)
            mid = (s.start + s.end) / 2
            if why is None and not any(a - 0.3 <= mid <= b + 0.3 for a, b in gaps):
                why = "outside gap"
            if why is None:
                d = seg_dict(s, {"gap_fill": True})
                # never duplicate a word Whisper already had at that time
                d["words"] = [w for w in d["words"] if not any(abs(w["start"] - a) < 0.15 and abs(w["end"] - b) < 0.15
                                                                  for a, b in words)]
                if d["words"]:
                    added.append(d)
            else:
                rejected.append({"start": round(s.start, 2), "end": round(s.end, 2), "text": s.text.strip(), "why": why})
    merged = sorted(segments + added, key=lambda s: (s["start"], s["end"]))
    after = V.coverage(regions, V.word_spans({"segments": merged}))
    rep = {"vad": "silero_vad_v5", "gaps": [[round(a, 2), round(b, 2)] for a, b in gaps],
           "added_segments": len(added), "added_words": sum(len(s["words"]) for s in added),
           "rejected": rejected, "coverage_before": before, "coverage_after": after}
    return merged, rep


def caption_word_count(vtt: Path):
    """Words in YouTube rolling auto captions (new last line of each non-freeze cue only)."""
    if not vtt.exists():
        return None
    lines = vtt.read_text(encoding="utf-8", errors="replace").splitlines()
    n, i = 0, 0
    while i < len(lines):
        m = re.match(r"(\d+):(\d\d):(\d\d\.\d+) --> (\d+):(\d\d):(\d\d\.\d+)", lines[i])
        i += 1
        if not m:
            continue
        a = int(m[1]) * 3600 + int(m[2]) * 60 + float(m[3])
        b = int(m[4]) * 3600 + int(m[5]) * 60 + float(m[6])
        body = []
        while i < len(lines) and lines[i] != "":
            if lines[i].strip():
                body.append(lines[i])
            i += 1
        if b - a >= 0.05 and body:
            last = re.sub(r"<[^>]+>|\[[^\]]*\]", " ", body[-1])
            n += len(re.findall(r"[A-Za-z0-9']+", last))
    return n


def gap_seconds(segments, audio_sec, min_gap=15.0):
    edges = [0.0] + [x for s in segments for x in (s["start"], s["end"])] + [audio_sec]
    return sum(b - a for a, b in zip(edges[::2], edges[1::2]) if b - a > min_gap)


def arg(name, default=None):
    if name in sys.argv:
        i = sys.argv.index(name)
        return sys.argv[i + 1]
    return default


def arglist(name):
    if name not in sys.argv:
        return None
    i, out = sys.argv.index(name) + 1, []
    while i < len(sys.argv) and not sys.argv[i].startswith("--"):
        out.append(sys.argv[i])
        i += 1
    return out


def write(out, d, doc, segments):
    (out / "transcript.json").write_text(json.dumps(doc, indent=1, ensure_ascii=False), encoding="utf-8")
    (out / "transcript.md").write_text(
        f"# {d.name}\n\n" + "\n".join(f"[{fmt(s['start'])}] {s['text']}" for s in segments) + "\n", encoding="utf-8")


def main():
    raw, sets = Path(sys.argv[1]), Path(sys.argv[2])
    device = "cpu" if "--cpu" in sys.argv else "cuda"
    force = "--force" in sys.argv
    vad_mode = arg("--vad", "whisper")
    do_gap = "--gap-fill" in sys.argv
    gap_only = "--gap-fill-only" in sys.argv
    batch_size = int(arg("--batch-size", "16"))
    only = arglist("--sets")
    compute_type = "float16" if device == "cuda" else "int8"
    t0 = time.time()
    model = WhisperModel(MODEL, device=device, compute_type=compute_type)
    print(f"model loaded on {device}/{compute_type} in {time.time() - t0:.1f}s", flush=True)
    sil = None
    if vad_mode == "silero-clips" or do_gap or gap_only:
        from tools.audio import pcm as P, vad as V
        sil = V.Silero()
    tot_audio = tot_compute = 0.0
    for d in sorted(p for p in raw.iterdir() if (p / "audio16k.wav").exists()):
        if only and d.name not in only:
            continue
        out = sets / d.name
        if gap_only:
            tp = out / "transcript.json"
            if not tp.exists():
                print("no transcript, skip", d.name)
                continue
            doc = json.loads(tp.read_text(encoding="utf-8"))
            if doc.get("gap_fill") and not force:
                print("already gap-filled, skip", d.name)
                continue
            doc["segments"] = [s for s in doc["segments"] if not s.get("gap_fill")]  # idempotent re-run
            t = time.time()
            audio, _ = P.load(path=d / "audio16k.wav")
            audio = audio.__array__()
            vt = time.time()
            regions = V.speech_timestamps(sil.probs(audio), len(audio))
            vad_s = time.time() - vt
            segs, rep = gap_fill(model, audio, doc["segments"], regions, doc["audio_sec"])
            rep["vad_s"] = round(vad_s, 2)
            rep["compute_s"] = round(time.time() - t, 2)
            doc["segments"], doc["gap_fill"] = segs, rep
            doc["vad_coverage"] = {"before": rep["coverage_before"], "after": rep["coverage_after"]}
            write(out, d, doc, segs)
            print(f"{d.name}: gap-fill +{rep['added_words']} words in {rep['added_segments']} segments "
                  f"({len(rep['gaps'])} gaps, {len(rep['rejected'])} rejected), coverage "
                  f"{rep['coverage_before']['coverage']} -> {rep['coverage_after']['coverage']} in {rep['compute_s']}s", flush=True)
            continue
        if (out / "transcript.json").exists() and not force:
            print("skip", d.name)
            continue
        out.mkdir(parents=True, exist_ok=True)
        with wave.open(str(d / "audio16k.wav")) as w:
            audio_sec = w.getnframes() / w.getframerate()
        t = time.time()
        wav = str(d / "audio16k.wav")
        regions = None
        if sil is not None:
            audio, _ = P.load(path=d / "audio16k.wav")
            audio = audio.__array__()
            regions = V.speech_timestamps(sil.probs(audio), len(audio))
        vad_used, fallback = True, None
        if vad_mode == "silero-clips":
            segments = run_silero_clips(model, audio, regions, batch_size)
        else:
            segments = run(model, wav, vad=True)
            cap_words = caption_word_count(d / "captions.en.vtt")
            hyp_words = sum(len(s["words"]) for s in segments)
            gap = gap_seconds(segments, audio_sec)
            if cap_words and hyp_words < 0.85 * cap_words:
                fallback = f"VAD pass gave {hyp_words} words vs {cap_words} caption words; re-ran with VAD off"
            elif gap > 0.10 * audio_sec:
                fallback = f"VAD pass left {gap:.0f}s in gaps >15s; re-ran with VAD off"
            if fallback:
                print("  fallback:", fallback, flush=True)
                segments = run(model, wav, vad=False)
                vad_used = False
        rep = None
        if do_gap:
            segments, rep = gap_fill(model, audio, segments, regions, audio_sec)
        compute = time.time() - t
        tot_audio += audio_sec
        tot_compute += compute
        doc = {"model": MODEL, "engine": "faster-whisper", "device": device, "compute_type": compute_type,
               "language": "en", "vad_filter": vad_used if vad_mode == "whisper" else "silero_v5_clip_timestamps",
               "vad_fallback": fallback, "word_timestamps": True, "beam_size": 5,
               "condition_on_previous_text": False,
               "audio_sec": round(audio_sec, 1), "compute_sec": round(compute, 1),
               "rtf": round(audio_sec / compute, 1), "segments": segments}
        if vad_mode == "silero-clips":
            doc["batched"] = {"batch_size": batch_size, "clips": len(pack_clips(regions))}
        if rep:
            doc["gap_fill"] = rep
            doc["vad_coverage"] = {"before": rep["coverage_before"], "after": rep["coverage_after"]}
        elif regions is not None:
            from tools.audio import vad as V
            doc["vad_coverage"] = {"before": V.coverage(regions, V.word_spans({"segments": segments}))}
        write(out, d, doc, segments)
        print(f"{d.name}: {audio_sec:.0f}s audio in {compute:.1f}s (rtf {audio_sec / compute:.1f}), "
              f"{len(segments)} segments" + (f", gap-fill +{rep['added_words']} words, coverage "
                                             f"{rep['coverage_before']['coverage']} -> {rep['coverage_after']['coverage']}" if rep else ""),
              flush=True)
    if tot_compute:
        print(f"TOTAL rtf {tot_audio / tot_compute:.1f} ({tot_audio:.0f}s audio / {tot_compute:.1f}s compute)")


if __name__ == "__main__":
    main()
