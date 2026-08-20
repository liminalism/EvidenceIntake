"""Probe whether SigLIP can produce stable neutral labels for sampled frames.

This is an evaluation tool, not an evidentiary extractor. It uses the model's
learned SigLIP sigmoid calibration and abstains unless every independent prompt
wording clears the native 0.5 pair threshold and chooses the same label. Scores
are used internally but are never written.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from benchmark_cli import fingerprint_model, load_frames, load_json


SCHEMA_VERSION = 1


def _normalize(vector: list[float]) -> list[float]:
    norm = math.sqrt(math.fsum(value * value for value in vector))
    if not math.isfinite(norm) or norm == 0.0:
        raise ValueError("cannot normalize an empty or non-finite label vector")
    return [value / norm for value in vector]


def _dot(left: list[float], right: list[float]) -> float:
    if len(left) != len(right):
        raise ValueError("frame and label vector dimensions differ")
    return math.fsum(a * b for a, b in zip(left, right))


def _sigmoid(value: float) -> float:
    if value >= 0.0:
        inverse = math.exp(-value)
        return 1.0 / (1.0 + inverse)
    exponential = math.exp(value)
    return exponential / (1.0 + exponential)


def validate_vocabulary(document: dict[str, Any]) -> list[dict[str, Any]]:
    if document.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(f"label vocabulary schema_version must be {SCHEMA_VERSION}")
    groups = document.get("groups")
    if not isinstance(groups, list) or not groups:
        raise ValueError("label vocabulary needs a non-empty groups array")
    group_ids: set[str] = set()
    for group in groups:
        if not isinstance(group, dict):
            raise ValueError("each label group must be an object")
        group_id = group.get("id")
        if not isinstance(group_id, str) or not group_id.strip() or group_id in group_ids:
            raise ValueError("label group ids must be unique non-empty strings")
        group_ids.add(group_id)
        labels = group.get("labels")
        if not isinstance(labels, list) or len(labels) < 2:
            raise ValueError(f"label group {group_id} needs at least two labels")
        label_ids: set[str] = set()
        prompt_count: int | None = None
        for label in labels:
            if not isinstance(label, dict):
                raise ValueError(f"labels in group {group_id} must be objects")
            label_id = label.get("id")
            text = label.get("text")
            prompts = label.get("prompts")
            if not isinstance(label_id, str) or not label_id or label_id in label_ids:
                raise ValueError(f"label ids in group {group_id} must be unique")
            if not isinstance(text, str) or not text.strip():
                raise ValueError(f"label {group_id}/{label_id} needs display text")
            if (
                not isinstance(prompts, list)
                or len(prompts) < 2
                or any(not isinstance(prompt, str) or not prompt.strip() for prompt in prompts)
            ):
                raise ValueError(f"label {group_id}/{label_id} needs at least two prompts")
            prompt_count = prompt_count or len(prompts)
            if len(prompts) != prompt_count:
                raise ValueError(f"all labels in group {group_id} need the same prompt count")
            label_ids.add(label_id)
    return groups


def prepare_groups(
    groups: list[dict[str, Any]], prompt_vectors: list[list[float]]
) -> list[dict[str, Any]]:
    expected = sum(len(label["prompts"]) for group in groups for label in group["labels"])
    if len(prompt_vectors) != expected:
        raise ValueError(f"expected {expected} prompt vectors, received {len(prompt_vectors)}")
    prepared: list[dict[str, Any]] = []
    offset = 0
    for group in groups:
        prepared_labels = []
        for label in group["labels"]:
            count = len(label["prompts"])
            variants = prompt_vectors[offset : offset + count]
            offset += count
            prepared_labels.append(
                {
                    "id": label["id"],
                    "text": label["text"],
                    "variants": variants,
                }
            )
        prepared.append({"id": group["id"], "labels": prepared_labels})
    return prepared


def classify_group(
    frame_vector: list[float],
    group: dict[str, Any],
    *,
    logit_scale: float,
    logit_bias: float,
) -> dict[str, Any]:
    labels = group["labels"]
    prompt_count = len(labels[0]["variants"])
    prompt_choices: list[str | None] = []
    for prompt_index in range(prompt_count):
        chosen = max(
            labels,
            key=lambda label: (_dot(frame_vector, label["variants"][prompt_index]), label["id"]),
        )
        probability = _sigmoid(
            _dot(frame_vector, chosen["variants"][prompt_index]) * logit_scale
            + logit_bias
        )
        prompt_choices.append(chosen["id"] if probability >= 0.5 else None)
    stable = prompt_choices[0] is not None and len(set(prompt_choices)) == 1
    chosen = next(
        (label for label in labels if stable and label["id"] == prompt_choices[0]),
        None,
    )
    return {
        "group": group["id"],
        "available": stable,
        "label_id": chosen["id"] if chosen else None,
        "text": chosen["text"] if chosen else None,
        "stable_across_prompts": stable,
        "prompt_choices": prompt_choices,
    }


def classify_frames(
    frames: list[dict[str, Any]],
    groups: list[dict[str, Any]],
    *,
    logit_scale: float,
    logit_bias: float,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    results = []
    stable_counts = {group["id"]: 0 for group in groups}
    for frame in frames:
        identifications = [
            classify_group(
                frame["vector"],
                group,
                logit_scale=logit_scale,
                logit_bias=logit_bias,
            )
            for group in groups
        ]
        for identification in identifications:
            stable_counts[identification["group"]] += int(
                identification["stable_across_prompts"]
            )
        results.append(
            {
                "source_id": frame["source_id"],
                "time_ms": frame["time_ms"],
                "identifications": identifications,
            }
        )
    summary = {
        "frame_count": len(frames),
        "available_rate_by_group": {
            group_id: count / len(frames) for group_id, count in stable_counts.items()
        },
    }
    return results, summary


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--model", type=Path, required=True)
    result.add_argument("--scenes", type=Path, required=True)
    result.add_argument("--embeddings", type=Path, required=True)
    result.add_argument("--source-id", required=True)
    result.add_argument("--vocabulary", type=Path, required=True)
    result.add_argument("--output", type=Path, required=True)
    result.add_argument("--start-ms", type=int, default=0)
    result.add_argument("--end-ms", type=int, required=True)
    result.add_argument("--device", default=None)
    result.add_argument(
        "--batch-size",
        type=int,
        default=int(os.environ.get("EVIDENCE_SIGLIP_BATCH_SIZE", "16")),
    )
    return result


def run(arguments: argparse.Namespace) -> dict[str, Any]:
    import torch

    from embed_cli import SiglipEmbedder

    if arguments.start_ms < 0 or arguments.end_ms <= arguments.start_ms:
        raise ValueError("label window must be a non-empty [start_ms,end_ms) range")
    scenes = load_json(arguments.scenes)
    sources = {
        source.get("id"): source for source in scenes.get("sources", []) if isinstance(source, dict)
    }
    original = sources.get(arguments.source_id)
    if not isinstance(original, dict) or not isinstance(original.get("sha256"), str):
        raise ValueError(f"source {arguments.source_id} is missing from the scene document")
    manifest_stub = {
        "source": {"source_id": arguments.source_id, "sha256": original["sha256"]}
    }
    frames, identity = load_frames(scenes, load_json(arguments.embeddings), manifest_stub)
    frames = [
        frame
        for frame in frames
        if arguments.start_ms <= frame["time_ms"] < arguments.end_ms
    ]
    if not frames:
        raise ValueError("no sampled frames fall inside the requested window")
    vocabulary = load_json(arguments.vocabulary)
    groups = validate_vocabulary(vocabulary)
    prompts = [prompt for group in groups for label in group["labels"] for prompt in label["prompts"]]
    device = arguments.device or ("cuda" if torch.cuda.is_available() else "cpu")
    started = time.perf_counter()
    embedder = SiglipEmbedder(arguments.model, device, arguments.batch_size)
    prompt_vectors = embedder.texts(prompts)
    logit_scale, logit_bias = embedder.pair_calibration()
    text_embedding_seconds = time.perf_counter() - started
    prepared = prepare_groups(groups, prompt_vectors)
    results, summary = classify_frames(
        frames,
        prepared,
        logit_scale=logit_scale,
        logit_bias=logit_bias,
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "warning": (
            "Experimental abstaining labels for navigation only. Unavailable labels mean the "
            "model did not clear its native threshold consistently; available labels still "
            "require visual review."
        ),
        "source": manifest_stub["source"],
        "window": {"start_ms": arguments.start_ms, "end_ms": arguments.end_ms},
        "model": {
            **identity,
            "artifact_fingerprint": fingerprint_model(arguments.model),
            "device": device,
            "text_embedding_seconds": text_embedding_seconds,
        },
        "vocabulary": {"title": vocabulary.get("title"), "path": str(arguments.vocabulary)},
        "summary": summary,
        "frames": results,
    }


def main() -> int:
    try:
        arguments = parser().parse_args()
        report = run(arguments)
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(
            json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8"
        )
        return 0
    except Exception as error:  # noqa: BLE001 - CLI boundary reports one concise failure.
        print(f"label probe failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
