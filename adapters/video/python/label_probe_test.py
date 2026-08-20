"""Pure tests for calibrated prompt agreement and abstention; no model is loaded."""

from __future__ import annotations

import unittest

from label_probe import classify_group, classify_frames


class LabelProbeTests(unittest.TestCase):
    def test_agreeing_prompt_variants_produce_a_stable_label(self) -> None:
        group = {
            "id": "setting",
            "labels": [
                {
                    "id": "road",
                    "text": "road",
                    "variants": [[1.0, 0.0], [0.9, 0.1]],
                },
                {
                    "id": "room",
                    "text": "room",
                    "variants": [[0.0, 1.0], [0.1, 0.9]],
                },
            ],
        }
        result = classify_group(
            [1.0, 0.0], group, logit_scale=10.0, logit_bias=-5.0
        )
        self.assertEqual(result["text"], "road")
        self.assertTrue(result["stable_across_prompts"])
        self.assertNotIn("score", result)

    def test_disagreeing_prompt_variants_mark_the_label_unstable(self) -> None:
        group = {
            "id": "people",
            "labels": [
                {
                    "id": "one",
                    "text": "one person",
                    "variants": [[1.0, 0.0], [0.0, 1.0]],
                },
                {
                    "id": "none",
                    "text": "no person",
                    "variants": [[0.0, 1.0], [1.0, 0.0]],
                },
            ],
        }
        result = classify_group(
            [1.0, 0.0], group, logit_scale=10.0, logit_bias=-5.0
        )
        self.assertFalse(result["stable_across_prompts"])
        self.assertFalse(result["available"])
        self.assertIsNone(result["text"])

    def test_agreement_below_native_threshold_abstains(self) -> None:
        group = {
            "id": "people",
            "labels": [
                {"id": "one", "text": "one person", "variants": [[0.1, 0.0], [0.1, 0.0]]},
                {"id": "none", "text": "no person", "variants": [[0.0, 0.1], [0.0, 0.1]]},
            ],
        }
        result = classify_group(
            [1.0, 0.0], group, logit_scale=10.0, logit_bias=-5.0
        )
        self.assertFalse(result["available"])
        self.assertEqual(result["prompt_choices"], [None, None])

    def test_frame_summary_counts_stability_per_group(self) -> None:
        group = {
            "id": "quality",
            "labels": [
                {
                    "id": "clear",
                    "text": "clear",
                    "variants": [[1.0, 0.0], [1.0, 0.0]],
                },
                {
                    "id": "dark",
                    "text": "dark",
                    "variants": [[0.0, 1.0], [0.0, 1.0]],
                },
            ],
        }
        frames, summary = classify_frames(
            [{"source_id": "still", "time_ms": 10, "vector": [1.0, 0.0]}],
            [group],
            logit_scale=10.0,
            logit_bias=-5.0,
        )
        self.assertEqual(frames[0]["identifications"][0]["text"], "clear")
        self.assertEqual(summary["available_rate_by_group"]["quality"], 1.0)


if __name__ == "__main__":
    unittest.main()
