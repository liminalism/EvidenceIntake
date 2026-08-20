"""Pure contract tests for the interval benchmark; no model is loaded."""

from __future__ import annotations

import copy
import unittest

from benchmark_cli import evaluate, load_frames, validate_manifest


SOURCE_HASH = "a" * 64


def manifest() -> dict:
    return {
        "schema_version": 1,
        "source": {"source_id": "video", "sha256": SOURCE_HASH},
        "result_budgets": [2, 1, 2],
        "queries": [
            {
                "id": "visible",
                "text": "blue object",
                "category": "visible-state",
                "mode": "still",
                "relevant_intervals": [
                    {"start_ms": 4000, "end_ms": 6000},
                    {"start_ms": 2000, "end_ms": 3000},
                ],
            },
            {
                "id": "motion",
                "text": "object begins moving",
                "category": "temporal-action",
                "mode": "temporal",
                "relevant_intervals": [{"start_ms": 0, "end_ms": 1500}],
            },
            {
                "id": "absent",
                "text": "helicopter",
                "category": "negative",
                "mode": "negative",
                "relevant_intervals": [],
            },
        ],
    }


class BenchmarkContractTests(unittest.TestCase):
    def test_manifest_modes_and_budgets_are_validated(self) -> None:
        checked = validate_manifest(manifest())
        self.assertEqual(checked["result_budgets"], [1, 2])

        invalid = manifest()
        invalid["queries"][2]["relevant_intervals"] = [{"start_ms": 1, "end_ms": 2}]
        with self.assertRaisesRegex(ValueError, "negative query"):
            validate_manifest(invalid)

    def test_frames_keep_the_derived_stills_original_timeline_instants(self) -> None:
        scenes = {
            "case_id": "case",
            "sources": [
                {"id": "video", "sha256": SOURCE_HASH, "segments": []},
                {"id": "still-1", "segments": [{"start_ms": 5000}]},
            ],
            "edges": [
                {"from_id": "still-1", "to_id": "video", "relation": "derived_from"}
            ],
        }
        embeddings = {
            "case_id": "case",
            "embeddings": [
                {
                    "source_id": "still-1",
                    "model": "candidate",
                    "extractor": "keyframe_embed",
                    "version": "candidate-revision",
                    "vector": [0.0, 1.0],
                }
            ],
        }
        frames, identity = load_frames(scenes, embeddings, validate_manifest(manifest()))
        self.assertEqual(frames[0]["time_ms"], 5000)
        self.assertEqual(identity["label"], "candidate")

    def test_sampling_opportunity_is_separate_from_retrieval_recall(self) -> None:
        checked = validate_manifest(copy.deepcopy(manifest()))
        frames = [
            {"source_id": "still-1", "time_ms": 1000, "vector": [1.0, 0.0]},
            {"source_id": "still-2", "time_ms": 5000, "vector": [0.0, 1.0]},
            {"source_id": "still-3", "time_ms": 9000, "vector": [-1.0, 0.0]},
        ]
        report = evaluate(
            checked,
            frames,
            [
                [0.0, 1.0],
                [1.0, 0.0],
                [-1.0, 0.0],
            ],
        )
        self.assertEqual(report["labelled_interval_count"], 2)
        self.assertEqual(report["sampled_interval_count"], 1)
        self.assertEqual(report["sampling_opportunity_recall"], 0.5)
        self.assertEqual(report["retrieval_interval_recall_at"]["1"], 0.5)
        self.assertEqual(report["retrieval_recall_given_sampling_opportunity_at"]["1"], 1.0)
        self.assertTrue(report["queries"][0]["metric_inclusion"])
        self.assertFalse(report["queries"][1]["metric_inclusion"])
        self.assertFalse(report["queries"][2]["metric_inclusion"])
        self.assertNotIn("similarity", str(report))


if __name__ == "__main__":
    unittest.main()
