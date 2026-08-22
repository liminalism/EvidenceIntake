# Mapping and forensic-video boundary

Observed 2026-08-20. These are external references, not project authority. The
project decides separately which boundary to adopt.

## NIST/OSAC forensic digital-video workflow

Source: [OSAC 2022-S-0031, Standard Guide for Forensic Digital Video
Examination Workflow, Version 2.0](https://www.nist.gov/document/osac-2022-s-0031-standard-guide-forensic-digital-video-examination-workflow-version-20)

The guide organizes forensic video examination into assessment, processing and
analysis domains. It describes analysis as applying subject-matter expertise to
interpret video data and draw opinions answering a forensic question. Its
analysis categories include authentication, photogrammetric analysis, content
analysis and comparative analysis. It also treats timeline-sequence
reconstruction—relating video, still images and other data to develop
chronological and contextual relationships—as part of the forensic workflow.

Project-relevant implication: authentication, measurements from imagery,
comparative identification, inferred geolocation and event or route
reconstruction require a forensic-methodology boundary. Merely returning a
reviewer to source-grounded material does not itself make such an opinion.

## National Institute of Justice on GIS

Source: [Bringing Geography to the Practice of Analyzing Crime Through
Technology](https://nij.ojp.gov/library/publications/bringing-geography-practice-analyzing-crime-through-technology)

NIJ describes geographic information systems as tools for visualizing and
manipulating geographic data, preparing it for analysis and displaying analysis
outputs. It distinguishes simple plotting from broader spatial analysis.

Project-relevant implication: a map can remain an organizational read model
when it plots supplied or reviewer-confirmed location anchors. Statistical
spatial analysis or machine-derived placement is a distinct expansion.

## Federal Rules of Evidence on aids and summaries

Source: [Federal Rules of Evidence, Rules 107 and
1006](https://www.uscourts.gov/sites/default/files/document/federal-rules-of-evidence.pdf)

Rule 107 addresses illustrative aids used to help a factfinder understand
evidence. Rule 1006 addresses summaries, charts or calculations offered to prove
the content of voluminous admissible writings, recordings or photographs and
requires the underlying originals or duplicates to be made available.

Project-relevant implication: an internal review map and an exhibit offered in
court are not the same product use. In either setting, preserving the original
location text, source identifier, exact locator, review state and a path back to
the underlying material avoids turning a visualization into an unsupported
substitute for its sources. Admissibility remains jurisdiction- and use-specific.

## Adoptable boundary

The relevant line is the software output, not the reviewer's professional title.
A public defender reviewing footage is analyzing evidence in the ordinary
litigation sense, but that alone does not make every review action a technical
forensic examination. The product remains on the collation side when it:

- plots only source-stated or reviewer-confirmed locations;
- preserves raw location language, provenance, exact locators and disagreement;
- labels any address-to-coordinate conversion as derived and reviewable; and
- uses the map to navigate back to document, audio or video evidence.

It crosses the adopted product boundary when it authenticates media, derives a
location from pixels, measures depicted objects or distances, compares scenes
or people to render an identification opinion, reconstructs routes or events, or
asserts that separately sourced items depict a common event.
