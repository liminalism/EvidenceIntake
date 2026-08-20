"""Pure caption-output safety tests; no model is loaded."""

from __future__ import annotations

import unittest

from caption_cli import normalize_caption


class CaptionCliTests(unittest.TestCase):
    def test_free_text_is_reduced_to_deterministic_visible_terms(self) -> None:
        self.assertEqual(
            normalize_caption("  Two military personnel in camouflage stand near a vehicle.\n"),
            "Visible navigation terms: multiple people, camouflage clothing, vehicle.",
        )

    def test_explicit_abstention_becomes_null(self) -> None:
        self.assertIsNone(normalize_caption("ABSTAIN"))

    def test_unsupported_event_words_do_not_enter_rendered_text(self) -> None:
        self.assertEqual(
            normalize_caption("A police officer watches a possible riot near smoke."),
            "Visible navigation terms: person, smoke.",
        )

    def test_response_with_no_allowed_visible_term_abstains(self) -> None:
        self.assertIsNone(normalize_caption("A possible protest is happening."))

    def test_more_specific_vehicle_term_suppresses_generic_vehicle(self) -> None:
        self.assertEqual(
            normalize_caption("A fire truck is parked on a street."),
            "Visible navigation terms: fire truck, road.",
        )


if __name__ == "__main__":
    unittest.main()
