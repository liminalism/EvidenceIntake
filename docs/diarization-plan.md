# Speaker diarization plan: anonymous speaker turns as a second broker pack

Status: proposed, planning detail. The governing records are
`@evidence-intake-project.decision.single-broker-inference` (every live model runs
behind the Evidence broker) and
`@evidence-intake-project.decision.phonon-2-default-speech-pack` (Phonon-2 is the
default speech pack and does not diarize). The work record is
`@evidence-intake-project.audio.speaker-diarization-pack`. Nothing here changes the
kernel.

## Thesis

> Diarization answers "which spans were the same voice." It never answers "whose."

The audio adapter already knows what to do with a speaker label: it writes an
anonymous `SPEAKER_nn` observation beside the statement, emits a `speaker_candidate`
edge, and the enrichment rule and review queue carry that edge to a named person.
What the product lost when the default speech pack moved from Whisper to Phonon-2 is
the *producer* of those labels. This plan restores it as its own model pack, decoupled
from speech recognition, so each can be replaced alone.

The part that makes diarization work on real discovery audio is a pretrained
speaker-embedding model. Everything around it is ordinary code: voice activity
detection, windowing, clustering, turn smoothing, and alignment to transcript
segments. A model-free version (MFCC features plus clustering) is writable but
performs poorly on 911 and body-worn-camera audio, where far-field voices, overlap,
radio traffic and automatic gain control collapse the features together. A wrong
candidate costs a reviewer time, so the model-free version is not the default; it is
not built at all unless the pretrained path is blocked.

## Hard rules, restated

- **A label is a candidate, never an identity.** The worker refuses any label that is
  not of the form `SPEAKER_` followed by digits; the adapter keeps refusing the same at
  its boundary. Tying a label to a person is authoring work for a named person.
- **Absence is honest.** A segment with no dominant turn gets no label. No turn is
  invented to fill a gap, and the number of speakers is never reported as a fact.
- **Nothing is scored.** Clustering distances, embedding similarities and VAD
  probabilities are discarded inside the worker. No number crosses the broker
  boundary, reaches the kernel, or appears in a view or export.
- **Deterministic and pinned.** Thresholds, window sizes and smoothing rules are
  constants baked into the worker and the pack revision, never operator-tuned per
  recording. The same pack over the same bytes produces the same turns.
- **Brokered, checksummed, fail-closed.** The pack follows `runtime/tensorrt` exactly:
  manifest with pinned artifacts, real load probe, no network, no fallback. A broker
  without the pack means intake runs without diarization and says so.
- **Kernel stays clean.** No new table, no new content kind, no new edge kind. The
  existing `observation` content and `speaker_candidate` edge are the whole surface.

## Architecture

```text
original ──► audio adapter ──► working copy (48 kHz mono) ──► broker
                                   │                            ├── transcribe_audio (phonon-2)
                                   │                            └── diarize_audio   (speaker-turns)
                                   ▼
                       align turns to segments ──► NormalizedBatch
                       (statement + diarization observation + speaker_candidate edge)
```

### Protocol (`runtime/tensorrt/src/protocol.rs`)

- `Operation::DiarizeAudio`, display `diarize_audio`.
- `ResultBody::SpeakerTurns { turns: Vec<SpeakerTurn> }` with
  `SpeakerTurn { start_ms: u64, end_ms: u64, speaker: String }` on the original PCM
  timeline, ordered by start.
- Input is the same payload the speech pack takes: `audio/x-f32le`, 16 kHz, mono. One
  working copy serves both operations.
- `PROTOCOL_VERSION` stays at 2. Adding a tagged variant breaks no existing message;
  an older broker answers `unsupported_operation`, which is already fail-closed.

### Worker (`runtime/tensorrt/src/bin/evidence-trt-diarize-worker.rs`)

A native Rust worker, no Python, mirroring `evidence-trt-phonon-worker`:

1. **VAD.** Silero VAD (MIT) as an ONNX graph. Frames of 32 ms; a span is speech when
   the model says so for at least 250 ms, and two spans closer than 300 ms merge.
2. **Embeddings.** A WeSpeaker ResNet speaker-embedding model (Apache-2.0) as an ONNX
   graph. Log-mel features are computed in Rust to match the model's training
   front end exactly; this parity is a test, not an assumption. Windows of 1.5 s with
   a 0.75 s hop over speech spans only; a span shorter than one window is embedded
   whole.
3. **Clustering.** Agglomerative hierarchical clustering on cosine distance with a
   pinned stopping threshold. The threshold is chosen once on the labelled sample set
   (below) and recorded in the pack revision; it is not a runtime parameter.
4. **Turns.** Consecutive windows with one cluster become a turn; turns shorter than
   300 ms are dropped; gaps under 250 ms inside one cluster close. Labels are assigned
   in order of first appearance, `SPEAKER_00` first, so output is stable across runs.
5. **Probe.** On `load`, decode one second of synthetic audio end to end through both
   graphs before answering, as the Phonon worker does.

Subcommands: `serve` (the protocol loop), `pack` (assemble and verify a pack from the
two ONNX files plus the worker, pinning both model hashes), and a hidden
`diarize-fixture` used by integration tests, so the protocol, alignment and pack
builder are tested without a model.

ONNX runtime choice is the one open spike: `tract` is pure Rust and keeps
`unsafe_code = "forbid"` trivially, but Silero's newer graphs carry an STFT operator
it may not implement; `ort` binds the onnxruntime library and needs a packaged
shared library like TurboOCR does. The spike runs both graphs through `tract` first
and falls back to `ort` only if an operator is missing. Either answer is recorded as a
decision before phase 2 starts.

### Adapter (`adapters/audio`)

- A `DiarizationBackend` trait beside `TranscriptBackend`: `turns(&self, wav) ->
  Vec<SpeakerTurn>`, with a `TrtDiarizeBackend` over the broker and a
  `JsonFileBackend` for tests and `--turns-json`.
- `transcribe` gains an optional diarization backend. The adapter sends the same
  working copy to both operations and aligns afterwards.
- **Alignment rule.** For each transcript segment, the turn with the largest overlap
  wins if it covers at least half the segment; otherwise the segment gets no label.
  Overlapping speech therefore yields no label rather than a guess. The result is
  written into the existing `WhisperxSegment.speaker` slot so `transcript_to_batch`
  is unchanged.
- `EXTRACTOR_DIARIZE` becomes `broker_diarize`; the enrichment rule
  `diarization_candidates` in `src/store/enrichment_rules.rs` matches both the new
  name and the retired `whisperx_diarize` so older databases keep their candidates.
  This is the one kernel-side edit and it is a string match in one query.
- The stereo channel observation stays separate and deterministic. Diarization runs on
  the mono mix; a 911 recording with caller and dispatcher on different channels still
  gets its channel hypothesis from `channel_split`, and the two observations sit side
  by side for the reviewer.

### Job contract, CLI and GUI

- `AdapterProfile::Audio` and `::Video` gain `diarization: Option<ModelRef>` with a
  serde default of `None`, so queued requests written before the field exists still
  parse. `None` means intake runs without diarization.
- `DEFAULT_DIARIZATION_MODEL = "speaker-turns"` beside `DEFAULT_SPEECH_MODEL`. The
  GUI prefers it when the broker lists a `diarize_audio` pack and otherwise proceeds
  with `None`; the adapter's progress event names which case applied, so an intake
  without labels is visible in the queue rather than silent.
- `evidence-audio transcribe` and `evidence-video analyze` gain `--diarize`,
  `--diarize-model`, `--diarize-revision` and `--turns-json`, following the existing
  speech flags.
- The transcript badge in `src/gui/mod.rs` is unchanged; `is_transcript_extractor`
  already ignores diarization observations.

### Packaging

`evidence-trt-diarize-worker pack --vad silero-vad.onnx --embedding wespeaker.onnx
--out models/speaker-turns` writes a manifest with `runtime: native_cpu`, operations
`[diarize_audio]`, both graphs as checksummed artifacts, and a revision derived from the
two model hashes and the pinned threshold constant, so a retuned threshold is a new
revision. The directory is handed to `package_windows_tensorrt.ps1` through
`-ModelPacks` like any other pack. Weights, licence files and the model cards travel in
the pack.

## Evaluation gate

Phase 2 is not done, and the pack is not the default, until this passes on the Windows
host:

- A hand-labelled sample set of at least ten recordings the project may lawfully hold:
  synthetic multi-speaker clips, public-domain 911 and body-camera releases, and
  staff-recorded phone audio. Labels are speaker turns, authored by a person.
- Diarization error rate is measured for pack selection and threshold choice only. The
  number lives in the evidence record for the gate and nowhere else: not in the pack,
  not in a view, not in an export.
- The pinned threshold is the one that minimises error on that set; it is recorded in
  the decision that adopts the pack, with the sample set hash.
- A reviewer walks three recordings in the GUI and confirms that every label reads as
  a candidate, that unlabeled segments are visible as unlabeled, and that nothing in
  the queue or the standing view counts speakers.

If no pretrained pack reaches a usable error rate on that set, the plan stops at
"intake runs without diarization," which is where it stands today. The model-free
fallback is not built to paper over that result.

## Phases

0. **Spike (runtime choice).** Run Silero VAD and a WeSpeaker export through `tract`
   on Linux; measure operator coverage and per-minute latency. Decide `tract` or
   `ort`. Output: one decision record. Two to three days.
1. **Protocol.** Operation, result body, round-trip tests, fixture branch in the
   broker's `protocol_fixture`. Half a day.
2. **Worker.** VAD, features, embeddings, clustering, turns, probe, `pack`, fixture,
   integration tests through `Broker::discover`. Front-end parity test against the
   model's reference features. One to two weeks.
3. **Adapter.** Backend trait, broker backend, JSON backend, alignment, extractor
   rename, enrichment-rule match, tests named as rules (`overlapping_speech_gets_no_label`,
   `a_label_that_names_a_person_is_refused`, `older_whisperx_labels_still_become_candidates`).
   Two to three days.
4. **Contract, CLI, GUI.** Optional model ref with alias-safe default, flags, intake
   wiring, progress message, tests for legacy request parsing. One to two days.
5. **Evaluation and adoption.** Build the sample set, run the gate, pick the
   threshold, build the pack on the Windows host, record evidence, write the adopting
   decision, add the pack to the packaging doctor. One week, mostly labelling.

Phases 1 through 4 are buildable and testable on Linux with the fixture. Phase 0 needs
the two ONNX files; phase 5 needs the Windows host and the sample set.

## Acceptance

- `diarize_audio` round-trips through the broker with the fixture worker, and an
  installed real pack passes the doctor's load probe.
- Every label crossing the broker boundary is `SPEAKER_nn`; a worker that returns
  anything else is refused by the adapter, with a test.
- The adapter produces identical batches from a live broker and from `--turns-json`
  holding the same turns.
- A segment covered less than half by its best turn carries no label.
- A database with `whisperx_diarize` observations still yields `speaker_candidate`
  proposals after the rename.
- A queued audio or video job written before the `diarization` field existed still
  parses and runs without diarization.
- `nothing_in_the_standing_view_scores_the_case` and
  `a_disclosable_export_carries_no_privileged_material` still pass; no similarity,
  probability or speaker count appears in any serialized read model.
- The evaluation gate above is recorded as evidence before the pack becomes the GUI
  default.

## Out of scope

- Speaker identification, voice comparison against a known person, and linking the
  same label across recordings. Each is a forensic assertion and belongs to a retained
  expert, per `@evidence-intake-project.video.no-forensic-assertions`.
- Reporting a speaker count, confidence, or "likely the same person" in any view.
- Live or streaming diarization; the adapter works on files.
- Languages other than English for the speech pack; diarization itself is
  language-independent and this plan does not change that constraint either way.
