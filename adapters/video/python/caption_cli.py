"""Fully local, load-once Qwen-style scene-caption CLI.

The ``batch`` command reads ``{"paths":[...]}`` from stdin, loads the model
once, captions each still independently, and writes ``{"captions":[...]}``.
Each item is either one deterministically rendered visible-feature index or
null. Free-form model prose never enters the case.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path
from typing import Any


VISIBLE_TERMS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("multiple people", ("people", "personnel", "group", "several persons")),
    ("person", ("person", "man", "woman", "individual", "officer")),
    ("camouflage clothing", ("camouflage", "camo")),
    ("reflective clothing", ("reflective clothing", "reflective stripes")),
    ("helmet", ("helmet",)),
    ("stretcher", ("stretcher",)),
    ("ambulance", ("ambulance",)),
    ("fire truck", ("fire truck", "fire engine")),
    ("vehicle interior", ("vehicle interior", "inside a vehicle")),
    ("vehicle", ("vehicle", "car", "truck", "suv", "van")),
    ("building", ("building", "storefront")),
    ("road", ("road", "street")),
    ("sidewalk", ("sidewalk", "pavement")),
    ("open doorway", ("open door", "doorway")),
    ("camera", ("camera",)),
    ("smoke", ("smoke",)),
    ("visible flames", ("visible flame", "flames", "burning")),
    ("hose", ("hose",)),
    ("debris", ("debris",)),
    ("snow", ("snow", "snowy")),
    ("water", ("water",)),
    ("low light", ("low light", "dark scene", "nighttime")),
    ("obstructed view", ("obstructed", "blocked view")),
    ("blurred view", ("blurred", "blurry")),
)

DEFAULT_PROMPT = """Write exactly one short plain-text sentence describing this single video frame as a navigation hint for a lawyer.

State only coarse features directly visible in the frame: clothing, objects,
setting, and spatial relationships. Prefer visible appearance over assigning a
role. Do not identify a person, organization, unit, location, insignia, vehicle
owner, or event. Do not infer intent, cause, sequence, speed, authenticity, or
anything before or after the frame. Return one sentence only, with no JSON,
list, explanation, or timestamp. If the scene cannot be described
conservatively, answer exactly ABSTAIN."""


def normalize_caption(raw: str) -> str | None:
    text = " ".join(raw.strip().split())
    if not text or text.upper() == "ABSTAIN":
        return None
    lowered = text.casefold()
    visible = [
        canonical
        for canonical, aliases in VISIBLE_TERMS
        if any(re.search(rf"\b{re.escape(alias)}\b", lowered) for alias in aliases)
    ]
    if "multiple people" in visible and "person" in visible:
        visible.remove("person")
    if "vehicle interior" in visible and "vehicle" in visible:
        visible.remove("vehicle")
    if "fire truck" in visible and "vehicle" in visible:
        visible.remove("vehicle")
    if not visible:
        return None
    return f"Visible navigation terms: {', '.join(visible)}."


class LocalCaptioner:
    """One resident local image-text model used sequentially for bounded VRAM."""

    def __init__(self, model_dir: Path, device: str, torch_dtype: str, max_new_tokens: int) -> None:
        import torch
        from transformers import AutoModelForImageTextToText, AutoProcessor

        if not model_dir.is_dir():
            raise ValueError(f"model directory does not exist: {model_dir}")
        if max_new_tokens < 1:
            raise ValueError("max-new-tokens must be positive")
        if device == "cuda" and not torch.cuda.is_available():
            raise RuntimeError("CUDA was requested but torch.cuda.is_available() is false")
        dtype = {
            "float16": torch.float16,
            "bfloat16": torch.bfloat16,
            "float32": torch.float32,
        }.get(torch_dtype)
        if dtype is None:
            raise ValueError("torch-dtype must be float16, bfloat16, or float32")
        self.device = torch.device(device)
        self.max_new_tokens = max_new_tokens
        self.processor = AutoProcessor.from_pretrained(
            model_dir,
            local_files_only=True,
            trust_remote_code=False,
        )
        self.model = (
            AutoModelForImageTextToText.from_pretrained(
                model_dir,
                local_files_only=True,
                trust_remote_code=False,
                dtype=dtype,
            )
            .eval()
            .to(self.device)
        )

    def caption(self, path: Path, prompt: str) -> str | None:
        import torch

        if not path.is_file():
            raise ValueError(f"still does not exist: {path}")
        messages = [
            {
                "role": "user",
                "content": [
                    {"type": "image", "image": str(path)},
                    {"type": "text", "text": prompt},
                ],
            }
        ]
        inputs = self.processor.apply_chat_template(
            messages,
            add_generation_prompt=True,
            tokenize=True,
            return_dict=True,
            return_tensors="pt",
        ).to(self.device)
        input_length = inputs["input_ids"].shape[-1]
        with torch.inference_mode():
            generated = self.model.generate(
                **inputs,
                max_new_tokens=self.max_new_tokens,
                do_sample=False,
            )
        raw = self.processor.batch_decode(
            generated[:, input_length:],
            skip_special_tokens=True,
            clean_up_tokenization_spaces=False,
        )[0]
        return normalize_caption(raw)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--model", type=Path, required=True)
    result.add_argument(
        "--device",
        default=os.environ.get("EVIDENCE_VLM_DEVICE", "cuda"),
    )
    result.add_argument(
        "--torch-dtype",
        default=os.environ.get("EVIDENCE_VLM_DTYPE", "float16"),
    )
    result.add_argument(
        "--max-new-tokens",
        type=int,
        default=int(os.environ.get("EVIDENCE_VLM_MAX_NEW_TOKENS", "64")),
    )
    result.add_argument("--prompt", default=DEFAULT_PROMPT)
    commands = result.add_subparsers(dest="command", required=True)
    still = commands.add_parser("still")
    still.add_argument("path", type=Path)
    commands.add_parser("batch")
    return result


def run(arguments: argparse.Namespace) -> dict[str, Any]:
    captioner = LocalCaptioner(
        arguments.model,
        arguments.device,
        arguments.torch_dtype,
        arguments.max_new_tokens,
    )
    if arguments.command == "still":
        return {"caption": captioner.caption(arguments.path, arguments.prompt)}
    request = json.load(sys.stdin)
    paths = request.get("paths")
    if not isinstance(paths, list) or not all(isinstance(path, str) for path in paths):
        raise ValueError("batch input must be a JSON object with a string paths array")
    return {
        "captions": [captioner.caption(Path(path), arguments.prompt) for path in paths]
    }


def main() -> int:
    try:
        arguments = parser().parse_args()
        json.dump(run(arguments), sys.stdout, separators=(",", ":"), allow_nan=False)
        sys.stdout.write("\n")
        return 0
    except Exception as error:  # noqa: BLE001 - CLI boundary reports one concise error.
        print(f"caption CLI failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
