"""Local SigLIP text/image embedding CLI for the Rust video adapter.

The process loads the model once. In particular, ``batch`` reads a JSON
document from stdin and embeds its stills in GPU-sized chunks before returning
one JSON document on stdout. Model files are opened with ``local_files_only``;
discovery pixels and query text are never sent over the network.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any

import torch
import torch.nn.functional as functional
from PIL import Image, ImageOps
from transformers import AutoModel, AutoProcessor


class SiglipEmbedder:
    """One locally loaded SigLIP model and its pinned preprocessing."""

    def __init__(self, model_dir: Path, device: str, batch_size: int) -> None:
        if not model_dir.is_dir():
            raise ValueError(f"model directory does not exist: {model_dir}")
        if batch_size < 1:
            raise ValueError("batch size must be positive")
        if device == "cuda" and not torch.cuda.is_available():
            raise RuntimeError("CUDA was requested but torch.cuda.is_available() is false")
        self.device = torch.device(device)
        self.batch_size = batch_size
        dtype = torch.float16 if self.device.type == "cuda" else torch.float32
        self.processor = AutoProcessor.from_pretrained(
            model_dir,
            local_files_only=True,
            use_fast=False,
        )
        self.model = (
            AutoModel.from_pretrained(
                model_dir,
                local_files_only=True,
                dtype=dtype,
            )
            .eval()
            .to(self.device)
        )

    def images(self, paths: list[Path]) -> list[list[float]]:
        vectors: list[list[float]] = []
        for offset in range(0, len(paths), self.batch_size):
            chunk = paths[offset : offset + self.batch_size]
            images = [self._read_image(path) for path in chunk]
            inputs = self.processor(images=images, return_tensors="pt").to(self.device)
            with torch.inference_mode():
                features = self.model.get_image_features(**inputs)
            vectors.extend(self._normalized(features))
        return vectors

    def text(self, value: str) -> list[float]:
        return self.texts([value])[0]

    def texts(self, values: list[str]) -> list[list[float]]:
        """Embed several queries without reloading the model."""
        cleaned = [value.strip() for value in values]
        if not cleaned or any(not value for value in cleaned):
            raise ValueError("query text is empty")
        vectors: list[list[float]] = []
        for offset in range(0, len(cleaned), self.batch_size):
            chunk = cleaned[offset : offset + self.batch_size]
            inputs = self.processor(
                text=chunk,
                padding="max_length",
                truncation=True,
                max_length=64,
                return_tensors="pt",
            ).to(self.device)
            with torch.inference_mode():
                features = self.model.get_text_features(**inputs)
            vectors.extend(self._normalized(features))
        return vectors

    def pair_calibration(self) -> tuple[float, float]:
        """Return the checkpoint's learned cosine scale and sigmoid bias."""
        logit_scale = getattr(self.model, "logit_scale", None)
        logit_bias = getattr(self.model, "logit_bias", None)
        if logit_scale is None or logit_bias is None:
            raise RuntimeError("model does not expose SigLIP pair calibration")
        scale = float(logit_scale.detach().float().exp().cpu().item())
        bias = float(logit_bias.detach().float().cpu().item())
        if not torch.isfinite(torch.tensor([scale, bias])).all().item() or scale <= 0.0:
            raise RuntimeError("model exposes invalid SigLIP pair calibration")
        return scale, bias

    @staticmethod
    def _read_image(path: Path) -> Image.Image:
        if not path.is_file():
            raise ValueError(f"still does not exist: {path}")
        with Image.open(path) as image:
            return ImageOps.exif_transpose(image).convert("RGB")

    @staticmethod
    def _normalized(features: torch.Tensor) -> list[list[float]]:
        normalized = functional.normalize(features.float(), p=2, dim=-1)
        if not bool(torch.isfinite(normalized).all()):
            raise RuntimeError("model produced a non-finite embedding")
        return normalized.cpu().tolist()


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description="Embed local stills and text with SigLIP")
    result.add_argument(
        "--model",
        type=Path,
        default=os.environ.get("EVIDENCE_SIGLIP_MODEL"),
        help="local Hugging Face model directory",
    )
    result.add_argument(
        "--device",
        default=os.environ.get(
            "EVIDENCE_SIGLIP_DEVICE", "cuda" if torch.cuda.is_available() else "cpu"
        ),
    )
    result.add_argument(
        "--batch-size",
        type=int,
        default=int(os.environ.get("EVIDENCE_SIGLIP_BATCH_SIZE", "16")),
    )
    commands = result.add_subparsers(dest="command", required=True)
    still = commands.add_parser("still", help="embed one image")
    still.add_argument("path", type=Path)
    query = commands.add_parser("query", help="embed one text query")
    query.add_argument("text")
    commands.add_parser("batch", help="read {paths:[...]} from stdin")
    return result


def run(arguments: argparse.Namespace) -> dict[str, Any]:
    if arguments.model is None:
        raise ValueError("pass --model or set EVIDENCE_SIGLIP_MODEL")
    embedder = SiglipEmbedder(arguments.model, arguments.device, arguments.batch_size)
    if arguments.command == "still":
        return {"vector": embedder.images([arguments.path])[0]}
    if arguments.command == "query":
        return {"vector": embedder.text(arguments.text)}
    request = json.load(sys.stdin)
    paths = request.get("paths")
    if not isinstance(paths, list) or not all(isinstance(path, str) for path in paths):
        raise ValueError("batch input must be a JSON object with a string paths array")
    return {"vectors": embedder.images([Path(path) for path in paths])}


def main() -> int:
    try:
        arguments = parser().parse_args()
        json.dump(run(arguments), sys.stdout, separators=(",", ":"), allow_nan=False)
        sys.stdout.write("\n")
        return 0
    except Exception as error:  # noqa: BLE001 - CLI boundary reports a concise hard failure.
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
