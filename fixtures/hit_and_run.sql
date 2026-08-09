INSERT INTO cases (id, name, reference, jurisdiction)
VALUES ('case-hit-run-001', 'State v. Morgan — hit-and-run fixture', 'PD-2026-0118', 'Example');

INSERT INTO productions (id, case_id, label, received_at, producing_party, notes) VALUES
  ('hr-prod-initial', 'case-hit-run-001', 'Initial crash discovery', '2026-03-05T16:00:00Z', 'Prosecution', 'Reports, witness material, and a transcoded camera clip'),
  ('hr-prod-supp', 'case-hit-run-001', 'Supplemental DUI discovery', '2026-03-14T10:30:00Z', 'Prosecution', 'Interview transcript, chemical-test report, and vehicle photographs');

INSERT INTO sources
  (id, case_id, production_id, logical_name, media_type, sha256, byte_length, review_state, integrity_status)
VALUES
  ('hr-src-crash-report', 'case-hit-run-001', 'hr-prod-initial', 'Crash report 26-0317 OCR.pdf', 'application/pdf',
   '1010101010101010101010101010101010101010101010101010101010101010', 194220, 'reviewed', 'available'),
  ('hr-src-victim', 'case-hit-run-001', 'hr-prod-initial', 'Lee witness statement OCR.pdf', 'application/pdf',
   '2020202020202020202020202020202020202020202020202020202020202020', 88312, 'verified', 'available'),
  ('hr-src-damage', 'case-hit-run-001', 'hr-prod-initial', 'Damage assessment OCR.pdf', 'application/pdf',
   '3030303030303030303030303030303030303030303030303030303030303030', 144020, 'reviewed', 'available'),
  ('hr-src-911', 'case-hit-run-001', 'hr-prod-initial', '911 call.wav', 'audio/wav',
   '4040404040404040404040404040404040404040404040404040404040404040', 3280401, 'reviewed', 'available'),
  ('hr-src-camera', 'case-hit-run-001', 'hr-prod-initial', 'Oak and Third camera prosecution clip.mp4', 'video/mp4',
   '5050505050505050505050505050505050505050505050505050505050505050', 12280334, 'unreviewed', 'truncated'),
  ('hr-src-interview', 'case-hit-run-001', 'hr-prod-supp', 'Morgan roadside interview.wav', 'audio/wav',
   '6060606060606060606060606060606060606060606060606060606060606060', 9480030, 'reviewed', 'available'),
  ('hr-src-test', 'case-hit-run-001', 'hr-prod-supp', 'Chemical test report OCR.pdf', 'application/pdf',
   '7070707070707070707070707070707070707070707070707070707070707070', 77411, 'verified', 'available'),
  ('hr-src-photos', 'case-hit-run-001', 'hr-prod-supp', 'Morgan vehicle photographs.zip', 'application/zip',
   '8080808080808080808080808080808080808080808080808080808080808080', 6228404, 'reviewed', 'available'),
  ('hr-src-full-video', 'case-hit-run-001', 'hr-prod-initial', 'Oak and Third full native camera export', 'video/unknown',
   '9090909090909090909090909090909090909090909090909090909090909090', 0, 'unreviewed', 'missing');

INSERT INTO source_segments (id, source_id, locator, page, start_ms, end_ms, bbox_json) VALUES
  ('hr-seg-report-narrative', 'hr-src-crash-report', 'page 2, narrative paragraphs 1–4', 2, NULL, NULL, '[72,110,460,330]'),
  ('hr-seg-report-dui', 'hr-src-crash-report', 'page 3, supplemental suspicion paragraph', 3, NULL, NULL, '[70,240,465,120]'),
  ('hr-seg-victim-crash', 'hr-src-victim', 'page 1, lines 3–12', 1, NULL, NULL, '[55,105,500,240]'),
  ('hr-seg-victim-driver', 'hr-src-victim', 'page 1, lines 13–18', 1, NULL, NULL, '[55,350,500,130]'),
  ('hr-seg-damage', 'hr-src-damage', 'page 2, findings 1–5', 2, NULL, NULL, '[60,95,490,300]'),
  ('hr-seg-911-impact', 'hr-src-911', '00:00:08.200–00:00:31.600', NULL, 8200, 31600, NULL),
  ('hr-seg-camera-approach', 'hr-src-camera', 'scene 1, 00:00:00.000–00:00:09.800', NULL, 0, 9800, NULL),
  ('hr-seg-camera-impact', 'hr-src-camera', 'scene 2, 00:00:09.800–00:00:15.400', NULL, 9800, 15400, NULL),
  ('hr-seg-camera-depart', 'hr-src-camera', 'scene 3, 00:00:15.400–00:00:28.000', NULL, 15400, 28000, NULL),
  ('hr-seg-interview-driving', 'hr-src-interview', '00:03:12.100–00:03:38.900', NULL, 192100, 218900, NULL),
  ('hr-seg-interview-drinking', 'hr-src-interview', '00:04:02.400–00:04:44.200', NULL, 242400, 284200, NULL),
  ('hr-seg-interview-observation', 'hr-src-interview', '00:00:20.000–00:01:10.000', NULL, 20000, 70000, NULL),
  ('hr-seg-test', 'hr-src-test', 'page 1, result and collection time', 1, NULL, NULL, '[80,180,440,170]'),
  ('hr-seg-photos-front', 'hr-src-photos', 'IMG_0041–IMG_0044, front-right quarter', NULL, NULL, NULL, NULL),
  ('hr-seg-missing-video', 'hr-src-full-video', 'expected full native export and metadata', NULL, NULL, NULL, NULL);

INSERT INTO entities (id, case_id, kind, display_name, is_client, notes) VALUES
  ('hr-person-client', 'case-hit-run-001', 'person', 'Casey Morgan', 1, 'Client and registered owner of compact sedan'),
  ('hr-person-victim', 'case-hit-run-001', 'person', 'Riley Lee', 0, 'Driver of idling vehicle'),
  ('hr-person-officer', 'case-hit-run-001', 'person', 'Officer Dana Ruiz', 0, 'Investigating officer'),
  ('hr-vehicle-victim', 'case-hit-run-001', 'object', 'Lee blue hatchback', 0, NULL),
  ('hr-vehicle-client', 'case-hit-run-001', 'object', 'Morgan silver compact sedan', 0, NULL),
  ('hr-vehicle-video', 'case-hit-run-001', 'object', 'Unresolved light-colored sedan in camera clip', 0, 'Not merged with Morgan vehicle'),
  ('hr-location-crash', 'case-hit-run-001', 'location', 'Oak Street at Third Avenue', 0, 'Signalized intersection');

INSERT INTO content
  (id, case_id, segment_id, kind, text, speaker_entity_id, attributed_to_entity_id,
   raw_time, asserted_time, normalized_start, normalized_end, time_basis, location_text,
   extractor, extractor_version, machine_generated, extractor_confidence, review_state)
VALUES
  ('hr-content-report-collision', 'case-hit-run-001', 'hr-seg-report-narrative', 'document_assertion',
   'Lee''s stopped vehicle was struck from behind by a light-colored compact sedan that left eastbound.',
   'hr-person-officer', 'hr-person-victim', NULL, '2026-02-27T21:07:00', NULL, NULL,
   'Officer narrative summarizing later interviews', 'Oak Street at Third Avenue',
   'document_ocr', 'ocr-4.2.1', 1, 0.97, 'suggested'),
  ('hr-content-report-owner', 'case-hit-run-001', 'hr-seg-report-narrative', 'document_assertion',
   'Registration records identify Morgan as owner of a silver compact sedan.',
   'hr-person-officer', NULL, NULL, NULL, NULL, NULL, 'registration lookup described in report', NULL,
   'document_ocr', 'ocr-4.2.1', 1, 0.99, 'suggested'),
  ('hr-content-report-dui', 'case-hit-run-001', 'hr-seg-report-dui', 'document_assertion',
   'Ruiz suspected Morgan had operated while impaired based on observations made during contact at 22:04.',
   'hr-person-officer', NULL, NULL, '2026-02-27T21:07:00', NULL, NULL,
   'Suspicion documented after encounter; no measurement at collision time', NULL,
   'document_ocr', 'ocr-4.2.1', 1, 0.95, 'suggested'),
  ('hr-content-victim-impact', 'case-hit-run-001', 'hr-seg-victim-crash', 'statement',
   'I was stopped at the red light when a car hit me hard from behind and then drove around me.',
   'hr-person-victim', NULL, NULL, 'about 9:07 p.m.', NULL, NULL, 'witness estimate', 'Oak and Third',
   'document_ocr', 'ocr-4.2.1', 1, 0.96, 'suggested'),
  ('hr-content-victim-driver', 'case-hit-run-001', 'hr-seg-victim-driver', 'statement',
   'I could not see the driver clearly and cannot say who was driving.',
   'hr-person-victim', NULL, NULL, 'at collision', NULL, NULL, 'witness statement', 'Oak and Third',
   'document_ocr', 'ocr-4.2.1', 1, 0.98, 'suggested'),
  ('hr-content-victim-injury', 'case-hit-run-001', 'hr-seg-victim-crash', 'statement',
   'My neck felt sore the following morning; at the scene I told the dispatcher I did not need an ambulance.',
   'hr-person-victim', NULL, NULL, 'morning after collision', NULL, NULL, 'witness statement', NULL,
   'document_ocr', 'ocr-4.2.1', 1, 0.94, 'suggested'),
  ('hr-content-damage-match', 'case-hit-run-001', 'hr-seg-damage', 'document_assertion',
   'Damage heights and paint transfer on the two vehicles are mutually consistent with rear-to-front contact.',
   NULL, NULL, NULL, NULL, NULL, NULL, 'repair estimator comparison', NULL,
   'document_ocr', 'ocr-4.2.1', 1, 0.97, 'suggested'),
  ('hr-content-damage-limit', 'case-hit-run-001', 'hr-seg-damage', 'document_assertion',
   'The observed pattern cannot uniquely identify the striking vehicle without material comparison.',
   NULL, NULL, NULL, NULL, NULL, NULL, 'express limitation in report', NULL,
   'document_ocr', 'ocr-4.2.1', 1, 0.98, 'suggested'),
  ('hr-content-911-time', 'case-hit-run-001', 'hr-seg-911-impact', 'statement',
   'It just happened. I was sitting at the light. The other car is leaving toward Pine.',
   'hr-person-victim', NULL, 'call 21:08:11.200', 'seconds before call', '2026-02-27T21:07:35Z', '2026-02-27T21:08:11Z',
   'bounded by call connection and phrase “just happened”', 'Oak and Third',
   'local_asr', 'asr-2.8.0', 1, 0.91, 'suggested'),
  ('hr-content-911-injury', 'case-hit-run-001', 'hr-seg-911-impact', 'statement',
   'No, I do not think I am hurt. I do not need an ambulance.',
   'hr-person-victim', NULL, 'call 21:08:26.500', 'at scene', '2026-02-27T21:08:26Z', NULL,
   'dispatch server timestamp', 'Oak and Third',
   'local_asr', 'asr-2.8.0', 1, 0.94, 'suggested'),
  ('hr-content-video-vehicle', 'case-hit-run-001', 'hr-seg-camera-approach', 'observation',
   'A light-colored compact sedan approaches a stationary blue hatchback from the rear.',
   NULL, NULL, 'camera 21:06:51.000–21:07:00.800', NULL, '2026-02-27T21:06:51Z', '2026-02-27T21:07:00Z',
   'camera overlay provisionally aligned to dispatch time', 'Oak and Third',
   'local_video_scene_model', 'vision-3.1.0', 1, 0.93, 'suggested'),
  ('hr-content-video-impact', 'case-hit-run-001', 'hr-seg-camera-impact', 'observation',
   'The light-colored sedan contacts the rear of the stationary hatchback; both vehicles move visibly.',
   NULL, NULL, 'camera 21:07:00.800–21:07:06.400', NULL, '2026-02-27T21:07:00Z', '2026-02-27T21:07:06Z',
   'camera overlay provisionally aligned to dispatch time', 'Oak and Third',
   'local_video_scene_model', 'vision-3.1.0', 1, 0.96, 'suggested'),
  ('hr-content-video-depart', 'case-hit-run-001', 'hr-seg-camera-depart', 'observation',
   'The striking sedan pauses for approximately 1.8 seconds, moves around the hatchback, and exits eastbound.',
   NULL, NULL, 'camera 21:07:06.400–21:07:19.000', NULL, '2026-02-27T21:07:06Z', '2026-02-27T21:07:19Z',
   'camera overlay provisionally aligned to dispatch time', 'Oak and Third',
   'local_video_scene_model', 'vision-3.1.0', 1, 0.94, 'suggested'),
  ('hr-content-video-plate', 'case-hit-run-001', 'hr-seg-camera-depart', 'observation',
   'Possible plate characters “7K?2”; character sequence is unstable across frames.',
   NULL, NULL, 'camera 21:07:12.000–21:07:15.000', NULL, '2026-02-27T21:07:12Z', '2026-02-27T21:07:15Z',
   'camera overlay provisionally aligned to dispatch time', 'Oak and Third',
   'local_video_scene_model', 'vision-3.1.0', 1, 0.42, 'suggested'),
  ('hr-content-client-driving', 'case-hit-run-001', 'hr-seg-interview-driving', 'statement',
   'I drove my car home from work, but I did not know I hit anybody.',
   'hr-person-client', NULL, 'recorder 00:03:18.200', 'earlier that evening', '2026-02-27T20:50:00Z', '2026-02-27T21:25:00Z',
   'broad interval from work exit and home contact; exact route unresolved', NULL,
   'local_asr_diarized', 'asr-2.8.0', 1, 0.89, 'suggested'),
  ('hr-content-client-after-drink', 'case-hit-run-001', 'hr-seg-interview-drinking', 'statement',
   'I had two drinks after I got home because I was shaken up.',
   'hr-person-client', NULL, 'recorder 00:04:09.000', 'after arriving home', '2026-02-27T21:25:00Z', '2026-02-27T22:04:00Z',
   'client account; arrival time requires corroboration', 'Morgan residence',
   'local_asr_diarized', 'asr-2.8.0', 1, 0.92, 'suggested'),
  ('hr-content-officer-observation', 'case-hit-run-001', 'hr-seg-interview-observation', 'observation',
   'At 22:04 Ruiz described odor of alcohol, watery eyes, and uncertain balance.',
   'hr-person-officer', NULL, 'recorder 00:00:25.000–00:01:05.000', '2026-02-27T22:04:00', '2026-02-27T22:04:00Z', '2026-02-27T22:05:00Z',
   'body recorder synchronized to dispatch server', 'Morgan residence',
   'human_transcript_review', 'review-1', 0, 1.0, 'verified'),
  ('hr-content-test-result', 'case-hit-run-001', 'hr-seg-test', 'document_assertion',
   'Breath result recorded as 0.060 at 22:42; the report contains no retrograde estimate for 21:07.',
   'hr-person-officer', NULL, NULL, '2026-02-27T22:42:00', '2026-02-27T22:42:00Z', NULL,
   'instrument timestamp; result is not silently moved to collision time', NULL,
   'document_ocr_reviewed', 'ocr-4.2.1', 0, 1.0, 'verified'),
  ('hr-content-photo-damage', 'case-hit-run-001', 'hr-seg-photos-front', 'observation',
   'Fresh-appearing deformation and blue paint-like transfer are visible on the sedan front-right area.',
   NULL, NULL, NULL, 'photographed at 22:21', '2026-02-27T22:21:00Z', NULL,
   'photograph metadata', 'Morgan residence',
   'local_image_model', 'vision-3.1.0', 1, 0.88, 'suggested'),
  ('hr-content-full-video-ref', 'case-hit-run-001', 'hr-seg-report-narrative', 'evidence_reference',
   'Report states the traffic unit retained the full native Oak and Third recording and signal phase log.',
   'hr-person-officer', NULL, NULL, NULL, NULL, NULL, NULL, 'Oak and Third',
   'document_ocr', 'ocr-4.2.1', 1, 0.96, 'suggested');

INSERT INTO propositions (id, case_id, text, status, review_state, created_by) VALUES
  ('hr-prop-collision', 'case-hit-run-001', 'A light-colored sedan struck Lee''s stopped hatchback at approximately 21:07.', 'contested', 'reviewed', 'fixture-attorney'),
  ('hr-prop-client-car', 'case-hit-run-001', 'Morgan''s sedan was the striking vehicle.', 'contested', 'reviewed', 'fixture-attorney'),
  ('hr-prop-client-driver', 'case-hit-run-001', 'Morgan was driving the striking vehicle at the collision time.', 'contested', 'reviewed', 'fixture-attorney'),
  ('hr-prop-knowledge', 'case-hit-run-001', 'The driver knew or reasonably should have known a collision occurred.', 'contested', 'reviewed', 'fixture-attorney'),
  ('hr-prop-left', 'case-hit-run-001', 'The striking driver left without stopping to provide identifying information.', 'contested', 'reviewed', 'fixture-attorney'),
  ('hr-prop-injury', 'case-hit-run-001', 'The collision caused bodily injury to Lee.', 'contested', 'reviewed', 'fixture-attorney'),
  ('hr-prop-impaired-driving', 'case-hit-run-001', 'Morgan was impaired while driving at approximately 21:07.', 'contested', 'reviewed', 'fixture-attorney'),
  ('hr-prop-post-driving-alcohol', 'case-hit-run-001', 'Morgan consumed alcohol only after arriving home.', 'contested', 'reviewed', 'fixture-attorney'),
  ('hr-prop-property-damage', 'case-hit-run-001', 'The collision caused property damage to Lee''s hatchback.', 'contested', 'reviewed', 'fixture-attorney');

INSERT INTO edges
  (id, case_id, source_kind, source_id, relation, target_kind, target_id, rationale, review_state, created_by)
VALUES
  ('hr-edge-report-collision', 'case-hit-run-001', 'content', 'hr-content-report-collision', 'supports', 'proposition', 'hr-prop-collision', 'After-the-fact police summary; verify reporting chain.', 'suggested', 'fixture-attorney'),
  ('hr-edge-victim-collision', 'case-hit-run-001', 'content', 'hr-content-victim-impact', 'supports', 'proposition', 'hr-prop-collision', 'First-person account.', 'suggested', 'fixture-attorney'),
  ('hr-edge-video-collision', 'case-hit-run-001', 'content', 'hr-content-video-impact', 'corroborates', 'proposition', 'hr-prop-collision', 'Scene-model observation requires frame verification.', 'suggested', 'fixture-attorney'),
  ('hr-edge-damage-collision', 'case-hit-run-001', 'content', 'hr-content-damage-match', 'corroborates', 'proposition', 'hr-prop-collision', 'Damage geometry is consistent but not unique.', 'suggested', 'fixture-attorney'),
  ('hr-edge-photo-car', 'case-hit-run-001', 'content', 'hr-content-photo-damage', 'supports', 'proposition', 'hr-prop-client-car', 'Damage and transfer warrant comparison.', 'suggested', 'fixture-attorney'),
  ('hr-edge-damage-car', 'case-hit-run-001', 'content', 'hr-content-damage-match', 'supports', 'proposition', 'hr-prop-client-car', 'Mutually consistent damage.', 'suggested', 'fixture-attorney'),
  ('hr-edge-damage-limit', 'case-hit-run-001', 'content', 'hr-content-damage-limit', 'qualifies', 'proposition', 'hr-prop-client-car', 'Assessment expressly cannot uniquely identify the vehicle.', 'suggested', 'fixture-attorney'),
  ('hr-edge-plate-car', 'case-hit-run-001', 'content', 'hr-content-video-plate', 'qualifies', 'proposition', 'hr-prop-client-car', 'Low-confidence partial plate must not be treated as identification.', 'suggested', 'fixture-attorney'),
  ('hr-edge-owner-car', 'case-hit-run-001', 'content', 'hr-content-report-owner', 'qualifies', 'proposition', 'hr-prop-client-driver', 'Ownership does not establish who drove at the relevant time.', 'suggested', 'fixture-attorney'),
  ('hr-edge-client-driver', 'case-hit-run-001', 'content', 'hr-content-client-driving', 'supports', 'proposition', 'hr-prop-client-driver', 'Admits driving that evening but exact route and collision time remain unresolved.', 'suggested', 'fixture-attorney'),
  ('hr-edge-victim-noid', 'case-hit-run-001', 'content', 'hr-content-victim-driver', 'qualifies', 'proposition', 'hr-prop-client-driver', 'Victim cannot identify driver.', 'suggested', 'fixture-attorney'),
  ('hr-edge-video-knowledge', 'case-hit-run-001', 'content', 'hr-content-video-impact', 'supports', 'proposition', 'hr-prop-knowledge', 'Visible vehicle movement may bear on notice, subject to human frame review.', 'suggested', 'fixture-attorney'),
  ('hr-edge-depart-knowledge', 'case-hit-run-001', 'content', 'hr-content-video-depart', 'supports', 'proposition', 'hr-prop-knowledge', 'Brief pause and maneuver may support an inference but do not directly show mental state.', 'suggested', 'fixture-attorney'),
  ('hr-edge-client-knowledge', 'case-hit-run-001', 'content', 'hr-content-client-driving', 'contradicts', 'proposition', 'hr-prop-knowledge', 'Client expressly disputes awareness.', 'suggested', 'fixture-attorney'),
  ('hr-edge-video-left', 'case-hit-run-001', 'content', 'hr-content-video-depart', 'supports', 'proposition', 'hr-prop-left', 'Clip shows departure without a visible exchange.', 'suggested', 'fixture-attorney'),
  ('hr-edge-victim-left', 'case-hit-run-001', 'content', 'hr-content-victim-impact', 'supports', 'proposition', 'hr-prop-left', 'Victim describes immediate departure.', 'suggested', 'fixture-attorney'),
  ('hr-edge-soreness-injury', 'case-hit-run-001', 'content', 'hr-content-victim-injury', 'supports', 'proposition', 'hr-prop-injury', 'Later soreness supports but does not resolve causation or statutory degree.', 'suggested', 'fixture-attorney'),
  ('hr-edge-911-injury', 'case-hit-run-001', 'content', 'hr-content-911-injury', 'qualifies', 'proposition', 'hr-prop-injury', 'Contemporaneous statement reports no perceived injury or ambulance need.', 'suggested', 'fixture-attorney'),
  ('hr-edge-damage-property', 'case-hit-run-001', 'content', 'hr-content-damage-match', 'supports', 'proposition', 'hr-prop-property-damage', 'Documents mutually consistent collision damage.', 'suggested', 'fixture-attorney'),
  ('hr-edge-later-observation-dui', 'case-hit-run-001', 'content', 'hr-content-officer-observation', 'supports', 'proposition', 'hr-prop-impaired-driving', 'Indicators observed about 57 minutes after collision.', 'verified', 'fixture-attorney'),
  ('hr-edge-test-dui', 'case-hit-run-001', 'content', 'hr-content-test-result', 'qualifies', 'proposition', 'hr-prop-impaired-driving', 'Later result has no admitted extrapolation to driving time.', 'verified', 'fixture-attorney'),
  ('hr-edge-report-dui', 'case-hit-run-001', 'content', 'hr-content-report-dui', 'supports', 'proposition', 'hr-prop-impaired-driving', 'Records suspicion, not a contemporaneous observation at 21:07.', 'suggested', 'fixture-attorney'),
  ('hr-edge-after-drink-dui', 'case-hit-run-001', 'content', 'hr-content-client-after-drink', 'explains', 'proposition', 'hr-prop-impaired-driving', 'If corroborated, post-driving consumption could explain later indicators.', 'suggested', 'fixture-attorney'),
  ('hr-edge-after-drink-prop', 'case-hit-run-001', 'content', 'hr-content-client-after-drink', 'supports', 'proposition', 'hr-prop-post-driving-alcohol', 'Client account; requires independent corroboration.', 'suggested', 'fixture-attorney'),
  ('hr-edge-missing-video', 'case-hit-run-001', 'content', 'hr-content-full-video-ref', 'expected_but_missing', 'source', 'hr-src-full-video', 'Only a prosecution-selected transcoded clip was produced.', 'suggested', 'fixture-attorney');

INSERT INTO charges (id, case_id, label, citation, posture, grade) VALUES
  ('hr-charge-injury-flight', 'case-hit-run-001', 'Leaving the scene of an injury collision', 'Example Code § 20-101', 'charged', 'felony'),
  ('hr-charge-dui', 'case-hit-run-001', 'Operating while impaired', 'Example Code § 30-202', 'charged', 'misdemeanor'),
  ('hr-charge-property-flight', 'case-hit-run-001', 'Failure to stop and provide information after property-damage collision', 'Example Code § 20-102', 'lesser_candidate', 'misdemeanor');

INSERT INTO elements (id, charge_id, ordinal, text) VALUES
  ('hr-el-fi-drive', 'hr-charge-injury-flight', 1, 'Morgan drove a vehicle involved in the collision.'),
  ('hr-el-fi-knowledge', 'hr-charge-injury-flight', 2, 'Morgan knew or reasonably should have known the collision occurred.'),
  ('hr-el-fi-injury', 'hr-charge-injury-flight', 3, 'The collision caused bodily injury.'),
  ('hr-el-fi-leave', 'hr-charge-injury-flight', 4, 'Morgan failed to stop and perform the required duties.'),
  ('hr-el-dui-drive', 'hr-charge-dui', 1, 'Morgan operated a motor vehicle.'),
  ('hr-el-dui-impaired', 'hr-charge-dui', 2, 'Morgan was impaired at the time of operation.'),
  ('hr-el-pd-drive', 'hr-charge-property-flight', 1, 'Morgan drove a vehicle involved in a property-damage collision.'),
  ('hr-el-pd-knowledge', 'hr-charge-property-flight', 2, 'Morgan knew or reasonably should have known the collision occurred.'),
  ('hr-el-pd-damage', 'hr-charge-property-flight', 3, 'The collision caused property damage.'),
  ('hr-el-pd-leave', 'hr-charge-property-flight', 4, 'Morgan failed to stop and provide required information.');

INSERT INTO element_links (id, element_id, proposition_id, assessment, notes, created_by) VALUES
  ('hr-link-fi-drive', 'hr-el-fi-drive', 'hr-prop-client-driver', 'uncertain', 'Ownership and broad admission do not yet fix route and time.', 'fixture-attorney'),
  ('hr-link-fi-knowledge', 'hr-el-fi-knowledge', 'hr-prop-knowledge', 'uncertain', 'Impact and maneuver support inference; client disputes awareness.', 'fixture-attorney'),
  ('hr-link-fi-injury', 'hr-el-fi-injury', 'hr-prop-injury', 'uncertain', 'Later soreness conflicts with contemporaneous no-injury report.', 'fixture-attorney'),
  ('hr-link-fi-leave', 'hr-el-fi-leave', 'hr-prop-left', 'supports', 'Video and first account show departure.', 'fixture-attorney'),
  ('hr-link-dui-drive', 'hr-el-dui-drive', 'hr-prop-client-driver', 'uncertain', 'Driving admitted broadly, exact collision-time identity unresolved.', 'fixture-attorney'),
  ('hr-link-dui-impaired', 'hr-el-dui-impaired', 'hr-prop-impaired-driving', 'uncertain', 'No contemporaneous impairment evidence or retrograde estimate.', 'fixture-attorney'),
  ('hr-link-dui-after', 'hr-el-dui-impaired', 'hr-prop-post-driving-alcohol', 'opposes', 'If corroborated, later drinking explains later observations.', 'fixture-attorney'),
  ('hr-link-pd-drive', 'hr-el-pd-drive', 'hr-prop-client-driver', 'uncertain', 'Same driver-identity issue remains.', 'fixture-attorney'),
  ('hr-link-pd-knowledge', 'hr-el-pd-knowledge', 'hr-prop-knowledge', 'uncertain', 'Brief pause and maneuver may support constructive knowledge.', 'fixture-attorney'),
  ('hr-link-pd-damage', 'hr-el-pd-damage', 'hr-prop-property-damage', 'supports', 'Damage evidence is substantially stronger than injury evidence.', 'fixture-attorney'),
  ('hr-link-pd-leave', 'hr-el-pd-leave', 'hr-prop-left', 'supports', 'Departure is recorded and contemporaneously reported.', 'fixture-attorney');

INSERT INTO events
  (id, case_id, label, lane, raw_time, normalized_start, normalized_end, time_basis, location_text, proposition_id, review_state)
VALUES
  ('hr-event-video-impact', 'case-hit-run-001', 'Camera clip: rear impact', 'recorded', 'camera 21:07:00.800', '2026-02-27T21:07:00Z', '2026-02-27T21:07:06Z', 'provisional camera/dispatch alignment', 'Oak and Third', 'hr-prop-collision', 'suggested'),
  ('hr-event-video-leave', 'case-hit-run-001', 'Camera clip: striking sedan departs', 'recorded', 'camera 21:07:06.400–21:07:19.000', '2026-02-27T21:07:06Z', '2026-02-27T21:07:19Z', 'provisional camera/dispatch alignment', 'Oak and Third', 'hr-prop-left', 'suggested'),
  ('hr-event-victim-account', 'case-hit-run-001', 'Lee account: stopped, struck, other vehicle left', 'witness_account', 'about 9:07 p.m.', '2026-02-27T21:07:00Z', '2026-02-27T21:08:11Z', 'bounded by 911 connection', 'Oak and Third', 'hr-prop-collision', 'suggested'),
  ('hr-event-client-driving', 'case-hit-run-001', 'Morgan account: drove home, unaware of impact', 'client_account', 'earlier that evening', '2026-02-27T20:50:00Z', '2026-02-27T21:25:00Z', 'broad unresolved interval', NULL, 'hr-prop-client-driver', 'suggested'),
  ('hr-event-client-drink', 'case-hit-run-001', 'Morgan account: drank after arriving home', 'client_account', 'after I got home', '2026-02-27T21:25:00Z', '2026-02-27T22:04:00Z', 'uncorroborated client account', 'Morgan residence', 'hr-prop-post-driving-alcohol', 'suggested'),
  ('hr-event-officer-contact', 'case-hit-run-001', 'Officer observes possible impairment indicators', 'police_narrative', '22:04', '2026-02-27T22:04:00Z', '2026-02-27T22:05:00Z', 'synchronized recorder', 'Morgan residence', 'hr-prop-impaired-driving', 'verified'),
  ('hr-event-test', 'case-hit-run-001', 'Breath result recorded as 0.060', 'recorded', 'instrument 22:42', '2026-02-27T22:42:00Z', NULL, 'instrument time', NULL, 'hr-prop-impaired-driving', 'verified'),
  ('hr-event-defense-gap', 'case-hit-run-001', 'Unresolved interval between collision and impairment observations', 'attorney_hypothesis', '21:07–22:04', '2026-02-27T21:07:19Z', '2026-02-27T22:04:00Z', 'comparison of recorded times', NULL, 'hr-prop-impaired-driving', 'reviewed');

INSERT INTO advocacy_items (id, case_id, kind, title, body, status, author) VALUES
  ('hr-issue-identity', 'case-hit-run-001', 'legal_issue', 'Driver and vehicle identity',
   'Separate vehicle ownership, vehicle comparison, and driver identity. The victim cannot identify the driver and the scene model reports only unstable partial plate characters.',
   'open', 'fixture-attorney'),
  ('hr-issue-dui-time', 'case-hit-run-001', 'motion_issue', 'Impairment at operation time',
   'Later observations and testing do not silently establish impairment at 21:07. Determine whether admissible evidence can bridge the 57-minute interval and evaluate the post-driving-consumption account.',
   'open', 'fixture-attorney'),
  ('hr-theory-lesser', 'case-hit-run-001', 'defense_theory', 'Property-damage lesser resolution',
   'Property damage and departure appear materially better supported than bodily injury or impairment at operation time. This is a negotiation hypothesis, not a recommendation or legal conclusion.',
   'open', 'fixture-attorney'),
  ('hr-task-native-video', 'case-hit-run-001', 'investigation_task', 'Obtain full native camera export',
   'Request pre-event and post-event footage, native metadata, signal phase log, export history, and untranscoded frames.',
   'open', 'fixture-attorney'),
  ('hr-task-route', 'case-hit-run-001', 'investigation_task', 'Test route and arrival-time account',
   'Obtain work departure records, residence camera material, receipts, witnesses, and vehicle access information.',
   'open', 'fixture-attorney'),
  ('hr-task-damage', 'case-hit-run-001', 'investigation_task', 'Independent vehicle comparison',
   'Preserve paint transfer and assess whether damage can exclude other collisions or vehicles.',
   'open', 'fixture-attorney'),
  ('hr-task-medical', 'case-hit-run-001', 'investigation_task', 'Review injury evidence',
   'Obtain medical records and compare onset, causation, and statutory injury definition with contemporaneous 911 statement.',
   'open', 'fixture-attorney');

INSERT INTO edges
  (id, case_id, source_kind, source_id, relation, target_kind, target_id, rationale, review_state, created_by)
VALUES
  ('hr-edge-identity-issue', 'case-hit-run-001', 'proposition', 'hr-prop-client-driver', 'relevant_to', 'advocacy', 'hr-issue-identity', 'Driver identity remains distinct from ownership.', 'reviewed', 'fixture-attorney'),
  ('hr-edge-car-issue', 'case-hit-run-001', 'proposition', 'hr-prop-client-car', 'relevant_to', 'advocacy', 'hr-issue-identity', 'Vehicle comparison remains non-unique.', 'reviewed', 'fixture-attorney'),
  ('hr-edge-dui-issue', 'case-hit-run-001', 'proposition', 'hr-prop-impaired-driving', 'relevant_to', 'advocacy', 'hr-issue-dui-time', 'Relevant time is driving, not later contact.', 'reviewed', 'fixture-attorney'),
  ('hr-edge-after-issue', 'case-hit-run-001', 'proposition', 'hr-prop-post-driving-alcohol', 'relevant_to', 'advocacy', 'hr-issue-dui-time', 'Alternative timing account requires corroboration.', 'reviewed', 'fixture-attorney'),
  ('hr-edge-identity-video', 'case-hit-run-001', 'advocacy', 'hr-issue-identity', 'requires_follow_up', 'advocacy', 'hr-task-native-video', 'Native frames and metadata bear directly on the partial plate.', 'reviewed', 'fixture-attorney'),
  ('hr-edge-identity-damage', 'case-hit-run-001', 'advocacy', 'hr-issue-identity', 'requires_follow_up', 'advocacy', 'hr-task-damage', 'Vehicle comparison is the identity question in physical form.', 'reviewed', 'fixture-attorney'),
  ('hr-edge-identity-route', 'case-hit-run-001', 'advocacy', 'hr-issue-identity', 'requires_follow_up', 'advocacy', 'hr-task-route', 'Route and vehicle access bear on who was driving.', 'reviewed', 'fixture-attorney'),
  ('hr-edge-dui-route', 'case-hit-run-001', 'advocacy', 'hr-issue-dui-time', 'requires_follow_up', 'advocacy', 'hr-task-route', 'Arrival time bounds the interval the state must bridge.', 'reviewed', 'fixture-attorney');

INSERT INTO decision_briefs
  (id, case_id, posture, summary, strengths, risks, unresolved_questions, client_topics, version, author)
VALUES
  ('hr-brief-negotiation-v1', 'case-hit-run-001', 'negotiation',
   'Evaluate a property-damage lesser resolution because departure and property damage are better supported than bodily injury or impairment at the time of operation.',
   'No eyewitness driver identification; non-unique vehicle comparison; contemporaneous no-injury statement; no impairment observation at 21:07; no retrograde estimate in produced report.',
   'Morgan admits driving that evening; vehicle damage is mutually consistent; video shows a meaningful impact, pause, and departure that may support knowledge.',
   'Can route and arrival time be fixed? Does native video improve or weaken identification? Can post-arrival drinking be corroborated? What medical evidence supports statutory injury?',
   'Review the camera clip and interview transcript; clarify route, vehicle access, awareness of impact, post-arrival alcohol timeline, and acceptable misdemeanor outcomes.',
   1, 'fixture-attorney');

UPDATE sources SET source_kind = 'document', temporal_relation = 'after_event'
WHERE id IN ('hr-src-crash-report', 'hr-src-victim', 'hr-src-damage', 'hr-src-test');
UPDATE sources SET source_kind = 'audio', temporal_relation = 'contemporaneous'
WHERE id = 'hr-src-911';
UPDATE sources SET source_kind = 'audio', temporal_relation = 'after_event'
WHERE id = 'hr-src-interview';
UPDATE sources SET source_kind = 'video', temporal_relation = 'contemporaneous'
WHERE id IN ('hr-src-camera', 'hr-src-full-video');
UPDATE sources SET source_kind = 'image_set', temporal_relation = 'after_event'
WHERE id = 'hr-src-photos';

UPDATE content SET content_created_at = '2026-02-27T23:30:00Z'
WHERE id IN ('hr-content-report-collision', 'hr-content-report-owner', 'hr-content-report-dui',
             'hr-content-full-video-ref');
UPDATE content SET content_created_at = '2026-02-28T11:00:00Z'
WHERE id IN ('hr-content-victim-impact', 'hr-content-victim-driver', 'hr-content-victim-injury');
UPDATE content SET content_created_at = '2026-03-03T14:00:00Z'
WHERE id IN ('hr-content-damage-match', 'hr-content-damage-limit');
UPDATE content SET content_created_at = '2026-02-27T21:08:00Z'
WHERE id IN ('hr-content-911-time', 'hr-content-911-injury');
UPDATE content SET content_created_at = normalized_start
WHERE id IN ('hr-content-video-vehicle', 'hr-content-video-impact', 'hr-content-video-depart',
             'hr-content-video-plate', 'hr-content-officer-observation',
             'hr-content-test-result', 'hr-content-photo-damage');
UPDATE content SET content_created_at = '2026-02-27T22:07:12Z'
WHERE id = 'hr-content-client-driving';
UPDATE content SET content_created_at = '2026-02-27T22:08:02Z'
WHERE id = 'hr-content-client-after-drink';
