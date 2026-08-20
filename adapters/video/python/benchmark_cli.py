"""Evaluate text-to-frame retrieval against human-labelled timeline intervals.

The manifest is authored before looking at model rankings. This runner reports
whether the sampler had a frame inside each labelled interval separately from
whether the encoder returned such a frame within each reviewer result budget.
Similarity values remain internal and are not written to the report.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import statistics
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 1
MODES = {"still", "temporal", "negative"}
FINGERPRINT_FILES = (
    "config.json",
    "model.safetensors",
    "preprocessor_config.json",
    "tokenizer.model",
    "tokenizer_config.json",
)


def _object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be a JSON object")
    return value


def _array(value: Any, label: str) -> list[Any]:
    if not isinstance(value, list):
        raise ValueError(f"{label} must be a JSON array")
    return value


def _text(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{label} must be non-empty text")
    return value.strip()


def load_json(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        return _object(json.load(handle), str(path))


def validate_manifest(document: dict[str, Any]) -> dict[str, Any]:
    if document.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(f"manifest schema_version must be {SCHEMA_VERSION}")
    source = _object(document.get("source"), "manifest.source")
    _text(source.get("source_id"), "manifest.source.source_id")
    sha256 = _text(source.get("sha256"), "manifest.source.sha256")
    if len(sha256) != 64 or any(char not in "0123456789abcdef" for char in sha256.lower()):
        raise ValueError("manifest.source.sha256 must be a 64-digit hexadecimal SHA-256")
    source["sha256"] = sha256.lower()

    budgets = _array(document.get("result_budgets"), "manifest.result_budgets")
    if not budgets or any(not isinstance(value, int) or value < 1 for value in budgets):
        raise ValueError("manifest.result_budgets must contain positive integers")
    document["result_budgets"] = sorted(set(budgets))

    queries = _array(document.get("queries"), "manifest.queries")
    if not queries:
        raise ValueError("manifest.queries must not be empty")
    seen_ids: set[str] = set()
    for index, raw_query in enumerate(queries):
        query = _object(raw_query, f"manifest.queries[{index}]")
        query_id = _text(query.get("id"), f"manifest.queries[{index}].id")
        if query_id in seen_ids:
            raise ValueError(f"duplicate query id: {query_id}")
        seen_ids.add(query_id)
        _text(query.get("text"), f"query {query_id}.text")
        _text(query.get("category"), f"query {query_id}.category")
        mode = _text(query.get("mode"), f"query {query_id}.mode")
        if mode not in MODES:
            raise ValueError(f"query {query_id}.mode must be one of {sorted(MODES)}")
        intervals = _array(query.get("relevant_intervals"), f"query {query_id}.relevant_intervals")
        if mode == "negative" and intervals:
            raise ValueError(f"negative query {query_id} must not have relevant intervals")
        if mode != "negative" and not intervals:
            raise ValueError(f"query {query_id} needs at least one human-labelled interval")
        for interval_index, raw_interval in enumerate(intervals):
            interval = _object(raw_interval, f"query {query_id} interval {interval_index}")
            start_ms = interval.get("start_ms")
            end_ms = interval.get("end_ms")
            if (
                not isinstance(start_ms, int)
                or not isinstance(end_ms, int)
                or start_ms < 0
                or end_ms <= start_ms
            ):
                raise ValueError(
                    f"query {query_id} interval {interval_index} must be a non-empty "
                    "[start_ms,end_ms) range"
                )
    return document


def load_frames(
    scenes: dict[str, Any], embeddings: dict[str, Any], manifest: dict[str, Any]
) -> tuple[list[dict[str, Any]], dict[str, str]]:
    source_contract = _object(manifest["source"], "manifest.source")
    source_id = str(source_contract["source_id"])
    raw_sources = _array(scenes.get("sources"), "scenes.sources")
    sources = {
        _text(source.get("id"), "source.id"): source
        for source in (_object(item, "scenes.sources[]") for item in raw_sources)
    }
    original = _object(sources.get(source_id), f"source {source_id}")
    if original.get("sha256") != source_contract.get("sha256"):
        raise ValueError("manifest source SHA-256 does not match the sampled scene document")
    if embeddings.get("case_id") != scenes.get("case_id"):
        raise ValueError("scene and embedding documents belong to different cases")
    derived_from = {
        (edge.get("from_id"), edge.get("to_id"))
        for edge in (
            _object(item, "scenes.edges[]")
            for item in _array(scenes.get("edges"), "scenes.edges")
        )
        if edge.get("relation") == "derived_from"
    }

    frames: list[dict[str, Any]] = []
    identities: set[tuple[str, str, str]] = set()
    vector_size: int | None = None
    for raw_embedding in _array(embeddings.get("embeddings"), "embeddings.embeddings"):
        embedding = _object(raw_embedding, "embedding")
        still_id = _text(embedding.get("source_id"), "embedding.source_id")
        if (still_id, source_id) not in derived_from:
            raise ValueError(f"embedding source {still_id} is not derived from {source_id}")
        still = _object(sources.get(still_id), f"derived still {still_id}")
        segments = _array(still.get("segments"), f"derived still {still_id}.segments")
        if len(segments) != 1:
            raise ValueError(f"derived still {still_id} must have exactly one segment")
        segment = _object(segments[0], f"derived still {still_id}.segments[0]")
        time_ms = segment.get("start_ms")
        if not isinstance(time_ms, int) or time_ms < 0:
            raise ValueError(f"derived still {still_id} has no valid original-timeline start_ms")
        vector = _array(embedding.get("vector"), f"embedding {still_id}.vector")
        invalid_vector = any(
            not isinstance(value, (int, float)) or not math.isfinite(value) for value in vector
        )
        if not vector or invalid_vector:
            raise ValueError(f"embedding {still_id} must contain finite numbers")
        vector_size = vector_size or len(vector)
        if len(vector) != vector_size:
            raise ValueError("stored frame embeddings do not share one vector dimension")
        identities.add(
            (
                _text(embedding.get("model"), f"embedding {still_id}.model"),
                _text(embedding.get("extractor"), f"embedding {still_id}.extractor"),
                _text(embedding.get("version"), f"embedding {still_id}.version"),
            )
        )
        frames.append({"source_id": still_id, "time_ms": time_ms, "vector": vector})
    if not frames:
        raise ValueError("embedding document contains no frames")
    if len(identities) != 1:
        raise ValueError("stored frame embeddings do not share one model identity")
    frames.sort(key=lambda frame: (frame["time_ms"], frame["source_id"]))
    model, extractor, version = identities.pop()
    return frames, {"label": model, "extractor": extractor, "version": version}


def _inside(time_ms: int, interval: dict[str, Any]) -> bool:
    return int(interval["start_ms"]) <= time_ms < int(interval["end_ms"])


def _rank(query_vector: list[float], frames: list[dict[str, Any]]) -> list[dict[str, Any]]:
    if len(query_vector) != len(frames[0]["vector"]):
        raise ValueError("query and frame vector dimensions differ")
    scored = [
        (math.fsum(left * right for left, right in zip(query_vector, frame["vector"])), frame)
        for frame in frames
    ]
    scored.sort(key=lambda item: (-item[0], item[1]["source_id"]))
    return [frame for _, frame in scored]


def evaluate(
    manifest: dict[str, Any], frames: list[dict[str, Any]], query_vectors: list[list[float]]
) -> dict[str, Any]:
    queries = _array(manifest["queries"], "manifest.queries")
    if len(queries) != len(query_vectors):
        raise ValueError("one query vector is required for every manifest query")
    budgets = [int(value) for value in manifest["result_budgets"]]
    maximum_budget = max(budgets)
    aggregate_interval_count = 0
    aggregate_sampled_count = 0
    retrieved_counts = {budget: 0 for budget in budgets}
    reports: list[dict[str, Any]] = []

    for query, query_vector in zip(queries, query_vectors):
        intervals = _array(query["relevant_intervals"], f"query {query['id']}.relevant_intervals")
        opportunity = [
            any(_inside(frame["time_ms"], interval) for frame in frames)
            for interval in intervals
        ]
        ranked = _rank(query_vector, frames)
        retrieved_at: dict[str, list[bool]] = {}
        for budget in budgets:
            selected = ranked[:budget]
            retrieved_at[str(budget)] = [
                any(_inside(frame["time_ms"], interval) for frame in selected)
                for interval in intervals
            ]
        if query["mode"] == "still":
            aggregate_interval_count += len(intervals)
            aggregate_sampled_count += sum(opportunity)
            for budget in budgets:
                retrieved_counts[budget] += sum(retrieved_at[str(budget)])
        reports.append(
            {
                "id": query["id"],
                "text": query["text"],
                "category": query["category"],
                "mode": query["mode"],
                "metric_inclusion": query["mode"] == "still",
                "sampling_opportunity": opportunity,
                "retrieved_intervals_at": retrieved_at,
                "candidates_at_max_budget": [
                    {
                        "rank": rank,
                        "source_id": frame["source_id"],
                        "time_ms": frame["time_ms"],
                    }
                    for rank, frame in enumerate(ranked[:maximum_budget], start=1)
                ],
            }
        )

    def ratio(numerator: int, denominator: int) -> float | None:
        return numerator / denominator if denominator else None

    return {
        "positive_still_query_count": sum(query["mode"] == "still" for query in queries),
        "labelled_interval_count": aggregate_interval_count,
        "sampled_interval_count": aggregate_sampled_count,
        "sampling_opportunity_recall": ratio(aggregate_sampled_count, aggregate_interval_count),
        "retrieval_interval_recall_at": {
            str(budget): ratio(retrieved_counts[budget], aggregate_interval_count)
            for budget in budgets
        },
        "retrieval_recall_given_sampling_opportunity_at": {
            str(budget): ratio(retrieved_counts[budget], aggregate_sampled_count)
            for budget in budgets
        },
        "queries": reports,
    }


def fingerprint_model(model_dir: Path) -> dict[str, Any]:
    digest = hashlib.sha256()
    files: list[dict[str, Any]] = []
    for name in FINGERPRINT_FILES:
        path = model_dir / name
        if not path.is_file():
            continue
        file_digest = hashlib.sha256()
        digest.update(name.encode("utf-8") + b"\0")
        with path.open("rb") as handle:
            while chunk := handle.read(8 * 1024 * 1024):
                digest.update(chunk)
                file_digest.update(chunk)
        files.append(
            {"name": name, "bytes": path.stat().st_size, "sha256": file_digest.hexdigest()}
        )
    if "model.safetensors" not in {item["name"] for item in files}:
        raise ValueError(f"model.safetensors is missing from {model_dir}")
    return {"sha256": digest.hexdigest(), "files": files}


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        "--model", type=Path, required=True, help="local Hugging Face model directory"
    )
    result.add_argument("--scenes", type=Path, required=True, help="evidence-video scenes JSON")
    result.add_argument("--embeddings", type=Path, required=True, help="evidence-video embed JSON")
    result.add_argument(
        "--manifest", type=Path, required=True, help="human-authored interval labels"
    )
    result.add_argument("--output", type=Path, required=True, help="benchmark report JSON")
    result.add_argument(
        "--device", default=None, help="cuda or cpu; defaults to CUDA when available"
    )
    result.add_argument(
        "--batch-size",
        type=int,
        default=int(os.environ.get("EVIDENCE_SIGLIP_BATCH_SIZE", "16")),
    )
    return result


def run(arguments: argparse.Namespace) -> dict[str, Any]:
    import torch

    from embed_cli import SiglipEmbedder

    manifest = validate_manifest(load_json(arguments.manifest))
    frames, identity = load_frames(
        load_json(arguments.scenes), load_json(arguments.embeddings), manifest
    )
    device = arguments.device or ("cuda" if torch.cuda.is_available() else "cpu")
    if device == "cuda":
        torch.cuda.empty_cache()
        torch.cuda.reset_peak_memory_stats()
    load_started = time.perf_counter()
    embedder = SiglipEmbedder(arguments.model, device, arguments.batch_size)
    load_seconds = time.perf_counter() - load_started
    query_texts = [str(query["text"]) for query in manifest["queries"]]
    query_started = time.perf_counter()
    query_vectors = embedder.texts(query_texts)
    query_seconds = time.perf_counter() - query_started
    interactive_query_ms: list[float] = []
    for query_text in query_texts:
        started = time.perf_counter()
        embedder.text(query_text)
        interactive_query_ms.append((time.perf_counter() - started) * 1000.0)
    evaluation = evaluate(manifest, frames, query_vectors)
    fingerprint = fingerprint_model(arguments.model)
    model_report: dict[str, Any] = {
        **identity,
        "artifact_fingerprint": fingerprint,
        "vector_dimension": len(frames[0]["vector"]),
        "device": device,
        "load_seconds": load_seconds,
        "query_batch_seconds": query_seconds,
        "query_batch_throughput_per_second": len(query_texts) / query_seconds,
        "interactive_query_median_ms": statistics.median(interactive_query_ms),
        "interactive_query_max_ms": max(interactive_query_ms),
    }
    if device == "cuda":
        model_report["peak_cuda_allocated_bytes"] = torch.cuda.max_memory_allocated()
        model_report["peak_cuda_reserved_bytes"] = torch.cuda.max_memory_reserved()
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source": manifest["source"],
        "result_budgets": manifest["result_budgets"],
        "model": model_report,
        "evaluation": evaluation,
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
        print(f"benchmark failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
