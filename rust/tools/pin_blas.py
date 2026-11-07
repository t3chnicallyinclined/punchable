"""Import this FIRST in every gen_*_goldens.py (before anything imports numpy).

numpy's wheels carry OpenBLAS built with DYNAMIC_ARCH: the float32 `dot` / `norm` kernel is chosen from the CPU at
import time (Haswell on an AVX2 box, SkylakeX on an AVX-512 box, ...), and each sums in a different order, so the
cosines of creative/data.py differ in the 9th digit between machines (the CI `goldens-current` job on an AVX-512
runner vs a Haswell-class PC: 134 changed numbers in corpus/notebook.json and merge/merge.json). Pinning the kernel
to Haswell (which every x86-64 CPU with AVX2 runs, including AVX-512 ones) makes the generators machine-independent.
The Rust tests compare cosines at 1e-5, so the pin is for the drift check, not for correctness.
"""
import os

os.environ.setdefault("OPENBLAS_CORETYPE", "Haswell")
