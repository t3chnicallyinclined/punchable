"""Run tools/laughs/detect_laughs.py on ONE set with redirected folders.

Runs inside the laughs venv (torch + panns). detect_laughs.py has its corpus and
tapes folders as module constants; this points them at the inbox run's folders
and calls its own detect(), unchanged.
"""
import argparse
import datetime as dt
import importlib.util
import sys
from pathlib import Path


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tools", required=True)
    ap.add_argument("--set", required=True)
    ap.add_argument("--tapes", required=True)
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--run-id", default=None)
    ap.add_argument("--cpu", action="store_true")
    a = ap.parse_args()
    path = Path(a.tools) / "laughs" / "detect_laughs.py"
    spec = importlib.util.spec_from_file_location("detect_laughs", str(path))
    dl = importlib.util.module_from_spec(spec)
    sys.modules["detect_laughs"] = dl
    spec.loader.exec_module(dl)
    dl.CORPUS = Path(a.corpus)
    dl.TAPES = Path(a.tapes)
    import torch
    device = "cuda" if torch.cuda.is_available() and not a.cpu else "cpu"
    try:
        from importlib.metadata import version
        pv = version("panns_inference")
    except Exception:
        pv = "?"
    detector = f"panns_inference@{pv}:{dl.CKPT_NAME}+transcript_fusion@1"
    run_id = a.run_id or ("auto-" + dt.datetime.now().strftime("%Y%m%d-%H%M") + "-laughs")
    model = dl.build_model(device)
    d = dl.detect(a.set, model, device, dl.load_labels(), run_id, detector)
    print(f"{a.set}: {len(d['laughs'])} laughs on {device}")


if __name__ == "__main__":
    main()
