//! Burned-in clock overlays. OCR reads what the camera painted on the frame;
//! it never normalizes that reading into a timeline.
//!
//! A body-worn or in-car camera burns a timestamp into the picture. Reading it
//! is an observation about the frame, so it lands as `raw_time` on a suggested
//! scene observation and nowhere else: no normalized interval, no `time_basis`,
//! no event. Normalization is a hypothesis a person writes down. A frame whose
//! overlay does not parse yields no reading at all — never a guess — and two
//! cameras whose overlays disagree stay disagreed, for the timeline lanes to
//! surface.

use std::path::{Path, PathBuf};
use std::process::Command;

use evidence_audio::format_locator;
use evidence_intake::{
    ContentKind, ExtractionProvenance, NormalizedBatch, NormalizedContent, NormalizedSegment,
    ReviewState,
};
use serde::{Deserialize, Serialize};

use crate::map::{VideoIdentity, scenes_to_batch};
use crate::scene::SceneAnalysis;
use crate::{Error, Result, SceneRequest, open_and_cut};

/// Extractor name written on a clock-overlay observation.
pub const EXTRACTOR_CLOCK: &str = "clock_overlay_ocr";

/// Which band of the frame the overlay was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayBand {
    /// The top ~15% of the frame.
    Top,
    /// The bottom ~15% of the frame.
    Bottom,
}

impl OverlayBand {
    /// Stable label used in locators and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }

    /// The bands an OCR pass looks at, in reading order.
    pub fn all() -> [Self; 2] {
        [Self::Top, Self::Bottom]
    }

    /// ffmpeg `crop` expression for this band of a still.
    pub fn crop_expression(self) -> &'static str {
        match self {
            Self::Top => "crop=iw:ih*0.15:0:0",
            Self::Bottom => "crop=iw:ih*0.15:0:ih*0.85",
        }
    }
}

/// One overlay timestamp read off one scene keyframe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockReading {
    /// One-based scene index the keyframe came from.
    pub scene_index: u32,
    /// Inclusive scene start on the original timeline.
    pub start_ms: u64,
    /// Exclusive scene end on the original timeline.
    pub end_ms: u64,
    /// Band of the frame the overlay sits in.
    pub band: OverlayBand,
    /// What OCR returned for that band, trimmed and otherwise untouched.
    pub raw_text: String,
    /// The timestamp parsed out of `raw_text`, verbatim characters.
    pub reading: String,
}

/// A JSON document of clock readings, from a prior run or a test fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClockDocument {
    /// Ordered readings.
    #[serde(default)]
    pub readings: Vec<ClockReading>,
    /// Backend version stored on each observation.
    #[serde(default)]
    pub version: Option<String>,
}

/// Text OCR returned for one band of one still, before parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawClockText {
    /// Band the text was cropped from.
    pub band: OverlayBand,
    /// What OCR returned, trimmed.
    pub text: String,
}

/// Something that can read text out of the overlay bands of a still.
pub trait ClockBackend {
    /// Read every overlay band of one working-copy still. A still with no
    /// legible overlay yields an empty vector, not an error.
    fn read(&self, still: &Path) -> Result<Vec<RawClockText>>;

    /// Version string stored on each observation.
    fn version(&self) -> String;
}

/// Reads a [`ClockDocument`] from disk. Used in tests and when the operator
/// already ran OCR over the stills.
#[derive(Debug, Clone)]
pub struct JsonClockBackend {
    /// Path to the JSON document.
    pub path: PathBuf,
}

impl JsonClockBackend {
    /// Load the document. Readings are already placed on the original timeline.
    pub fn load(&self) -> Result<ClockDocument> {
        let json = std::fs::read_to_string(&self.path).map_err(|error| {
            Error::Backend(format!("could not read {}: {error}", self.path.display()))
        })?;
        serde_json::from_str(&json).map_err(|error| {
            Error::Backend(format!(
                "{} is not a clock document: {error}",
                self.path.display()
            ))
        })
    }
}

/// Crops the overlay bands with ffmpeg and runs the Tesseract CLI on each.
///
/// Overnight, not realtime. A missing binary is a hard error with an install
/// hint; an illegible band is simply absent from the result.
#[derive(Debug, Clone)]
pub struct TesseractCliBackend {
    /// Binary name or path. Default `tesseract`.
    pub binary: PathBuf,
    /// Tesseract page-segmentation mode. Default 7, one text line.
    pub psm: u32,
    /// Tesseract language pack. Default `eng`.
    pub lang: String,
}

impl TesseractCliBackend {
    /// `tesseract` on PATH, one-line segmentation, English.
    pub fn default_local() -> Self {
        Self {
            binary: PathBuf::from("tesseract"),
            psm: 7,
            lang: "eng".to_owned(),
        }
    }

    fn ensure_binary(&self) -> Result<()> {
        let ok = Command::new(&self.binary)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if ok {
            Ok(())
        } else {
            Err(Error::Backend(format!(
                "could not run `{}`. Install Tesseract OCR (winget install UB-Mannheim.TesseractOCR, \
                 apt install tesseract-ocr, or brew install tesseract) or pass a JSON clock document.",
                self.binary.display()
            )))
        }
    }

    fn crop_band(still: &Path, band: OverlayBand, into: &Path) -> Result<()> {
        let status = Command::new("ffmpeg")
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(still)
            .args(["-vf", band.crop_expression(), "-frames:v", "1"])
            .arg(into)
            .status()
            .map_err(|error| {
                Error::Backend(format!(
                    "could not run ffmpeg to crop the {} overlay band: {error}. \
                     Install ffmpeg to read burned-in clocks.",
                    band.as_str()
                ))
            })?;
        if !status.success() || !into.is_file() {
            return Err(Error::Backend(format!(
                "ffmpeg did not write a {} band crop of {}",
                band.as_str(),
                still.display()
            )));
        }
        Ok(())
    }

    fn ocr(&self, png: &Path) -> Result<String> {
        let output = Command::new(&self.binary)
            .arg(png)
            .arg("stdout")
            .arg("-l")
            .arg(&self.lang)
            .args(["--psm", &self.psm.to_string()])
            .output()
            .map_err(|error| {
                Error::Backend(format!(
                    "could not run `{}`: {error}. Install Tesseract OCR or pass a JSON clock document.",
                    self.binary.display()
                ))
            })?;
        if !output.status.success() {
            return Err(Error::Backend(format!(
                "`{}` exited with {}: {}",
                self.binary.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

impl ClockBackend for TesseractCliBackend {
    fn read(&self, still: &Path) -> Result<Vec<RawClockText>> {
        self.ensure_binary()?;
        let work = tempfile::tempdir()
            .map_err(|error| Error::Backend(format!("could not create a crop dir: {error}")))?;
        let mut texts = Vec::new();
        for band in OverlayBand::all() {
            let png = work.path().join(format!("{}.png", band.as_str()));
            Self::crop_band(still, band, &png)?;
            let text = self.ocr(&png)?;
            if text.is_empty() {
                continue;
            }
            texts.push(RawClockText { band, text });
        }
        Ok(texts)
    }

    fn version(&self) -> String {
        format!("tesseract@{}/psm{}", self.lang, self.psm)
    }
}

/// Runs `backend` on every scene keyframe and keeps the timestamps that parse.
///
/// At most one reading per band per scene — the first that parses — ordered by
/// scene index then band. Scenes without a keyframe are skipped. A band whose
/// text does not parse is dropped rather than guessed at.
pub fn read_clocks(
    analysis: &SceneAnalysis,
    backend: &dyn ClockBackend,
) -> Result<Vec<ClockReading>> {
    let mut readings = Vec::new();
    for scene in &analysis.scenes {
        let Some(still) = scene.keyframe.as_ref() else {
            continue;
        };
        let texts = backend.read(&still.path)?;
        for band in OverlayBand::all() {
            let hit = texts
                .iter()
                .filter(|raw| raw.band == band)
                .find_map(|raw| parse_clock_text(&raw.text).map(|reading| (raw, reading)));
            if let Some((raw, reading)) = hit {
                readings.push(ClockReading {
                    scene_index: scene.index,
                    start_ms: scene.start_ms,
                    end_ms: scene.end_ms,
                    band,
                    raw_text: raw.text.trim().to_owned(),
                    reading,
                });
            }
        }
    }
    Ok(readings)
}

/// Appends one suggested observation per clock reading onto the original video.
///
/// The reading lands as `raw_time` only. Nothing here sets a normalized
/// interval or a `time_basis`: aligning a device clock to wall time is a
/// reviewable hypothesis a person writes.
pub fn attach_clock_readings(
    batch: &mut NormalizedBatch,
    identity: &VideoIdentity,
    readings: &[ClockReading],
    version: &str,
) -> Result<()> {
    let video = batch
        .sources
        .iter_mut()
        .find(|source| source.id == identity.source_id)
        .ok_or_else(|| Error::Mapping("batch has no original video source".to_owned()))?;
    for (ordinal, reading) in readings.iter().enumerate() {
        if reading.end_ms < reading.start_ms {
            return Err(Error::Mapping(format!(
                "clock reading on scene {} ends before it starts",
                reading.scene_index
            )));
        }
        if reading.reading.trim().is_empty() {
            return Err(Error::Mapping(format!(
                "clock reading on scene {} has no timestamp",
                reading.scene_index
            )));
        }
        video
            .segments
            .push(clock_segment(identity, reading, ordinal, version));
    }
    Ok(())
}

fn clock_segment(
    identity: &VideoIdentity,
    reading: &ClockReading,
    ordinal: usize,
    version: &str,
) -> NormalizedSegment {
    let locator = format!(
        "scene {}, {}; clock overlay ({})",
        reading.scene_index,
        format_locator(reading.start_ms, reading.end_ms),
        reading.band.as_str()
    );
    let raw = reading.raw_text.trim();
    let ocr_note = if raw == reading.reading {
        String::new()
    } else {
        format!(" OCR text was `{raw}`.")
    };
    NormalizedSegment {
        id: format!("{}-clk-{ordinal:04}", identity.source_id),
        locator,
        page: None,
        start_ms: Some(reading.start_ms),
        end_ms: Some(reading.end_ms),
        bounding_box: None,
        content: vec![NormalizedContent {
            id: format!("{}-clkc-{ordinal:04}", identity.source_id),
            kind: ContentKind::Observation,
            text: format!(
                "The burned-in clock overlay was read by OCR as `{}`.{ocr_note} \
                 Suggested; verify against the original frame.",
                reading.reading
            ),
            speaker_entity_id: None,
            attributed_to_entity_id: None,
            parent_content_id: None,
            raw_time: Some(format!("overlay {}", reading.reading)),
            content_created_at: None,
            asserted_time: None,
            normalized_start: None,
            normalized_end: None,
            time_basis: None,
            location_text: None,
            extraction: ExtractionProvenance {
                extractor: EXTRACTOR_CLOCK.to_owned(),
                version: version.to_owned(),
                machine_generated: true,
                confidence: None,
                review_state: ReviewState::Suggested,
            },
        }],
    }
}

/// Cuts scenes, reads every overlay band, and maps scenes plus readings.
pub fn read_clock_overlays(
    request: &SceneRequest,
    backend: &dyn ClockBackend,
) -> Result<NormalizedBatch> {
    let (identity, analysis) = open_and_cut(request)?;
    let readings = read_clocks(&analysis, backend)?;
    let mut batch = scenes_to_batch(&identity, &analysis)?;
    attach_clock_readings(&mut batch, &identity, &readings, &backend.version())?;
    Ok(batch)
}

/// Cuts scenes and attaches a JSON clock document.
pub fn read_clocks_from_json(request: &SceneRequest, json_path: &Path) -> Result<NormalizedBatch> {
    let document = JsonClockBackend {
        path: json_path.to_path_buf(),
    }
    .load()?;
    let (identity, analysis) = open_and_cut(request)?;
    let version = document
        .version
        .clone()
        .unwrap_or_else(|| "from-json".to_owned());
    let mut batch = scenes_to_batch(&identity, &analysis)?;
    attach_clock_readings(&mut batch, &identity, &document.readings, &version)?;
    Ok(batch)
}

/// Finds a device timestamp inside OCR text and returns it verbatim.
///
/// Only these shapes are recognised, anywhere in `text`:
///
/// - `YYYY-MM-DD HH:MM:SS`, optionally with a `Z` / `±HH:MM` / `±HHMM` offset
/// - `YYYY/MM/DD HH:MM:SS`, `MM/DD/YYYY HH:MM:SS`, `MM-DD-YYYY HH:MM:SS`,
///   `DD.MM.YYYY HH:MM:SS`
/// - `HH:MM:SS`, optionally ` AM` / ` PM`
///
/// Every shape may carry `.f`, `.ff` or `.fff` after the seconds, and it is
/// kept: the reading is the overlay's own characters, so dropping a fraction
/// would rewrite raw time. An offset is read on the ISO shape only.
///
/// Inside a digit run, `O` reads as `0` and `l` or `I` as `1`; the returned
/// string carries the corrected characters, and the caller keeps the OCR
/// original beside it so a reviewer sees both. Month, day, hour, minute and
/// second must be plausible. A range such as `21:07:06.400-21:07:19.000` is a
/// span, not a clock, and yields `None` — as does anything else. This function
/// guesses at nothing.
pub fn parse_clock_text(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len() {
        if let Some((end, reading)) = match_at(&chars, start) {
            if in_range_context(&chars, start, end) {
                // One end of a span. A span is not a clock, and the other end
                // is no better, so the whole text yields nothing.
                return None;
            }
            return Some(reading);
        }
    }
    None
}

fn match_at(chars: &[char], start: usize) -> Option<(usize, String)> {
    if start > 0 && digit_at(chars[start - 1]).is_some() {
        return None;
    }
    let dated = match_iso(chars, start)
        .or_else(|| match_ymd(chars, start, '/'))
        .or_else(|| match_dmy(chars, start, '/', DayFirst::No))
        .or_else(|| match_dmy(chars, start, '-', DayFirst::No))
        .or_else(|| match_dmy(chars, start, '.', DayFirst::Yes));
    let (end, text) = match dated {
        Some(found) => found,
        // A bare time directly after a date-shaped token means the date failed
        // its plausibility check. Read nothing rather than half of it.
        None if follows_failed_date(chars, start) => return None,
        None => match_bare_time(chars, start)?,
    };
    if digit_char(chars, end).is_some() {
        return None;
    }
    Some((end, text))
}

/// Whether the token before `start` is `NNNN sep NN sep NN` (in any order) that
/// no date shape accepted.
fn follows_failed_date(chars: &[char], start: usize) -> bool {
    let mut back = start;
    while back > 0 && chars[back - 1] == ' ' {
        back -= 1;
    }
    if back == start {
        return false;
    }
    let mut token = Vec::new();
    while back > 0
        && (digit_at(chars[back - 1]).is_some() || matches!(chars[back - 1], '/' | '-' | '.'))
    {
        back -= 1;
        token.push(chars[back]);
    }
    token.reverse();
    let mut groups = Vec::new();
    let mut separators = Vec::new();
    let mut run = 0_usize;
    for value in token {
        if digit_at(value).is_some() {
            run += 1;
        } else {
            groups.push(run);
            separators.push(value);
            run = 0;
        }
    }
    groups.push(run);
    groups.len() == 3
        && separators.len() == 2
        && separators[0] == separators[1]
        && groups.iter().all(|size| *size == 2 || *size == 4)
}

/// Whether the leading two-digit field of a `NN sep NN sep YYYY` date is a day.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DayFirst {
    Yes,
    No,
}

fn match_iso(chars: &[char], start: usize) -> Option<(usize, String)> {
    let (mut index, mut text) = match_ymd(chars, start, '-')?;
    if let Some((next, offset)) = take_offset(chars, index) {
        index = next;
        text.push_str(&offset);
    }
    Some((index, text))
}

fn match_ymd(chars: &[char], start: usize, sep: char) -> Option<(usize, String)> {
    let (index, _, year) = take_digits(chars, start, 4)?;
    let index = take_char(chars, index, sep)?;
    let (index, month, month_text) = take_digits(chars, index, 2)?;
    let index = take_char(chars, index, sep)?;
    let (index, day, day_text) = take_digits(chars, index, 2)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let index = take_char(chars, index, ' ')?;
    let (index, time, _) = take_time(chars, index)?;
    Some((
        index,
        format!("{year}{sep}{month_text}{sep}{day_text} {time}"),
    ))
}

fn match_dmy(chars: &[char], start: usize, sep: char, order: DayFirst) -> Option<(usize, String)> {
    let (index, first, first_text) = take_digits(chars, start, 2)?;
    let index = take_char(chars, index, sep)?;
    let (index, second, second_text) = take_digits(chars, index, 2)?;
    let index = take_char(chars, index, sep)?;
    let (index, _, year) = take_digits(chars, index, 4)?;
    let (month, day) = if order == DayFirst::Yes {
        (second, first)
    } else {
        (first, second)
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let index = take_char(chars, index, ' ')?;
    let (index, time, _) = take_time(chars, index)?;
    Some((
        index,
        format!("{first_text}{sep}{second_text}{sep}{year} {time}"),
    ))
}

fn match_bare_time(chars: &[char], start: usize) -> Option<(usize, String)> {
    let (index, mut text, hour) = take_time(chars, start)?;
    if (1..=12).contains(&hour)
        && let Some((next, meridiem)) = take_meridiem(chars, index)
    {
        text.push(' ');
        text.push_str(&meridiem);
        return Some((next, text));
    }
    Some((index, text))
}

fn take_time(chars: &[char], at: usize) -> Option<(usize, String, u32)> {
    let (index, hour, hour_text) = take_digits(chars, at, 2)?;
    let index = take_char(chars, index, ':')?;
    let (index, minute, minute_text) = take_digits(chars, index, 2)?;
    let index = take_char(chars, index, ':')?;
    let (index, second, second_text) = take_digits(chars, index, 2)?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut index = index;
    let mut text = format!("{hour_text}:{minute_text}:{second_text}");
    if let Some((next, fraction)) = take_fraction(chars, index) {
        index = next;
        text.push_str(&fraction);
    }
    Some((index, text, hour))
}

fn take_meridiem(chars: &[char], at: usize) -> Option<(usize, String)> {
    if chars.get(at) != Some(&' ') {
        return None;
    }
    let half = *chars.get(at + 1)?;
    let mark = *chars.get(at + 2)?;
    if !matches!(half, 'A' | 'a' | 'P' | 'p') || !matches!(mark, 'M' | 'm') {
        return None;
    }
    if chars.get(at + 3).is_some_and(char::is_ascii_alphanumeric) {
        return None;
    }
    Some((at + 3, format!("{half}{mark}")))
}

fn take_fraction(chars: &[char], at: usize) -> Option<(usize, String)> {
    if chars.get(at) != Some(&'.') {
        return None;
    }
    let mut text = String::from(".");
    let mut index = at + 1;
    while text.len() <= 3
        && let Some(digit) = digit_char(chars, index)
    {
        text.push(digit);
        index += 1;
    }
    if text.len() == 1 {
        return None;
    }
    Some((index, text))
}

fn take_offset(chars: &[char], at: usize) -> Option<(usize, String)> {
    match chars.get(at) {
        Some('Z') => Some((at + 1, "Z".to_owned())),
        Some(sign @ ('+' | '-')) => {
            let (index, hour, hour_text) = take_digits(chars, at + 1, 2)?;
            let (index, colon) = if chars.get(index) == Some(&':') {
                (index + 1, ":")
            } else {
                (index, "")
            };
            let (index, minute, minute_text) = take_digits(chars, index, 2)?;
            if hour > 14 || minute > 59 {
                return None;
            }
            Some((index, format!("{sign}{hour_text}{colon}{minute_text}")))
        }
        _ => None,
    }
}

fn take_digits(chars: &[char], at: usize, count: usize) -> Option<(usize, u32, String)> {
    let mut text = String::new();
    for offset in 0..count {
        text.push(digit_char(chars, at + offset)?);
    }
    let value = text.parse().ok()?;
    Some((at + count, value, text))
}

fn take_char(chars: &[char], at: usize, expected: char) -> Option<usize> {
    if chars.get(at) == Some(&expected) {
        Some(at + 1)
    } else {
        None
    }
}

fn digit_char(chars: &[char], at: usize) -> Option<char> {
    digit_at(*chars.get(at)?)
}

/// A digit, allowing the two substitutions OCR makes inside a digit run.
fn digit_at(value: char) -> Option<char> {
    match value {
        '0'..='9' => Some(value),
        'O' => Some('0'),
        'l' | 'I' => Some('1'),
        _ => None,
    }
}

fn is_dash(value: char) -> bool {
    matches!(
        value,
        '-' | '\u{2010}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}' | '~'
    )
}

/// Whether the match at `start..end` is one end of a range such as
/// `21:07:06.400-21:07:19.000`. A range is a span, not a clock reading.
fn in_range_context(chars: &[char], start: usize, end: usize) -> bool {
    let mut index = end;
    if chars.get(index) == Some(&'.') && digit_char(chars, index + 1).is_some() {
        index += 1;
        while digit_char(chars, index).is_some() {
            index += 1;
        }
    }
    while chars.get(index) == Some(&' ') {
        index += 1;
    }
    if chars.get(index).copied().is_some_and(is_dash) {
        index += 1;
        while chars.get(index) == Some(&' ') {
            index += 1;
        }
        if digit_char(chars, index).is_some() {
            return true;
        }
    }
    let mut back = start;
    while back > 0 && chars[back - 1] == ' ' {
        back -= 1;
    }
    if back > 0 && is_dash(chars[back - 1]) {
        back -= 1;
        while back > 0 && chars[back - 1] == ' ' {
            back -= 1;
        }
        if back > 0 && digit_at(chars[back - 1]).is_some() {
            return true;
        }
    }
    false
}
