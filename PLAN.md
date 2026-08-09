# Evidence Intake — Preliminary Plan v1

**Working thesis:** A local-first evidence triage and collation workstation for low-budget
criminal law offices. Ingest a discovery dump (documents, audio, video), process it entirely
on-premises, and produce a unified, searchable, timestamped index where every extracted fact
carries provenance back to the unaltered original (page/line, audio timestamp, video frame).
The office searches and builds a timeline; the originals remain the evidence.

---

## 1. Who is the customer — defenders or prosecutors?

**Answer: public defenders first, private criminal defense bar second, small prosecutor
offices later.** Your own reasoning is correct, and the data backs it up.

### Why defenders

- **They receive raw evidence.** Discovery arrives as an unprocessed dump: bodycam, dashcam,
  jail calls, 911 audio, CCTV, phone extractions, scanned paper. Police forensic units work
  for the prosecution; the defense gets no pre-processing. A triage tool substitutes for the
  forensic support staff they don't have.
- **The volume problem is documented and worsening.** Reported figures: felony attorneys
  carrying ~100 cases with 4–6 hours of bodycam per case (≈400 hours of video per attorney);
  individual felony cases with up to 200 hours of footage; 63% of defenders say most cases
  involve audiovisual evidence; the dominant workflow is still "media player + Word document."
  Some offices report that what takes a prosecutor 2 hours takes them 40.
- **A worse irritant: vendor lock.** Police departments store evidence with third-party
  vendors, and defense attorneys sometimes must buy thousands of dollars of per-vendor
  software licenses just to *open* the files. A local tool that normalizes proprietary
  formats (GTL jail calls, Watchguard/AV-Viewer bodycam containers, FTR courtroom audio)
  removes a real, hated cost.
- **Demand is validated.** JusticeText built exactly this category (cloud transcription +
  flagging for defenders) and has paying offices. You are not proving a market exists; you
  are competing on deployment model, depth, and price.

### Why not prosecutors first

- They receive evidence already curated by police, and police forensics (Axon Evidence.com,
  Cellebrite, department video units) sits upstream of them.
- They are already served: Guardify, Axon, NICE Justice, Veritone. Procurement is slower and
  incumbent-friendly.
- **However** — small/rural DA offices getting evidence from small departments with no
  forensic capability are a genuine secondary market. Nothing in the architecture is
  defender-specific; keep the door open, but don't design for them in v1.

### Third segment worth noting

Private criminal defense solos/small firms: same workflow, actual money, shorter sales
cycle, and often *more* cloud-averse (client confidentiality, no IT department). Possibly
the best early-revenue segment even if defenders are the mission.

---

## 2. The legal question: can video forensics be "used at the law level"?

This is the pivotal design constraint, and the answer splits cleanly in two:

### Investigative / triage use — no admissibility bar at all

Using AI to *find* things (search transcripts, detect a gun at 02:14, summarize a scene) is
**attorney work product**. It never has to be admitted. What gets introduced in court is the
*original* video/audio/document, plus the attorney's or a human expert's testimony about it.
This is ~95% of the product's value and it is legally unencumbered. JusticeText operates
entirely in this zone.

### AI output *as evidence* — currently a minefield, avoid it

- *State v. Puloka* (Wash. Super. Ct. 2024): first known ruling on AI-enhanced video —
  **excluded** under Frye because generative enhancement "represents what the AI model
  thinks should be shown," not what happened, and isn't accepted in the forensic video
  community.
- **Proposed FRE 707** (machine-generated evidence): comment period closed Feb 2026, final
  committee vote scheduled May 2026, earliest effect Dec 2027. It would subject
  machine-generated output offered as evidence to Rule 702 expert-reliability standards
  (reliable principles and methods, reliably applied). State analogues will follow.

### Design consequences (these become selling points, not just constraints)

1. **Originals are immutable.** SHA-256 hash on intake, write-once storage, every derived
   artifact (binarized page, cleaned audio, transcript, detection) is a separate object
   pointing at the original + offset. This is a chain-of-custody story you can print on the
   brochure.
2. **Everything is a pointer.** OCR line → page + bounding box on original scan. Transcript
   word → millisecond offset in original audio. Detection/scene description → frame range in
   original video. One click from any search hit to the untouched source.
3. **Derived ≠ evidence, and the UI says so.** Binarized docs and denoised audio are labeled
   "working copies — verify against original." Transcripts and detections display confidence
   and are editable/annotatable by a human. The tool's output is a *finding aid*.
4. **No generative enhancement.** No super-resolution, no frame interpolation, no "clean up
   this face." Binarization (thresholding) and spectral denoising are conventional,
   well-understood signal processing — defensible if ever questioned, unlike generative
   models — and even those stay in the working-copy lane.

Framed this way, the honest marketing answer to "is this admissible?" is: *nothing this tool
produces needs to be admitted; it tells you where to look in the evidence you already have.*

---

## 3. Competitive landscape and the "too pedestrian?" test

| Product | Focus | Deployment | Analysis depth |
|---|---|---|---|
| **Guardify** (ex-VidaNyx) | DEMS: storage, sharing, chain of custody, CAC/MDT workflows; AI suite (transcription, summaries, chat, object tracking/redaction via Engage Vision) | Cloud | Moderate, bolt-on |
| **JusticeText** | Defender-side transcription of bodycam/jail calls/911; flags (Miranda, sobriety tests), clipping, 100+ languages, proprietary formats | Cloud | Deep on audio; little visual-channel analysis |
| **Axon Evidence / NICE / Veritone** | Law-enforcement/prosecutor DEMS ecosystems | Cloud | Varies; LE-oriented |
| **Reduct.video** | Transcription + clipping, mostly civil/deposition | Cloud | Audio only |
| **Everlaw / Relativity / Casetext** | General e-discovery / legal research | Cloud | Not AV-oriented |

Berkeley Law's survey of AI tools for public defenders lists ~15 tools. **Every single one
is cloud.** Their listed concerns: cost, funding-limited access, and cloud confidentiality.

### What would be pedestrian (kill-criteria territory)

- Cloud transcription + keyword search alone — JusticeText and Reduct already do this well.
- A generic DEMS — Guardify's whole company is that.

### What is not pedestrian (the actual product)

1. **Local-first, air-gapped capable.** Zero peers in the space. Sells on confidentiality
   (work product never leaves the office; protective orders on discovery sometimes bar
   third-party upload), on cost (no per-GB/per-seat cloud metering — discovery dumps are
   huge, cloud pricing punishes exactly this customer), and on rural offices with bad
   bandwidth. "Your evidence never leaves the building" is a one-sentence pitch a
   non-technical chief defender understands.
2. **Cross-modal collation.** Nobody merges documents + audio + video into one
   case-level timeline searchable by date/time/location/content. JusticeText gives you
   per-file transcripts; the mountain-of-evidence problem is *cross-file*: "show me
   everything from the night of the 14th near the intersection."
3. **Visual-channel analysis.** JusticeText analyzes what is *said*. Object detection
   (guns, knives, people) plus per-scene VLM narration analyzes what is *shown* — nobody
   serves this to the defense side. Bodycam audio is often useless (wind, shouting) exactly
   when the visual channel matters most.
4. **Flat, predictable pricing** (license + support, no metering) for offices with fixed
   annual budgets.

**Kill criteria (write these down and hold to them):** stop if Phase-0 interviews show
defenders are satisfied with JusticeText's coverage and unbothered by cloud; if offices
won't/can't host a GPU box; or if JusticeText ships local deployment + video analysis before
MVP. Re-evaluate honestly at end of Phase 1.

---

## 4. Architecture (all local)

Target hardware: one office workstation with a consumer GPU (RTX 4070–5090 class, 16–24 GB
VRAM), batch-oriented: evidence dumps process overnight, search is instant during the day.
This matches the workflow — intake is bursty (a discovery production lands), review is daily.

```
                    ┌─────────────────────────────────────────┐
                    │            Intake & Custody Core        │
                    │  hash (SHA-256) · dedup · format probe  │
                    │  proprietary-container normalization    │
                    │  metadata extraction (EXIF, bodycam GPS │
                    │  /timecode, filename heuristics) + form │
                    └───────┬──────────┬──────────┬───────────┘
                            │          │          │
                   ┌────────▼───┐ ┌────▼─────┐ ┌──▼────────────┐
                   │ Documents  │ │  Audio   │ │    Video      │
                   │ binarize   │ │ denoise  │ │ scene segment │
                   │ (Lege/     │ │ (DeepFil-│ │ detect objects│
                   │  jp2lam)   │ │  terNet) │ │ (YOLO-class / │
                   │ OCR w/ page│ │ WhisperX │ │ open-vocab)   │
                   │ +line+bbox │ │ + diariz.│ │ VLM per-scene │
                   │ provenance │ │ word-ts  │ │ description   │
                   └────────┬───┘ └────┬─────┘ │ audio → Audio │
                            │          │       └──┬────────────┘
                    ┌───────▼──────────▼──────────▼───────────┐
                    │         Collation & Index layer         │
                    │  SQLite case DB · Tantivy full-text ·    │
                    │  local embeddings (LanceDB) · unified    │
                    │  event schema: (case, source, offset/    │
                    │  page-line, datetime, location, text,    │
                    │  confidence, provenance-pointer)         │
                    └───────────────────┬─────────────────────┘
                    ┌───────────────────▼─────────────────────┐
                    │   UI (Freya): search · timeline view ·   │
                    │   media player w/ synced transcript ·    │
                    │   annotate/clip/export · verify-against- │
                    │   original everywhere                    │
                    └─────────────────────────────────────────┘
```

### Component notes

- **Intake/custody:** ffmpeg handles most containers; the proprietary-format long tail
  (GTL, Watchguard, FTR…) is unglamorous work that JusticeText proved customers value.
  Write-once original store + audit log of every access (cheap to build locally, and it
  mirrors Guardify's "immutable logs" pitch).
- **Documents (the Lege retool):** binarization for readability (you already have the
  imaging stack and jp2lam); OCR via a modern local engine (Tesseract as floor; PaddleOCR,
  docTR, or Surya as better defaults; a local VLM for handwriting/degraded scans later).
  Provenance = page + line + bounding box, stored per extracted line.
- **Audio:** WhisperX is the right call — word-level timestamps via forced alignment +
  pyannote diarization. Denoising: **DeepFilterNet** (Rust core, real-time-capable, local —
  fits your stack) as pre-processing *working copy only*; transcribe from cleaned audio but
  timestamps map to the original. Batched WhisperX large-v3 on a 4090 runs ~50–70×
  realtime: an attorney's entire 400-hour backlog transcribes in under a day. That single
  number is the demo.
- **Video:** three layers, in order of maturity:
  1. Scene segmentation (PySceneDetect-class) + keyframe extraction — cheap, structures
     everything downstream.
  2. Object detection with timestamped hits. Prefer **open-vocabulary detectors**
     (YOLO-World / Grounding-DINO class) over fixed-class gun models — "person holding
     object near vehicle" queries generalize; the two-stage classify-then-detect design
     from the arXiv paper you saved (2503.06317) is the right efficiency pattern for the
     rare-object case (gun in 200 hours of footage).
  3. Per-scene natural-language description via a current local VLM (Qwen2.5-VL-class) —
     your "modernize" instinct is right, and do **both**: detector for recall/timestamps
     ("find every frame with a gun"), VLM for narration/semantic search ("two people
     arguing with an officer, 1:00–4:30"). VLM is the expensive pass; run it per-scene on
     keyframes, not per-frame.
  - **Explicitly out (v1):** facial recognition (legally/politically radioactive in this
    market), generative enhancement (§2). Person re-identification *within* one video is
    acceptable later for the play-by-play feature.
  - **LingBot-Map** (streaming 3D reconstruction, in your folder): genuinely interesting
    for the "semantic reconstruction" ambition — reconstructing scene geometry from
    bodycam to establish positions/sightlines. Phase 4+, work-product-only, and by then
    FRE 707's fate will be known. Don't let it near the MVP.
- **Index/search:** SQLite as the case database, Tantivy for full-text, LanceDB (or
  sqlite-vec) for local embeddings → hybrid search. The unified event schema above is the
  heart of the product; get it right early because every pipeline writes into it.
- **UI:** your Freya fork. Core screens: intake wizard, processing queue, search, case
  timeline (by datetime/location), media player with synced transcript + detections, clip/
  export for motions. Export = the artifact attorneys actually file (clip + transcript
  excerpt + provenance citation).

---

## 5. Phased roadmap

### Phase 0 — Validation (2–4 weeks, before serious code)

- Interview 5–10 people: public defenders (state PD associations, NAPD/NLADA are the
  channel), 2–3 private criminal defense solos, ideally one small-county DA. Questions:
  current triage workflow, JusticeText awareness/objections, cloud policy for discovery
  materials, willingness to host a GPU workstation, budget authority and grant sources
  (JAG/BJA grants fund defender tech too).
- Assemble a realistic test corpus: public bodycam releases, court-released 911/jail-call
  audio, scanned court filings. Real bodycam audio is *terrible* — validate WhisperX +
  DeepFilterNet on it before promising anything.
- Benchmark JusticeText's actual output (demo/trial) so differentiation claims are honest.
- **Gate:** proceed only if ≥3 offices say "we would pilot a local box."

### Phase 1 — MVP: intake + audio + search (~2–3 months)

Audio first, not documents — it's the documented worst pain (bodycam/jail calls), the
clearest time-savings demo, and the pipeline is the most off-the-shelf. Deliver: intake/
custody core, WhisperX+diarization+denoise pipeline, unified event store, search UI with
synced playback, clip/export. This alone matches JusticeText's core, locally.

### Phase 2 — Documents (~1–2 months)

Lege retool: binarization + OCR + page/line provenance into the same event store. Now
"search the whole case" spans transcripts *and* paper — first cross-modal differentiation.

### Phase 3 — Video visual channel (~2–3 months)

Scene segmentation → open-vocab detection → VLM per-scene description, all into the event
store. This is the "nobody else has this for defenders" release.

### Phase 4 — Reconstruction & timeline intelligence

Cross-evidence timeline assembly (cluster events by datetime/location across files),
play-by-play summaries per case, LingBot-Map spatial reconstruction experiments.

---

## 6. Top risks

1. **Real-world audio/video quality.** Overlapping speech, wind, night footage. Mitigate:
   test on real corpus in Phase 0; show confidence scores; never overclaim.
2. **JusticeText velocity.** They could ship visual analysis or on-prem. Mitigate: local +
   cross-modal + flat pricing is a position, not a feature; move fast on Phase 1.
3. **Hardware at the customer.** Offices may lack a GPU box and an IT person. Mitigate:
   sell/lease a pre-configured workstation ("evidence appliance") — also solves support.
4. **Proprietary format long tail.** Budget ongoing time for it; it's a moat *because* it's
   miserable.
5. **Procurement/sales cycle.** Even defenders are government. Mitigate: private defense
   bar for early revenue; pilot programs; grant-funding assistance as part of the sale.
6. **Regulatory drift.** FRE 707 and state analogues (final vote May 2026). Low risk to the
   work-product positioning, but track it; it may actually *help* by making cloud AI
   vendors' "AI summaries as evidence" pitches harder.

---

## 7. Open questions for you

- Is Lege's imaging/GUI stack in a state to be retooled, or is this effectively a new
  workspace that borrows jp2lam + freya-main? (Affects Phase 2 estimate.)
- Appliance model (ship a box) vs. software-only install on customer hardware?
- Solo project or is there capacity to parallelize Phase 1 audio work with Phase 2 Lege
  retooling?
