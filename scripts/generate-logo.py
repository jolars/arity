#!/usr/bin/env python3
"""Generate Arity logo trials with an OpenAI image model.

Edit the prompt in ``scripts/logo-prompt.txt`` (or pass another file with
``-p``), then run this to drop trial PNGs under ``images/trials/``. Iterate by
tweaking the prompt and re-running; each run is timestamped so nothing is
overwritten.

Examples
--------
    python scripts/generate-logo.py                       # default prompt, 2 images
    python scripts/generate-logo.py -n 4                  # four trials
    python scripts/generate-logo.py -p scripts/logo-prompt-juggle.txt
    python scripts/generate-logo.py -s 1536x1024 -q high  # wide, high quality

Requirements
------------
* ``OPENAI_API_KEY`` in the environment.
* The ``openai`` package (``pip install openai``).

Notes
-----
* ``gpt-image-2`` renders on a white background only (no transparency). Flood
  fill the white away before tracing to SVG. Use ``-m gpt-image-1`` if you want
  a transparent background instead.
"""

from __future__ import annotations

import argparse
import base64
import sys
from datetime import datetime
from pathlib import Path

from openai import OpenAI

DEFAULT_PROMPT = Path("scripts/logo-prompt.txt")
DEFAULT_OUT_DIR = Path("images/trials")


def main() -> int:
    ap = argparse.ArgumentParser(description="Generate Arity logo trials.")
    ap.add_argument(
        "-p",
        "--prompt",
        type=Path,
        default=DEFAULT_PROMPT,
        help=f"prompt file to read (default: {DEFAULT_PROMPT})",
    )
    ap.add_argument(
        "-n", "--count", type=int, default=1, help="number of images (default: 1)"
    )
    ap.add_argument(
        "-m",
        "--model",
        default="gpt-image-2",
        help="image model (default: gpt-image-2)",
    )
    ap.add_argument(
        "-s",
        "--size",
        default="1024x1024",
        help="1024x1024 | 1536x1024 | 1024x1536 (default: 1024x1024)",
    )
    ap.add_argument(
        "-q",
        "--quality",
        default="medium",
        help="low | medium | high (default: medium)",
    )
    ap.add_argument(
        "--prefix",
        default=None,
        help="output filename prefix (default: the prompt file's stem)",
    )
    ap.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    args = ap.parse_args()

    if not args.prompt.exists():
        print(f"error: prompt file not found: {args.prompt}", file=sys.stderr)
        return 1

    prompt = args.prompt.read_text().strip()
    if not prompt:
        print(f"error: prompt file is empty: {args.prompt}", file=sys.stderr)
        return 1

    prefix = args.prefix or args.prompt.stem
    args.out_dir.mkdir(parents=True, exist_ok=True)

    print(f"model={args.model} n={args.count} size={args.size} quality={args.quality}")
    print(f"prompt={args.prompt} ({len(prompt)} chars) -> {args.out_dir}/")

    client = OpenAI()
    try:
        result = client.images.generate(
            model=args.model,
            prompt=prompt,
            n=args.count,
            size=args.size,
            quality=args.quality,
            output_format="png",
        )
    except Exception as exc:  # surface API errors plainly and keep going next run
        print(f"error: image generation failed: {exc}", file=sys.stderr)
        return 1

    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    wrote = 0
    for i, item in enumerate(result.data or []):
        if not getattr(item, "b64_json", None):
            print(f"  [{i}] no image data returned", file=sys.stderr)
            continue
        out = args.out_dir / f"{prefix}-{stamp}-{i}.png"
        out.write_bytes(base64.b64decode(item.b64_json))
        print(f"  wrote {out}")
        wrote += 1

    if not wrote:
        print("error: no images written", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
