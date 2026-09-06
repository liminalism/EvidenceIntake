//! `WinSafe` adapter for the platform-neutral evidence workspace.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use evidence_adapter_protocol::{
    ADAPTER_PROTOCOL_VERSION, AdapterJobRequest, AdapterProfile, ModelRef, TemporalRelationArg,
    VideoTier,
};
use evidence_trt::{Client, InputMetadata, Operation, Request, ResultBody};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use winsafe::{self as w, co, gui, prelude::*};

use office_core::{
    AppearanceType, CustodyState, DeadlineOrigin, MatterStatus, NoteScope, OfferState,
    PossiblePerson, Sex, USER_ROLES,
};

use super::{
    AuthorKind, COMMAND_SHEET, ClientDraft, DeadlineDraft, DeadlineGridRow, DocketGridRow,
    EnrichmentRow, EvidenceLink, GuiError, GuiResult, MatterDraft, NoteDraft, Pane, SettingDraft,
    Workspace, WorkspaceView, key_sheet, readable_text, review_target,
};
use crate::{
    ContentForm, EnrichmentField, EntityKind, ExportAudience, IntakeCoordinator, IntakeJobState,
    PerceptionBasis, PreviewDescriptor, ProposedSourceProfile, ReviewState, SourceRole,
    TemporalStance,
};

/// Design size of the client area, in logical units. `gui::dpi` scales every
/// coordinate below by the system DPI, so at 200% scaling this window wants
/// 2200x1400 device pixels — more than most screens have. `fit_to_work_area`
/// is what keeps it reachable.
const WIDTH: i32 = 1100;
const HEIGHT: i32 = 700;

/// Smallest logical client area the layout survives. Below `MIN_WIDTH` the
/// right-anchored authoring pair collides with `Reject` on the bottom row.
/// `MIN_HEIGHT` is set by the rail rather than by the output pane: the rail is
/// anchored and does not move, so its last button's lower edge — 600 — is the
/// floor, and anything less hides the export commands off the bottom.
/// `the_rail_fits_above_the_minimum_height` is what keeps the two in step.
const MIN_WIDTH: i32 = 940;
const MIN_HEIGHT: i32 = 620;

/// Left navigation rail: fixed width, fixed position, one button per row.
const RAIL_X: i32 = 20;
const RAIL_WIDTH: i32 = 180;
/// Vertical rule separating the rail from the working pane.
const DIVIDER_X: i32 = 208;
const DIVIDER_TOP: i32 = 70;

/// Working pane: search, output, and the review/authoring panel.
const PANE_X: i32 = 216;
const PANE_RIGHT: i32 = WIDTH - 20;
const PANE_WIDTH: i32 = PANE_RIGHT - PANE_X;

/// Action buttons use the roomier standard height.
const BUTTON_HEIGHT: i32 = 28;
/// Intake commands, which are few, keep the roomier pitch.
const INTAKE_RAIL_TOP: i32 = 78;
const INTAKE_RAIL_PITCH: i32 = 30;
/// Read-view controls are denser so twelve views fit above the action rail.
const VIEW_BUTTON_HEIGHT: i32 = 22;
const VIEW_RAIL_TOP: i32 = 182;
const VIEW_RAIL_PITCH: i32 = 24;
const ACTION_RAIL_TOP: i32 = 482;
const ACTION_RAIL_PITCH: i32 = 30;

/// Rail rules. The rules are the only thing that groups the rail's commands —
/// a heading over each group was tried and read as clutter.
const RAIL_RULES: [i32; 2] = [174, 474];

/// Offset and colour of each drop-shadow band, outermost first: the darker
/// band is painted last so it lands against the button's own edge. Both bands
/// stay close to the `BTNFACE` background, so a button reads as seated rather
/// than floating.
const SHADOW_BANDS: [(i32, (u8, u8, u8)); 2] = [(2, (0xEC, 0xEC, 0xEC)), (1, (0xDE, 0xDE, 0xDE))];
/// Etched separator: a dark line with a light line beneath it.
const RULE_SHADOW: (u8, u8, u8) = (0xD0, 0xD0, 0xD0);
const RULE_HIGHLIGHT: (u8, u8, u8) = (0xFC, 0xFC, 0xFC);

/// Command captions. A Win32 mnemonic is claimed window-wide, so a repeated
/// letter cycles focus between two buttons instead of pressing either one;
/// `every_command_has_its_own_alt_key` is what keeps these distinct. Static
/// labels take the same prefix, so a heading may not contain a bare `&`.
const OPEN_DATABASE: &str = "Open Data&base";
const NEW_CASE: &str = "New Case (&U)";
// The office entry commands carry no Alt letter — the namespace is exhausted —
// so each is reached by its `Ctrl+Shift` chord in `OFFICE_COMMANDS`.
const ACTING_AS: &str = "Acting As";
const NEW_CLIENT: &str = "New Client";
const NEW_MATTER: &str = "New Matter";
const NEW_SETTING: &str = "New Setting";
const NEW_DEADLINE: &str = "New Deadline";
const NEW_NOTE: &str = "New Note";
const INTAKE_EVIDENCE: &str = "Intake E&vidence";
const PROCESSING_QUEUE: &str = "P&rocessing Queue";
const ENRICHMENT_SWEEP: &str = "Enrichment Sweep (&X)";
const RUN_COLLATION: &str = "Find Tensions and Gaps (&L)";
const DISCLOSABLE_EXPORT: &str = "Disclosable &Export";
const WORK_FILE_EXPORT: &str = "&Privileged Work File";
const SAVE_EXPORT: &str = "Save Export to Dis&k";
const FIND: &str = "&Find";
const MARK_REVIEWED: &str = "M&ark Reviewed";
const VERIFY: &str = "Verif&y";
const REJECT: &str = "Re&ject";
const NEW_FROM_TEMPLATE: &str = "&New From Template";
const SAVE_AUTHORED: &str = "Save Ne&w Record";
const IMPORT_BATCH: &str = "Import Normali&zed Batch";

/// Workspace navigation, in the order a case is usually read.
///
/// The Alt namespace is full — twenty-five of the twenty-six letters are
/// claimed — so the two views added with the assembly layer carry no mnemonic
/// of their own. Every view is reachable by `Ctrl` accelerator instead, and
/// `every_command_has_its_own_accelerator` is what keeps both namespaces
/// unambiguous.
const VIEW_BUTTONS: [(&str, WorkspaceView); 12] = [
    ("&Overview", WorkspaceView::Overview),
    ("Case &Standing", WorkspaceView::Standing),
    ("&Discovery Ledger", WorkspaceView::Discovery),
    ("Element &Matrix", WorkspaceView::Elements),
    ("Contested &Timeline", WorkspaceView::Timeline),
    ("Time && Place Index (&G)", WorkspaceView::Collation),
    ("&Issue Workspaces", WorkspaceView::Issues),
    ("Offense &Comparison", WorkspaceView::Offenses),
    ("Review &Queue", WorkspaceView::ReviewQueue),
    ("Review &History", WorkspaceView::ReviewHistory),
    ("Proposition Packets", WorkspaceView::Packets),
    ("Case Digest", WorkspaceView::Digest),
];

/// `Ctrl` accelerators, which is how a command with no free Alt letter is
/// still reachable from the keyboard. The identifiers are far from `WinSafe`'s
/// automatic control identifiers, which count down from `0xdfff`.
///
const ACCEL_FIRST_VIEW: u16 = 0x0200;
const ACCEL_ENRICHMENT: u16 = 0x0220;
/// The pane switch, and the Court pane's own commands, each at a range of its
/// own so a later view can be added without walking into them.
const ACCEL_FIRST_PANE: u16 = 0x0240;
const ACCEL_FIRST_COURT: u16 = 0x0250;
const ACCEL_FIRST_OFFICE: u16 = 0x0260;

/// The two halves of the workspace, and the `Ctrl+Shift` letter that reaches
/// each. The Alt namespace is exhausted — every one of the twenty-six letters
/// is claimed by a command below — so the switch carries no mnemonic at all.
const PANE_BUTTONS: [(Pane, char); 2] = [(Pane::Court, 'K'), (Pane::Office, 'O')];

/// What the Court pane can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CourtCommand {
    /// Move the day to today.
    Today,
    /// Move back a week.
    PreviousWeek,
    /// Move back a day.
    PreviousDay,
    /// Move forward a day.
    NextDay,
    /// Move forward a week.
    NextWeek,
    /// Put the selected row on the clipboard.
    CopyRow,
    /// Seed the demonstration caseload into the office database.
    SeedOffice,
}

/// Court commands, their captions, and the `Ctrl+Shift` letter each carries.
///
/// None of these claims an Alt letter, for the reason stated above
/// [`PANE_BUTTONS`]; `every_command_has_its_own_accelerator` is what keeps the
/// chords unambiguous against the views' own.
const COURT_COMMANDS: [(&str, char, CourtCommand); 7] = [
    ("Today", 'T', CourtCommand::Today),
    ("Prev Week", 'B', CourtCommand::PreviousWeek),
    ("Prev Day", 'P', CourtCommand::PreviousDay),
    ("Next Day", 'N', CourtCommand::NextDay),
    ("Next Week", 'W', CourtCommand::NextWeek),
    ("Copy Row", 'C', CourtCommand::CopyRow),
    ("Seed Office Caseload", 'S', CourtCommand::SeedOffice),
];

/// What the Office pane's entry row can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OfficeCommand {
    /// Name the person every office write this session is attributed to.
    ActingAs,
    /// Open a client record, after the possible-duplicate question.
    NewClient,
    /// Open a matter, and its evidence case in the same flow.
    NewMatter,
    /// Schedule one setting covering several matters of one client.
    NewSetting,
    /// Record what is owed on a matter and when.
    NewDeadline,
    /// File a note under a client, a matter, or a setting.
    NewNote,
}

impl OfficeCommand {
    /// Every command, in the order the entry row shows them. Referenced by the
    /// tests that hold the table complete; the running window walks the table.
    #[cfg(test)]
    const ALL: [Self; 6] = [
        Self::ActingAs,
        Self::NewClient,
        Self::NewMatter,
        Self::NewSetting,
        Self::NewDeadline,
        Self::NewNote,
    ];
}

/// Office data entry, each on a `Ctrl+Shift` letter. The Alt namespace is
/// exhausted and the Court commands hold B, C, N, P, S, T and W, so the
/// letters here are the free ones nearest the word: L for cLient, H for
/// Hearing, J for Jot.
const OFFICE_COMMANDS: [(&str, char, OfficeCommand); 6] = [
    (ACTING_AS, 'A', OfficeCommand::ActingAs),
    (NEW_CLIENT, 'L', OfficeCommand::NewClient),
    (NEW_MATTER, 'M', OfficeCommand::NewMatter),
    (NEW_SETTING, 'H', OfficeCommand::NewSetting),
    (NEW_DEADLINE, 'D', OfficeCommand::NewDeadline),
    (NEW_NOTE, 'J', OfficeCommand::NewNote),
];

/// Docket grid columns, in logical units before DPI scaling.
///
/// One row is one setting, however many matters it covers, so `Matters` and
/// `Charges` hold several values on a line rather than splitting the row.
const DOCKET_COLUMNS: [(&str, i32); 11] = [
    ("Time", 56),
    ("For", 96),
    ("Court", 132),
    ("Client", 140),
    ("Matters", 140),
    ("Charges", 170),
    ("Custody", 92),
    ("Offer", 100),
    ("Last contact", 92),
    ("Evidence posture", 210),
    ("Open work", 100),
];

/// Deadline grid columns, in logical units before DPI scaling.
const DEADLINE_COLUMNS: [(&str, i32); 6] = [
    ("Due", 90),
    ("In", 120),
    ("Matter", 150),
    ("Client", 150),
    ("What is owed", 320),
    ("Origin", 120),
];

/// Court pane geometry. The two grids and the detail box stack down the full
/// width of the window, because the Court pane has no rail: on a court day the
/// question is what is on the docket, not which read model to open.
const COURT_ROW_TOP: i32 = 78;
const COURT_GRID_TOP: i32 = 118;
const COURT_GRID_HEIGHT: i32 = 240;
const COURT_DEADLINES_TOP: i32 = 388;
const COURT_DEADLINES_HEIGHT: i32 = 120;
const COURT_DETAIL_TOP: i32 = 538;
const COURT_DETAIL_HEIGHT: i32 = 120;
const COURT_STATUS_TOP: i32 = 666;

/// Office pane geometry: the output box, the entry row under it, and the
/// status line. The entry row is what makes the Office pane the appending
/// half of the workspace; the Court pane deliberately has no such row.
const OFFICE_OUTPUT_TOP: i32 = 118;
const OFFICE_OUTPUT_HEIGHT: i32 = 286;
const OFFICE_ENTRY_TOP: i32 = 412;
const OFFICE_STATUS_TOP: i32 = 446;

/// The `Ctrl` chord that reaches one view: the first ten take the digit row,
/// and later views take `Ctrl+Shift` over the same digits.
fn view_accelerator(index: usize) -> (bool, char) {
    match index {
        0..=8 => (
            false,
            char::from_digit(u32::try_from(index).unwrap_or(0) + 1, 10).unwrap_or('1'),
        ),
        9 => (false, '0'),
        other => (
            true,
            char::from_digit(u32::try_from(other).unwrap_or(10) - 9, 10).unwrap_or('1'),
        ),
    }
}

/// The chord as a reader sees it in the status line and the tests.
fn accelerator_label(shift: bool, key: char) -> String {
    if shift {
        format!("Ctrl+Shift+{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

const STATUS_HINT: &str = "Alt + the underlined letter runs a command; Ctrl + a digit opens a view; Enter in the search box finds. All derived material must be checked against the original.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntakeModality {
    Document,
    Audio,
    Video,
}

impl IntakeModality {
    const fn title(self) -> &'static str {
        match self {
            Self::Document => "Document intake",
            Self::Audio => "Audio intake",
            Self::Video => "Video intake",
        }
    }
}

#[derive(Debug, Clone)]
struct IntakeSelection {
    modality: IntakeModality,
    files: Vec<PathBuf>,
    production_index: usize,
    new_production: String,
    temporal: TemporalRelationArg,
    phone_band: bool,
    level_split: bool,
    overnight: bool,
}

#[derive(Debug)]
struct BrokerModels {
    by_operation: HashMap<Operation, Vec<ModelRef>>,
}

impl BrokerModels {
    fn required(&self, operation: Operation, preferred: &str) -> GuiResult<ModelRef> {
        let models = self.by_operation.get(&operation).ok_or_else(|| {
            GuiError::new(format!("The TensorRT broker has no model for {operation}."))
        })?;
        models
            .iter()
            .find(|model| model.id == preferred)
            .or_else(|| models.first())
            .cloned()
            .ok_or_else(|| {
                GuiError::new(format!("The TensorRT broker has no model for {operation}."))
            })
    }
}

#[derive(Clone)]
struct MainWindow {
    wnd: gui::WindowMain,
    workspace: Rc<RefCell<Workspace>>,
    coordinator: Rc<RefCell<Option<IntakeCoordinator>>>,
    /// Header captions, on screen in both panes.
    _labels: Vec<gui::Label>,
    /// Captions that belong to the Office pane and hide with it.
    office_labels: Vec<gui::Label>,
    /// Captions that belong to the Court pane and hide with it.
    court_labels: Vec<gui::Label>,
    pane_buttons: Vec<(Pane, gui::Button)>,
    court_date_edit: gui::Edit,
    court_buttons: Vec<(CourtCommand, gui::Button)>,
    docket_grid: gui::ListView<()>,
    deadline_grid: gui::ListView<()>,
    court_detail_edit: gui::Edit,
    court_status_label: gui::Label,
    matter_combo: gui::ComboBox,
    office_buttons: Vec<(OfficeCommand, gui::Button)>,
    acting_label: gui::Label,
    actor_label: gui::Label,
    database_edit: gui::Edit,
    open_button: gui::Button,
    case_combo: gui::ComboBox,
    new_case_button: gui::Button,
    intake_button: gui::Button,
    queue_button: gui::Button,
    enrichment_button: gui::Button,
    view_buttons: Vec<(WorkspaceView, gui::Button)>,
    suggest_button: gui::Button,
    safe_export_button: gui::Button,
    work_export_button: gui::Button,
    persist_export_button: gui::Button,
    search_mode: gui::ComboBox,
    search_edit: gui::Edit,
    search_button: gui::Button,
    output_edit: gui::Edit,
    status_label: gui::Label,
    actor_edit: gui::Edit,
    target_combo: gui::ComboBox,
    target_edit: gui::Edit,
    locator_edit: gui::Edit,
    payload_edit: gui::Edit,
    review_buttons: Vec<(ReviewState, gui::Button)>,
    author_combo: gui::ComboBox,
    template_button: gui::Button,
    author_button: gui::Button,
    import_button: gui::Button,
}

impl MainWindow {
    fn create_and_run(database: &Path) -> w::AnyResult<i32> {
        let workspace = Workspace::open(database)
            .or_else(|_| Workspace::in_memory())
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
        let workspace = Rc::new(RefCell::new(workspace));
        let coordinator = Rc::new(RefCell::new(
            workspace
                .borrow()
                .database()
                .and_then(|path| IntakeCoordinator::start(path).ok()),
        ));

        let mut accelerators = Vec::with_capacity(VIEW_BUTTONS.len() + 1);
        for index in 0..VIEW_BUTTONS.len() {
            let (shift, key) = view_accelerator(index);
            let mut modifiers = co::ACCELF::VIRTKEY | co::ACCELF::CONTROL;
            if shift {
                modifiers |= co::ACCELF::SHIFT;
            }
            accelerators.push(w::ACCEL {
                fVirt: modifiers,
                key: digit_key(key),
                cmd: ACCEL_FIRST_VIEW + u16::try_from(index).unwrap_or(0),
            });
        }
        accelerators.push(w::ACCEL {
            fVirt: co::ACCELF::VIRTKEY | co::ACCELF::CONTROL,
            key: co::VK::CHAR_E,
            cmd: ACCEL_ENRICHMENT,
        });
        // The pane switch and the Court commands, all on `Ctrl+Shift` letters:
        // the Alt namespace has no letter left to give any of them.
        for (index, (_, key)) in PANE_BUTTONS.iter().enumerate() {
            accelerators.push(w::ACCEL {
                fVirt: co::ACCELF::VIRTKEY | co::ACCELF::CONTROL | co::ACCELF::SHIFT,
                key: chord_key(*key),
                cmd: ACCEL_FIRST_PANE + u16::try_from(index).unwrap_or(0),
            });
        }
        for (index, (_, key, _)) in COURT_COMMANDS.iter().enumerate() {
            accelerators.push(w::ACCEL {
                fVirt: co::ACCELF::VIRTKEY | co::ACCELF::CONTROL | co::ACCELF::SHIFT,
                key: chord_key(*key),
                cmd: ACCEL_FIRST_COURT + u16::try_from(index).unwrap_or(0),
            });
        }
        for (index, (_, key, _)) in OFFICE_COMMANDS.iter().enumerate() {
            accelerators.push(w::ACCEL {
                fVirt: co::ACCELF::VIRTKEY | co::ACCELF::CONTROL | co::ACCELF::SHIFT,
                key: chord_key(*key),
                cmd: ACCEL_FIRST_OFFICE + u16::try_from(index).unwrap_or(0),
            });
        }
        let accel_table = w::HACCEL::CreateAcceleratorTable(&accelerators).ok();

        let wnd = gui::WindowMain::new(gui::WindowMainOpts {
            title: "Evidence Intake — Local Case Workspace",
            size: gui::dpi(WIDTH, HEIGHT),
            accel_table,
            // HREDRAW/VREDRAW repaint the whole client area on a resize, so the
            // shadows and rules follow the controls the layout has just moved.
            class_style: co::CS::DBLCLKS | co::CS::HREDRAW | co::CS::VREDRAW,
            style: co::WS::CAPTION
                | co::WS::SYSMENU
                | co::WS::MINIMIZEBOX
                | co::WS::MAXIMIZEBOX
                | co::WS::THICKFRAME
                | co::WS::CLIPCHILDREN,
            ..Default::default()
        });

        // --- Header: which database, and which half of the workspace ---------
        let labels = vec![
            label(
                &wnd,
                "EVIDENCE INTAKE  /  LOCAL CASE WORKSPACE",
                20,
                10,
                700,
                ANCHOR,
            ),
            label(&wnd, "Database", 20, 41, 62, ANCHOR),
        ];

        // The pane switch. Two plain buttons rather than a tab strip: a
        // `gui::Tab` page is repositioned only on `TCN::SELCHANGE` and never on
        // a parent resize, and correcting that needs `tcm::AdjustRect` through
        // an unsafe `SendMessage` — which `unsafe_code = "forbid"` rules out.
        // Buttons over two shown-and-hidden groups keep every existing
        // `resize_behavior` exactly as it was.
        let pane_buttons = PANE_BUTTONS
            .iter()
            .enumerate()
            .map(|(index, (pane, _))| {
                let column = i32::try_from(index).expect("two panes");
                (
                    *pane,
                    button(&wnd, pane.label(), 912 + column * 88, 6, 84, SLIDE_X),
                )
            })
            .collect::<Vec<_>>();

        let database_text = database.display().to_string();
        let database_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: &database_text,
                position: gui::dpi(86, 37),
                width: gui::dpi_x(448),
                resize_behavior: (gui::Horz::Resize, gui::Vert::None),
                ..Default::default()
            },
        );
        let open_button = button(&wnd, OPEN_DATABASE, 542, 34, 112, SLIDE_X);

        let mut office_labels = vec![label(&wnd, "Case", 674, 41, 34, SLIDE_X)];
        let initial_case_labels = case_labels(&workspace.borrow());
        let initial_case_refs = initial_case_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let case_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(712, 37),
                width: gui::dpi_x(228),
                items: &initial_case_refs,
                selected_item: workspace
                    .borrow()
                    .active_case_index()
                    .and_then(|index| u32::try_from(index).ok()),
                resize_behavior: (gui::Horz::Repos, gui::Vert::None),
                ..Default::default()
            },
        );
        let new_case_button = button(&wnd, NEW_CASE, 948, 34, 132, SLIDE_X);

        // --- Rail group 1: put evidence in the workspace, and read it --------
        let intake_button = button(
            &wnd,
            INTAKE_EVIDENCE,
            RAIL_X,
            INTAKE_RAIL_TOP,
            RAIL_WIDTH,
            ANCHOR,
        );
        let queue_button = button(
            &wnd,
            PROCESSING_QUEUE,
            RAIL_X,
            INTAKE_RAIL_TOP + INTAKE_RAIL_PITCH,
            RAIL_WIDTH,
            ANCHOR,
        );
        let enrichment_button = button(
            &wnd,
            ENRICHMENT_SWEEP,
            RAIL_X,
            INTAKE_RAIL_TOP + 2 * INTAKE_RAIL_PITCH,
            RAIL_WIDTH,
            ANCHOR,
        );

        // --- Rail group 2: read the case -------------------------------------
        let view_buttons = VIEW_BUTTONS
            .iter()
            .enumerate()
            .map(|(index, (caption, view))| {
                let row = i32::try_from(index).expect("navigation has a small, fixed row count");
                (
                    *view,
                    view_button(
                        &wnd,
                        caption,
                        RAIL_X,
                        VIEW_RAIL_TOP + row * VIEW_RAIL_PITCH,
                        RAIL_WIDTH,
                        ANCHOR,
                    ),
                )
            })
            .collect::<Vec<_>>();

        // --- Rail group 3: act on the case -----------------------------------
        let suggest_button = button(
            &wnd,
            RUN_COLLATION,
            RAIL_X,
            ACTION_RAIL_TOP,
            RAIL_WIDTH,
            ANCHOR,
        );
        let safe_export_button = button(
            &wnd,
            DISCLOSABLE_EXPORT,
            RAIL_X,
            ACTION_RAIL_TOP + ACTION_RAIL_PITCH,
            RAIL_WIDTH,
            ANCHOR,
        );
        let work_export_button = button(
            &wnd,
            WORK_FILE_EXPORT,
            RAIL_X,
            ACTION_RAIL_TOP + 2 * ACTION_RAIL_PITCH,
            RAIL_WIDTH,
            ANCHOR,
        );
        let persist_export_button = button(
            &wnd,
            SAVE_EXPORT,
            RAIL_X,
            ACTION_RAIL_TOP + 3 * ACTION_RAIL_PITCH,
            RAIL_WIDTH,
            ANCHOR,
        );

        // --- Pane: which matter, and search over its originals ----------------
        // The matter chooser sits above the output pane rather than beside the
        // case chooser in the header, because a matter is how the office
        // actually reaches discovery: a court number, not a case identifier.
        office_labels.push(label(&wnd, "Matter", PANE_X, 82, 46, ANCHOR));
        let matter_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(262, 78),
                width: gui::dpi_x(238),
                items: &[],
                ..Default::default()
            },
        );
        let search_mode = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(512, 78),
                width: gui::dpi_x(120),
                items: &["Text", "Video Frames"],
                selected_item: Some(0),
                ..Default::default()
            },
        );
        let search_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: "",
                position: gui::dpi(642, 81),
                width: gui::dpi_x(310),
                resize_behavior: (gui::Horz::Resize, gui::Vert::None),
                ..Default::default()
            },
        );
        let search_button = button(&wnd, FIND, 960, 78, 120, SLIDE_X);

        // --- Pane: the read model currently on screen ------------------------
        let output_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: "",
                position: gui::dpi(PANE_X, OFFICE_OUTPUT_TOP),
                width: gui::dpi_x(PANE_WIDTH),
                height: gui::dpi_y(OFFICE_OUTPUT_HEIGHT),
                control_style: co::ES::MULTILINE
                    | co::ES::AUTOVSCROLL
                    | co::ES::AUTOHSCROLL
                    | co::ES::READONLY
                    | co::ES::WANTRETURN,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::BORDER
                    | co::WS::VSCROLL
                    | co::WS::HSCROLL
                    | co::WS::TABSTOP,
                resize_behavior: (gui::Horz::Resize, gui::Vert::Resize),
                ..Default::default()
            },
        );
        let status_label = label(
            &wnd,
            STATUS_HINT,
            PANE_X,
            OFFICE_STATUS_TOP,
            PANE_WIDTH,
            STRETCH_X_SLIDE_Y,
        );

        // --- Pane: the office entry row ---------------------------------------
        // Where records enter the workspace. Every command also carries a
        // `Ctrl+Shift` chord, because entry is fastest when the mouse never
        // has to be picked up; the acting-as label sits in the gap after the
        // first button so the row also says who is writing.
        let office_layout: [(i32, i32); 6] = [
            (PANE_X, 104),
            (484, 100),
            (590, 100),
            (696, 100),
            (802, 106),
            (914, 90),
        ];
        let office_buttons = OFFICE_COMMANDS
            .iter()
            .zip(office_layout)
            .map(|((caption, _, command), (x, width))| {
                (
                    *command,
                    button(&wnd, caption, x, OFFICE_ENTRY_TOP, width, SLIDE_Y),
                )
            })
            .collect::<Vec<_>>();
        let acting_label = label(
            &wnd,
            "Acting as: nobody yet",
            326,
            OFFICE_ENTRY_TOP + 5,
            150,
            SLIDE_Y,
        );

        // --- Pane: the human review and authoring panel ----------------------
        // The rule above this panel is anchored to the first field label, so it
        // follows the panel when the window grows.
        let actor_label = label(&wnd, "Actor", PANE_X, 482, 60, SLIDE_Y);
        office_labels.extend([
            label(&wnd, "Target kind", 356, 482, 84, SLIDE_Y),
            label(&wnd, "Target ID", 476, 482, 80, SLIDE_Y),
            label(&wnd, "Original locator", 676, 482, 110, SLIDE_Y),
        ]);

        let actor_edit = edit(&wnd, PANE_X, 502, 132, SLIDE_Y);
        let target_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(356, 502),
                width: gui::dpi_x(112),
                items: &["content", "source", "edge", "proposition", "event"],
                selected_item: Some(0),
                resize_behavior: SLIDE_Y,
                ..Default::default()
            },
        );
        let target_edit = edit(&wnd, 476, 502, 192, SLIDE_Y);
        let locator_edit = edit(&wnd, 676, 502, 404, STRETCH_X_SLIDE_Y);

        // The payload box is shared: a review reads it as the written basis, an
        // authoring or intake command reads it as JSON. Say so above the box,
        // and keep the template loader beside the selector that fills it.
        office_labels.extend([
            label(
                &wnd,
                "Basis for a review decision, or JSON for authoring and intake",
                PANE_X,
                538,
                420,
                // Resizes rather than sitting still, so its right edge keeps
                // its distance from the authoring selector as the window
                // narrows instead of running underneath it.
                STRETCH_X_SLIDE_Y,
            ),
            label(&wnd, "Authoring kind", 644, 538, 92, SLIDE_XY),
        ]);
        let author_labels = AuthorKind::ALL.map(AuthorKind::label);
        let author_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(740, 534),
                width: gui::dpi_x(160),
                items: &author_labels,
                selected_item: Some(0),
                resize_behavior: SLIDE_XY,
                ..Default::default()
            },
        );
        let template_button = button(&wnd, NEW_FROM_TEMPLATE, 908, 532, 172, SLIDE_XY);

        let initial_template = windows_lines(AuthorKind::Proposition.template());
        let payload_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: &initial_template,
                position: gui::dpi(PANE_X, 566),
                width: gui::dpi_x(PANE_WIDTH),
                height: gui::dpi_y(72),
                control_style: co::ES::MULTILINE
                    | co::ES::AUTOVSCROLL
                    | co::ES::AUTOHSCROLL
                    | co::ES::WANTRETURN,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::BORDER
                    | co::WS::VSCROLL
                    | co::WS::HSCROLL
                    | co::WS::TABSTOP,
                resize_behavior: STRETCH_X_SLIDE_Y,
                ..Default::default()
            },
        );

        // Deciding verbs on the left, writing verbs on the right, so a reviewer
        // never reaches past an authoring button to record a decision.
        let review_buttons = vec![
            (
                ReviewState::Reviewed,
                button(&wnd, MARK_REVIEWED, PANE_X, 650, 120, SLIDE_Y),
            ),
            (
                ReviewState::Verified,
                button(&wnd, VERIFY, 344, 650, 120, SLIDE_Y),
            ),
            (
                ReviewState::Rejected,
                button(&wnd, REJECT, 472, 650, 120, SLIDE_Y),
            ),
        ];
        let author_button = button(&wnd, SAVE_AUTHORED, 786, 650, 118, SLIDE_XY);
        let import_button = button(&wnd, IMPORT_BATCH, 912, 650, 168, SLIDE_XY);

        // --- The Court pane: what the office has to be in court for ----------
        // No rail here. On a court day the question is what is on the docket,
        // so the two grids and the detail box take the whole width, and the
        // day's navigation sits on one line above them.
        let mut court_labels = vec![label(&wnd, "Day", 20, COURT_ROW_TOP + 4, 34, ANCHOR)];
        let court_date_edit = edit(&wnd, 58, COURT_ROW_TOP, 110, ANCHOR);
        let mut court_buttons = Vec::with_capacity(COURT_COMMANDS.len());
        for (caption, _, command) in COURT_COMMANDS {
            let (x, width, behavior) = match command {
                CourtCommand::Today => (178, 84, ANCHOR),
                CourtCommand::PreviousWeek => (268, 96, ANCHOR),
                CourtCommand::PreviousDay => (370, 90, ANCHOR),
                CourtCommand::NextDay => (466, 90, ANCHOR),
                CourtCommand::NextWeek => (562, 96, ANCHOR),
                CourtCommand::CopyRow => (824, 96, SLIDE_X),
                CourtCommand::SeedOffice => (926, 154, SLIDE_X),
            };
            court_buttons.push((
                command,
                button(&wnd, caption, x, COURT_ROW_TOP, width, behavior),
            ));
        }

        let docket_columns = DOCKET_COLUMNS
            .map(|(caption, width)| (caption, gui::dpi_x(width)))
            .to_vec();
        let docket_grid = gui::ListView::<()>::new(
            &wnd,
            gui::ListViewOpts {
                position: gui::dpi(20, COURT_GRID_TOP),
                size: gui::dpi(WIDTH - 40, COURT_GRID_HEIGHT),
                columns: &docket_columns,
                control_ex_style: co::LVS_EX::FULLROWSELECT | co::LVS_EX::GRIDLINES,
                resize_behavior: (gui::Horz::Resize, gui::Vert::Resize),
                ..Default::default()
            },
        );

        court_labels.push(label(
            &wnd,
            "What is owed, counted from the day above",
            20,
            COURT_DEADLINES_TOP - 20,
            520,
            SLIDE_Y,
        ));
        let deadline_columns = DEADLINE_COLUMNS
            .map(|(caption, width)| (caption, gui::dpi_x(width)))
            .to_vec();
        let deadline_grid = gui::ListView::<()>::new(
            &wnd,
            gui::ListViewOpts {
                position: gui::dpi(20, COURT_DEADLINES_TOP),
                size: gui::dpi(WIDTH - 40, COURT_DEADLINES_HEIGHT),
                columns: &deadline_columns,
                control_ex_style: co::LVS_EX::FULLROWSELECT | co::LVS_EX::GRIDLINES,
                resize_behavior: STRETCH_X_SLIDE_Y,
                ..Default::default()
            },
        );

        // Selecting a setting expands it here rather than navigating away from
        // the docket — the record stays on screen while it is being read.
        court_labels.push(label(
            &wnd,
            "The setting under the cursor: the client, every matter it covers, and what their evidence rests on",
            20,
            COURT_DETAIL_TOP - 20,
            860,
            SLIDE_Y,
        ));
        let court_detail_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: "",
                position: gui::dpi(20, COURT_DETAIL_TOP),
                width: gui::dpi_x(WIDTH - 40),
                height: gui::dpi_y(COURT_DETAIL_HEIGHT),
                // Read-only, but not disabled: an edit control still answers
                // Ctrl+C, which is how a name or a number is lifted out of the
                // detail without being retyped.
                control_style: co::ES::MULTILINE
                    | co::ES::AUTOVSCROLL
                    | co::ES::AUTOHSCROLL
                    | co::ES::READONLY,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::BORDER
                    | co::WS::VSCROLL
                    | co::WS::HSCROLL
                    | co::WS::TABSTOP,
                resize_behavior: STRETCH_X_SLIDE_Y,
                ..Default::default()
            },
        );
        let court_status_label = label(
            &wnd,
            "No day is loaded.",
            20,
            COURT_STATUS_TOP,
            WIDTH - 40,
            STRETCH_X_SLIDE_Y,
        );

        let new_self = Self {
            wnd,
            workspace,
            coordinator,
            _labels: labels,
            office_labels,
            court_labels,
            pane_buttons,
            court_date_edit,
            court_buttons,
            docket_grid,
            deadline_grid,
            court_detail_edit,
            court_status_label,
            matter_combo,
            office_buttons,
            acting_label,
            actor_label,
            database_edit,
            open_button,
            case_combo,
            new_case_button,
            intake_button,
            queue_button,
            enrichment_button,
            view_buttons,
            suggest_button,
            safe_export_button,
            work_export_button,
            persist_export_button,
            search_mode,
            search_edit,
            search_button,
            output_edit,
            status_label,
            actor_edit,
            target_combo,
            target_edit,
            locator_edit,
            payload_edit,
            review_buttons,
            author_combo,
            template_button,
            author_button,
            import_button,
        };
        new_self.events();
        new_self.wnd.run_main(None)
    }

    /// Every push button, in creation order, for the shadow pass.
    fn buttons(&self) -> Vec<&gui::Button> {
        let mut all = vec![&self.open_button];
        all.extend(self.pane_buttons.iter().map(|(_, control)| control));
        all.extend(self.court_buttons.iter().map(|(_, control)| control));
        all.extend(self.office_buttons.iter().map(|(_, control)| control));
        all.extend([
            &self.new_case_button,
            &self.intake_button,
            &self.queue_button,
            &self.enrichment_button,
        ]);
        all.extend(self.view_buttons.iter().map(|(_, control)| control));
        all.extend([
            &self.suggest_button,
            &self.safe_export_button,
            &self.work_export_button,
            &self.persist_export_button,
            &self.search_button,
        ]);
        all.extend(self.review_buttons.iter().map(|(_, control)| control));
        all.extend([
            &self.template_button,
            &self.author_button,
            &self.import_button,
        ]);
        all
    }

    /// Every control that belongs to the Court pane.
    fn court_windows(&self) -> Vec<&w::HWND> {
        let mut all = vec![
            self.court_date_edit.hwnd(),
            self.docket_grid.hwnd(),
            self.deadline_grid.hwnd(),
            self.court_detail_edit.hwnd(),
            self.court_status_label.hwnd(),
        ];
        all.extend(self.court_labels.iter().map(gui::Label::hwnd));
        all.extend(self.court_buttons.iter().map(|(_, control)| control.hwnd()));
        all
    }

    /// Every control that belongs to the Office pane.
    ///
    /// The header — which database, and the pane switch itself — is in neither
    /// list, because it stays on screen in both.
    fn office_windows(&self) -> Vec<&w::HWND> {
        let mut all = vec![
            self.case_combo.hwnd(),
            self.new_case_button.hwnd(),
            self.matter_combo.hwnd(),
            self.intake_button.hwnd(),
            self.queue_button.hwnd(),
            self.enrichment_button.hwnd(),
            self.suggest_button.hwnd(),
            self.safe_export_button.hwnd(),
            self.work_export_button.hwnd(),
            self.persist_export_button.hwnd(),
            self.search_mode.hwnd(),
            self.search_edit.hwnd(),
            self.search_button.hwnd(),
            self.output_edit.hwnd(),
            self.status_label.hwnd(),
            self.actor_label.hwnd(),
            self.actor_edit.hwnd(),
            self.target_combo.hwnd(),
            self.target_edit.hwnd(),
            self.locator_edit.hwnd(),
            self.payload_edit.hwnd(),
            self.author_combo.hwnd(),
            self.template_button.hwnd(),
            self.author_button.hwnd(),
            self.import_button.hwnd(),
            self.acting_label.hwnd(),
        ];
        all.extend(
            self.office_buttons
                .iter()
                .map(|(_, control)| control.hwnd()),
        );
        all.extend(self.office_labels.iter().map(gui::Label::hwnd));
        all.extend(self.view_buttons.iter().map(|(_, control)| control.hwnd()));
        all.extend(
            self.review_buttons
                .iter()
                .map(|(_, control)| control.hwnd()),
        );
        all
    }

    /// Shows one pane and hides the other.
    ///
    /// Every control stays a direct child of the main window and keeps the
    /// `resize_behavior` it was built with; only its visibility changes. That
    /// is why this is `ShowWindow` over two groups rather than a tab control,
    /// whose pages would need an unsafe `tcm::AdjustRect` to survive a resize.
    fn show_pane(&self, pane: Pane) -> w::SysResult<()> {
        self.workspace.borrow_mut().set_pane(pane);
        let court = pane == Pane::Court;
        let (shown, hidden) = if court {
            (self.court_windows(), self.office_windows())
        } else {
            (self.office_windows(), self.court_windows())
        };
        // Hide first: showing first would flash the incoming controls over the
        // outgoing ones for a frame.
        for hwnd in hidden {
            hwnd.ShowWindow(co::SW::HIDE);
        }
        for hwnd in shown {
            hwnd.ShowWindow(co::SW::SHOW);
        }
        for (candidate, control) in &self.pane_buttons {
            control.hwnd().EnableWindow(*candidate != pane);
        }
        self.wnd.hwnd().InvalidateRect(None, true)?;
        if court {
            self.refresh_court()?;
        }
        Ok(())
    }

    /// Reloads the day on screen and both of its grids.
    fn refresh_court(&self) -> w::SysResult<()> {
        let loaded = self.workspace.borrow_mut().refresh_court();
        let (date, summary) = {
            let workspace = self.workspace.borrow();
            (
                workspace.docket_date().to_owned(),
                workspace.docket_summary(),
            )
        };
        self.court_date_edit.hwnd().SetWindowText(&date)?;
        match loaded {
            Ok(()) => {
                self.fill_docket_grid()?;
                self.court_status_label.hwnd().SetWindowText(&summary)?;
            }
            Err(error) => {
                self.docket_grid.items().delete_all()?;
                self.deadline_grid.items().delete_all()?;
                self.court_detail_edit.hwnd().SetWindowText("")?;
                self.court_status_label
                    .hwnd()
                    .SetWindowText(&error.to_string())?;
            }
        }
        self.refresh_matter_combo()
    }

    fn fill_docket_grid(&self) -> w::SysResult<()> {
        let (settings, owed) = {
            let workspace = self.workspace.borrow();
            (workspace.docket_rows(), workspace.deadline_rows())
        };

        self.docket_grid.set_redraw(false);
        self.docket_grid.items().delete_all()?;
        for row in &settings {
            self.docket_grid.items().add(&docket_texts(row), None, ())?;
        }
        self.docket_grid.set_redraw(true);

        self.deadline_grid.set_redraw(false);
        self.deadline_grid.items().delete_all()?;
        for row in &owed {
            self.deadline_grid
                .items()
                .add(&deadline_texts(row), None, ())?;
        }
        self.deadline_grid.set_redraw(true);

        if self.docket_grid.items().count() > 0 {
            let first = self.docket_grid.items().get(0);
            first.select(true)?;
            first.focus()?;
        }
        self.refresh_court_detail()
    }

    /// Fills the detail box from whichever setting the cursor is on.
    fn refresh_court_detail(&self) -> w::SysResult<()> {
        let Some(index) = self.docket_grid.items().focused() else {
            self.court_detail_edit
                .hwnd()
                .SetWindowText("Select a setting to see who it is for and what it rests on.")?;
            return Ok(());
        };
        let text = self
            .workspace
            .borrow()
            .court_detail(index.index() as usize)
            .unwrap_or_else(|error| error.to_string());
        self.court_detail_edit
            .hwnd()
            .SetWindowText(&windows_lines(&text))
    }

    /// Reloads the Office pane's matter chooser.
    fn refresh_matter_combo(&self) -> w::SysResult<()> {
        let matters = self.workspace.borrow().office_matters().unwrap_or_default();
        self.matter_combo.items().delete_all();
        let labels = matters
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>();
        self.matter_combo.items().add(&labels)
    }

    /// Runs one Court command.
    fn court_command(&self, command: CourtCommand) -> w::SysResult<()> {
        match command {
            CourtCommand::CopyRow => return self.copy_selected_row(),
            CourtCommand::SeedOffice => {
                let seeded = self.workspace.borrow_mut().seed_office();
                if let Err(error) = seeded {
                    self.court_status_label
                        .hwnd()
                        .SetWindowText(&error.to_string())?;
                    return Ok(());
                }
            }
            _ => {
                let moved = {
                    let mut workspace = self.workspace.borrow_mut();
                    match command {
                        CourtCommand::Today => workspace.docket_today(),
                        CourtCommand::PreviousWeek => workspace.docket_shift_weeks(-1),
                        CourtCommand::PreviousDay => workspace.docket_shift_days(-1),
                        CourtCommand::NextDay => workspace.docket_shift_days(1),
                        CourtCommand::NextWeek => workspace.docket_shift_weeks(1),
                        CourtCommand::CopyRow | CourtCommand::SeedOffice => Ok(()),
                    }
                };
                if let Err(error) = moved {
                    self.court_status_label
                        .hwnd()
                        .SetWindowText(&error.to_string())?;
                    return Ok(());
                }
            }
        }
        self.refresh_court()
    }

    /// Puts the selected row on the clipboard, whichever grid has the focus.
    ///
    /// Everything identifying is copyable: a court number, a client's name, a
    /// charge and a due date all leave this window without being retyped, which
    /// is the whole complaint behind the rule.
    fn copy_selected_row(&self) -> w::SysResult<()> {
        let focus = w::HWND::GetFocus();
        let from_deadlines = focus.as_ref() == Some(self.deadline_grid.hwnd());
        let grid = if from_deadlines {
            &self.deadline_grid
        } else {
            &self.docket_grid
        };
        let Some(item) = grid.items().focused() else {
            return self
                .court_status_label
                .hwnd()
                .SetWindowText("Select a row first; there is nothing to copy.");
        };
        let index = item.index() as usize;
        let workspace = self.workspace.borrow();
        let text = if from_deadlines {
            workspace
                .deadline_rows()
                .get(index)
                .map(|row| row.copy_text.clone())
        } else {
            workspace
                .docket_rows()
                .get(index)
                .map(|row| row.copy_text.clone())
        };
        drop(workspace);
        let Some(text) = text else {
            return Ok(());
        };
        self.copy_text(&text)?;
        self.court_status_label
            .hwnd()
            .SetWindowText(&format!("Copied: {text}"))
    }

    /// Puts one line of text on the clipboard as Unicode.
    fn copy_text(&self, text: &str) -> w::SysResult<()> {
        let mut bytes = Vec::with_capacity((text.len() + 1) * 2);
        for unit in text.encode_utf16().chain(std::iter::once(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let clipboard = self.wnd.hwnd().OpenClipboard()?;
        clipboard.EmptyClipboard()?;
        clipboard.SetClipboardData(co::CF::UNICODETEXT, &bytes)
    }

    fn events(&self) {
        let me = self.clone();
        self.wnd.on().wm_create(move |_| {
            me.fit_to_work_area()?;
            me.update_title()?;
            me.show_view(WorkspaceView::Overview)?;
            // Court opens first: the first question of a defender's day is
            // where they have to be, not what is in one case file.
            me.show_pane(Pane::Court)?;
            Ok(0)
        });

        let me = self.clone();
        self.wnd.on().wm_get_min_max_info(move |info| {
            if let Some((frame_width, frame_height)) = me.frame_margins() {
                info.info.ptMinTrackSize = w::POINT::with(
                    gui::dpi_x(MIN_WIDTH) + frame_width,
                    gui::dpi_y(MIN_HEIGHT) + frame_height,
                );
            }
            Ok(())
        });

        let me = self.clone();
        self.wnd.on().wm_paint(move || {
            me.paint_chrome()?;
            Ok(())
        });

        let me = self.clone();
        self.wnd.on().wm_size(move |_| {
            // The layout has already moved the controls; repaint so no shadow
            // or rule is left behind at an old position.
            me.wnd.hwnd().InvalidateRect(None, true)?;
            Ok(())
        });

        let me = self.clone();
        self.open_button.on().bn_clicked(move || {
            me.open_database()?;
            Ok(())
        });

        let me = self.clone();
        self.new_case_button.on().bn_clicked(move || {
            me.open_new_case()?;
            Ok(())
        });

        let me = self.clone();
        self.case_combo.on().cbn_sel_change(move || {
            if let Some(index) = me.case_combo.items().selected_index() {
                let result = me
                    .workspace
                    .borrow_mut()
                    .select_case(index as usize)
                    .and_then(|()| me.workspace.borrow().render(WorkspaceView::Overview));
                me.update_title()?;
                me.present(result, WorkspaceView::Overview.label())?;
            }
            Ok(())
        });

        for (view, control) in &self.view_buttons {
            let me = self.clone();
            let view = *view;
            control.on().bn_clicked(move || {
                me.show_view(view)?;
                Ok(())
            });
        }

        // The same views again, reached by their `Ctrl` accelerators. A view
        // whose caption has no free Alt letter has nothing else.
        for (index, (_, view)) in VIEW_BUTTONS.iter().enumerate() {
            let me = self.clone();
            let view = *view;
            let command = ACCEL_FIRST_VIEW + u16::try_from(index).unwrap_or(0);
            self.wnd.on().wm_command_acc_menu(command, move || {
                me.show_view(view)?;
                Ok(())
            });
        }

        // The pane switch, by button and by its `Ctrl+Shift` chord.
        for (index, (pane, control)) in self.pane_buttons.iter().enumerate() {
            let me = self.clone();
            let pane = *pane;
            control.on().bn_clicked(move || {
                me.show_pane(pane)?;
                Ok(())
            });

            let me = self.clone();
            let command = ACCEL_FIRST_PANE + u16::try_from(index).unwrap_or(0);
            self.wnd.on().wm_command_acc_menu(command, move || {
                me.show_pane(pane)?;
                Ok(())
            });
        }

        // The Court commands, likewise. None of them has an Alt letter left to
        // claim, so the chord is the only keyboard path to each.
        for (index, (command, control)) in self.court_buttons.iter().enumerate() {
            let me = self.clone();
            let command = *command;
            control.on().bn_clicked(move || {
                me.court_command(command)?;
                Ok(())
            });

            let me = self.clone();
            let accelerator = ACCEL_FIRST_COURT + u16::try_from(index).unwrap_or(0);
            self.wnd.on().wm_command_acc_menu(accelerator, move || {
                me.court_command(command)?;
                Ok(())
            });
        }

        // The office entry row, by button and by chord, exactly as the Court
        // commands are wired.
        for (index, (command, control)) in self.office_buttons.iter().enumerate() {
            let me = self.clone();
            let command = *command;
            control.on().bn_clicked(move || {
                me.office_command(command)?;
                Ok(())
            });

            let me = self.clone();
            let accelerator = ACCEL_FIRST_OFFICE + u16::try_from(index).unwrap_or(0);
            self.wnd.on().wm_command_acc_menu(accelerator, move || {
                me.office_command(command)?;
                Ok(())
            });
        }

        // Selecting a setting expands it in place rather than opening anything.
        let me = self.clone();
        self.docket_grid.on().lvn_item_changed(move |_| {
            me.refresh_court_detail()?;
            Ok(())
        });

        // A typed day is read when the box loses the focus, so a person can
        // type it and tab away rather than hunting for a button.
        let me = self.clone();
        self.court_date_edit.on().en_kill_focus(move || {
            let typed = me.court_date_edit.hwnd().GetWindowText()?;
            let moved = me.workspace.borrow_mut().set_docket_date(&typed);
            match moved {
                Ok(()) => me.refresh_court()?,
                Err(error) => me
                    .court_status_label
                    .hwnd()
                    .SetWindowText(&error.to_string())?,
            }
            Ok(())
        });

        // A matter is how the Office pane reaches discovery: choosing one
        // selects its evidence case and shows that case's overview.
        let me = self.clone();
        self.matter_combo.on().cbn_sel_change(move || {
            let Some(index) = me.matter_combo.items().selected_index() else {
                return Ok(());
            };
            let chosen = me
                .workspace
                .borrow()
                .office_matters()
                .ok()
                .and_then(|matters| matters.get(index as usize).map(|(id, _)| id.clone()));
            let Some(matter_id) = chosen else {
                return Ok(());
            };
            let selected = me.workspace.borrow_mut().select_matter(&matter_id);
            match selected {
                Ok(_) => {
                    me.refresh_case_combo()?;
                    me.update_title()?;
                    let rendered = me.workspace.borrow().render(WorkspaceView::Overview);
                    me.present(rendered, WorkspaceView::Overview.label())?;
                }
                Err(error) => me.present(Err(error), "")?,
            }
            Ok(())
        });

        let me = self.clone();
        self.enrichment_button.on().bn_clicked(move || {
            me.show_enrichment()?;
            Ok(())
        });

        let me = self.clone();
        self.wnd
            .on()
            .wm_command_acc_menu(ACCEL_ENRICHMENT, move || {
                me.show_enrichment()?;
                Ok(())
            });

        let me = self.clone();
        self.intake_button.on().bn_clicked(move || {
            me.intake_evidence()?;
            Ok(())
        });
        let me = self.clone();
        self.queue_button.on().bn_clicked(move || {
            me.show_processing_queue()?;
            Ok(())
        });

        let me = self.clone();
        self.search_button.on().bn_clicked(move || {
            me.run_search()?;
            Ok(())
        });

        // Enter runs Find when typed in the search box. The message loop's
        // `IsDialogMessage` turns an Enter no control claims into `IDOK`
        // (the multiline payload box claims its own through `ES_WANTRETURN`),
        // so the search box is told apart from every other edit by focus.
        let me = self.clone();
        self.wnd
            .on()
            .wm_command(co::DLGID::OK, co::CMD::Menu, move || {
                if w::HWND::GetFocus().as_ref() == Some(me.search_edit.hwnd()) {
                    me.run_search()?;
                }
                Ok(())
            });

        let me = self.clone();
        self.suggest_button.on().bn_clicked(move || {
            let result = me.workspace.borrow_mut().suggest_all();
            me.present(result, "Deterministic collation complete")?;
            Ok(())
        });

        for (button, audience, status) in [
            (
                self.safe_export_button.clone(),
                ExportAudience::Disclosable,
                "Disclosable export preview",
            ),
            (
                self.work_export_button.clone(),
                ExportAudience::WorkFile,
                "Privileged work-file export preview",
            ),
        ] {
            let me = self.clone();
            button.on().bn_clicked(move || {
                let result = me.workspace.borrow().export_preview(audience);
                me.present(result, status)?;
                Ok(())
            });
        }

        let me = self.clone();
        self.persist_export_button.on().bn_clicked(move || {
            me.save_export()?;
            Ok(())
        });

        for (state, button) in &self.review_buttons {
            let me = self.clone();
            let state = *state;
            button.on().bn_clicked(move || {
                me.apply_review(state)?;
                Ok(())
            });
        }

        let me = self.clone();
        self.template_button.on().bn_clicked(move || {
            let kind = me.selected_author_kind()?;
            me.payload_edit.set_text(&windows_lines(kind.template()))?;
            me.set_status(&format!("Loaded {} JSON template.", kind.label()))?;
            Ok(())
        });

        let me = self.clone();
        self.author_combo.on().cbn_sel_change(move || {
            let kind = me.selected_author_kind()?;
            me.payload_edit.set_text(&windows_lines(kind.template()))?;
            Ok(())
        });

        let me = self.clone();
        self.author_button.on().bn_clicked(move || {
            let kind = me.selected_author_kind()?;
            let payload = me.payload_edit.text()?;
            let result = me.workspace.borrow_mut().author_json(kind, &payload);
            me.present(result, &format!("Saved {}.", kind.label()))?;
            Ok(())
        });

        let me = self.clone();
        self.import_button.on().bn_clicked(move || {
            let payload = me.payload_edit.text()?;
            let result = me.workspace.borrow_mut().import_json(&payload);
            if result.is_ok() {
                me.refresh_case_combo()?;
            }
            me.present(result, "Normalized intake imported")?;
            Ok(())
        });
    }

    fn intake_evidence(&self) -> w::SysResult<()> {
        let Some(modality) = self.choose_modality()? else {
            return Ok(());
        };
        let productions = match self.workspace.borrow().productions() {
            Ok(productions) => productions,
            Err(error) => return self.present(Err(error), "Intake evidence"),
        };
        let Some(selection) = self.show_intake_dialog(modality, &productions)? else {
            return Ok(());
        };
        let result = self.queue_selection(selection, &productions);
        if result.is_ok()
            && let Some(coordinator) = self.coordinator.borrow().as_ref()
        {
            coordinator.wake();
        }
        self.present(result, "Evidence queued for local processing")
    }

    fn choose_modality(&self) -> w::SysResult<Option<IntakeModality>> {
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: "Intake Evidence",
            size: gui::dpi(420, 190),
            ..Default::default()
        });
        let _heading = label(
            &modal,
            "Choose the original evidence type. Each type has its own processing profile.",
            22,
            20,
            375,
            ANCHOR,
        );
        let document = button(&modal, "&Document", 22, 64, 116, ANCHOR);
        let audio = button(&modal, "&Audio", 152, 64, 116, ANCHOR);
        let video = button(&modal, "&Video", 282, 64, 116, ANCHOR);
        let cancel = button(&modal, "Cancel", 282, 126, 116, ANCHOR);
        let selected = Rc::new(RefCell::new(None));
        for (control, modality) in [
            (document, IntakeModality::Document),
            (audio, IntakeModality::Audio),
            (video, IntakeModality::Video),
        ] {
            let selected = selected.clone();
            let modal = modal.clone();
            control.on().bn_clicked(move || {
                *selected.borrow_mut() = Some(modality);
                modal.close();
                Ok(())
            });
        }
        let modal_close = modal.clone();
        cancel.on().bn_clicked(move || {
            modal_close.close();
            Ok(())
        });
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;
        Ok(*selected.borrow())
    }

    fn show_intake_dialog(
        &self,
        modality: IntakeModality,
        productions: &[crate::OpenedProduction],
    ) -> w::SysResult<Option<IntakeSelection>> {
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: modality.title(),
            size: gui::dpi(590, 360),
            ..Default::default()
        });
        let _files_label = label(&modal, "Originals", 22, 22, 90, ANCHOR);
        let choose_files = button(&modal, "Choose &Files…", 120, 16, 140, ANCHOR);
        let file_status = label(&modal, "No files selected", 274, 22, 290, ANCHOR);

        let _production_label = label(&modal, "Production", 22, 72, 90, ANCHOR);
        let mut production_labels = productions
            .iter()
            .map(|production| production.label.clone())
            .collect::<Vec<_>>();
        production_labels.push("<New production>".to_owned());
        let production_refs = production_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let production_combo = gui::ComboBox::new(
            &modal,
            gui::ComboBoxOpts {
                position: gui::dpi(120, 68),
                width: gui::dpi_x(220),
                items: &production_refs,
                selected_item: Some(if productions.is_empty() {
                    0
                } else {
                    u32::try_from(productions.len() - 1).unwrap_or(0)
                }),
                ..Default::default()
            },
        );
        let new_production = gui::Edit::new(
            &modal,
            gui::EditOpts {
                text: "",
                position: gui::dpi(352, 68),
                width: gui::dpi_x(212),
                ..Default::default()
            },
        );
        let _new_hint = label(
            &modal,
            "New production label (when selected)",
            352,
            94,
            212,
            ANCHOR,
        );

        let _temporal_label = label(&modal, "Temporal relation", 22, 130, 96, ANCHOR);
        let temporal_combo = gui::ComboBox::new(
            &modal,
            gui::ComboBoxOpts {
                position: gui::dpi(120, 126),
                width: gui::dpi_x(220),
                items: &["Unknown", "Contemporaneous", "After event", "Mixed"],
                selected_item: Some(0),
                ..Default::default()
            },
        );

        let (option_one_text, option_two_text) = match modality {
            IntakeModality::Document => (
                "Lege search-profile OCR through the Evidence TensorRT broker",
                "",
            ),
            IntakeModality::Audio => (
                "Telephone-band working copy (off by default)",
                "Near/far level observations (off by default)",
            ),
            IntakeModality::Video => (
                "Overnight analysis: embeddings, detections and captions",
                "Tier 1 always includes scenes, clock OCR and soundtrack transcription",
            ),
        };
        let option_one = gui::CheckBox::new(
            &modal,
            gui::CheckBoxOpts {
                text: option_one_text,
                position: gui::dpi(22, 184),
                check_state: if modality == IntakeModality::Document {
                    co::BST::CHECKED
                } else {
                    co::BST::UNCHECKED
                },
                ..Default::default()
            },
        );
        if modality == IntakeModality::Document {
            option_one.hwnd().EnableWindow(false);
        }
        let option_two = gui::CheckBox::new(
            &modal,
            gui::CheckBoxOpts {
                text: option_two_text,
                position: gui::dpi(22, 220),
                ..Default::default()
            },
        );
        if modality != IntakeModality::Audio {
            option_two.hwnd().EnableWindow(false);
        }
        let queue = button(&modal, "&Queue", 318, 300, 116, ANCHOR);
        let cancel = button(&modal, "Cancel", 448, 300, 116, ANCHOR);

        let files = Rc::new(RefCell::new(Vec::new()));
        let files_for_picker = files.clone();
        let file_status_for_picker = file_status.clone();
        let parent = modal.clone();
        choose_files.on().bn_clicked(move || {
            if let Ok(chosen) = select_files(parent.hwnd(), modality)
                && !chosen.is_empty()
            {
                let summary = if chosen.len() == 1 {
                    chosen[0].display().to_string()
                } else {
                    format!("{} files selected", chosen.len())
                };
                *files_for_picker.borrow_mut() = chosen;
                file_status_for_picker.hwnd().SetWindowText(&summary)?;
            }
            Ok(())
        });

        let result = Rc::new(RefCell::new(None));
        let result_for_queue = result.clone();
        let files_for_queue = files.clone();
        let modal_close = modal.clone();
        queue.on().bn_clicked(move || {
            let temporal = match temporal_combo.items().selected_index().unwrap_or(0) {
                1 => TemporalRelationArg::Contemporaneous,
                2 => TemporalRelationArg::AfterEvent,
                3 => TemporalRelationArg::Mixed,
                _ => TemporalRelationArg::Unknown,
            };
            *result_for_queue.borrow_mut() = Some(IntakeSelection {
                modality,
                files: files_for_queue.borrow().clone(),
                production_index: production_combo.items().selected_index().unwrap_or(0) as usize,
                new_production: new_production.text()?.trim().to_owned(),
                temporal,
                phone_band: modality == IntakeModality::Audio
                    && option_one.state() == co::BST::CHECKED,
                level_split: modality == IntakeModality::Audio
                    && option_two.state() == co::BST::CHECKED,
                overnight: modality == IntakeModality::Video
                    && option_one.state() == co::BST::CHECKED,
            });
            modal_close.close();
            Ok(())
        });
        let modal_close = modal.clone();
        cancel.on().bn_clicked(move || {
            modal_close.close();
            Ok(())
        });
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;
        Ok(result.borrow().clone())
    }

    fn queue_selection(
        &self,
        selection: IntakeSelection,
        productions: &[crate::OpenedProduction],
    ) -> GuiResult<String> {
        if selection.files.is_empty() {
            return Err(GuiError::new("Choose at least one original file."));
        }
        let production_id = if let Some(production) = productions.get(selection.production_index) {
            production.id.clone()
        } else {
            if selection.new_production.trim().is_empty() {
                return Err(GuiError::new("Enter a label for the new production."));
            }
            self.workspace
                .borrow_mut()
                .open_production(&selection.new_production)?
                .id
        };
        let (case_id, _) = self
            .workspace
            .borrow()
            .active_case()
            .cloned()
            .ok_or_else(|| GuiError::new("Select a case before intake."))?;
        let database = self
            .workspace
            .borrow()
            .database()
            .map(Path::to_path_buf)
            .ok_or_else(|| GuiError::new("Persistent intake requires a case database on disk."))?;
        let models = Self::broker_models("evidence-trt")?;
        let ocr = matches!(
            selection.modality,
            IntakeModality::Document | IntakeModality::Video
        )
        .then(|| models.required(Operation::PageOcr, "turbo-ocr"))
        .transpose()?;
        let whisper = matches!(
            selection.modality,
            IntakeModality::Audio | IntakeModality::Video
        )
        .then(|| models.required(Operation::TranscribeAudio, "whisper-large-v3"))
        .transpose()?;
        let (embedding, detector, caption) = if selection.overnight {
            (
                Some(models.required(Operation::EmbedImage, "siglip2-base-patch16-384")?),
                Some(models.required(Operation::DetectImage, "yolo-v8n")?),
                Some(models.required(Operation::CaptionImage, "qwen2-vl-2b")?),
            )
        } else {
            (None, None, None)
        };
        let executable_dir = std::env::current_exe()?
            .parent()
            .ok_or_else(|| GuiError::new("Application executable has no directory."))?
            .to_path_buf();
        let sidecar = artifact_sidecar(&database);
        let mut queued = 0_usize;
        for original_path in selection.files {
            let (original_sha256, original_byte_length) = hash_file(&original_path)?;
            let job_id = Uuid::now_v7().to_string();
            let source_id = format!("source-{}", Uuid::now_v7());
            let artifacts_dir = sidecar.join(&job_id).join("attempt-0001");
            let logical_name = original_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("original evidence")
                .to_owned();
            let profile = match selection.modality {
                IntakeModality::Document => AdapterProfile::Document {
                    lege_ocr: companion(&executable_dir, "lege-ocr"),
                    broker_bridge: companion(
                        &executable_dir,
                        "evidence-trt-lege-ocr-bridge",
                    ),
                    broker_endpoint: "evidence-trt".to_owned(),
                    ocr: ocr
                        .clone()
                        .ok_or_else(|| GuiError::new("Document intake requires broker OCR."))?,
                },
                IntakeModality::Audio => AdapterProfile::Audio {
                    broker_endpoint: "evidence-trt".to_owned(),
                    whisper: whisper
                        .clone()
                        .ok_or_else(|| GuiError::new("Audio intake requires broker Whisper."))?,
                    language: "en".to_owned(),
                    phone_band: selection.phone_band,
                    level_split: selection.level_split,
                    gap_ms: 2_000,
                },
                IntakeModality::Video => AdapterProfile::Video {
                    broker_endpoint: "evidence-trt".to_owned(),
                    tier: if selection.overnight {
                        VideoTier::Overnight
                    } else {
                        VideoTier::Tier1
                    },
                    ocr: ocr
                        .clone()
                        .ok_or_else(|| GuiError::new("Video intake requires broker OCR."))?,
                    whisper: whisper
                        .clone()
                        .ok_or_else(|| GuiError::new("Video intake requires broker Whisper."))?,
                    embedding: embedding.clone(),
                    detector: detector.clone(),
                    caption: caption.clone(),
                    language: "en".to_owned(),
                    threshold: 0.30,
                    gap_ms: 2_000,
                    sample_gap_ms: 5_000,
                    sample_dedup_ms: 250,
                    detector_confidence: 0.25,
                    caption_prompt:
                        "Describe only directly visible scene content; do not identify people, infer intent, enhance, reconstruct, or assess authenticity."
                            .to_owned(),
                },
            };
            let request = AdapterJobRequest {
                schema_version: ADAPTER_PROTOCOL_VERSION,
                job_id,
                case_id: case_id.0.clone(),
                production_id: production_id.clone(),
                source_id,
                original_path,
                original_sha256,
                original_byte_length,
                logical_name,
                temporal_relation: selection.temporal,
                artifacts_dir,
                profile,
            };
            request
                .validate()
                .map_err(|error| GuiError::new(error.to_string()))?;
            self.workspace.borrow_mut().queue_intake(&request)?;
            queued += 1;
        }
        Ok(format!(
            "Queued {queued} original(s). Processing is local, persistent, and one GPU-heavy job at a time."
        ))
    }

    fn broker_models(endpoint: &str) -> GuiResult<BrokerModels> {
        let mut client = Client::connect(endpoint)
            .map_err(|error| GuiError::new(format!("TensorRT broker is unavailable: {error}")))?;
        let ResultBody::Models { models } = client
            .request(Request::Models, &[])
            .map_err(|error| GuiError::new(format!("Could not inspect broker models: {error}")))?
        else {
            return Err(GuiError::new("Broker returned an incompatible model list."));
        };
        let mut by_operation: HashMap<Operation, Vec<ModelRef>> = HashMap::new();
        for model in models {
            for operation in model.operations {
                by_operation.entry(operation).or_default().push(ModelRef {
                    id: model.id.clone(),
                    revision: model.revision.clone(),
                });
            }
        }
        Ok(BrokerModels { by_operation })
    }

    /// Runs the search the mode selector names — from the Find button, or
    /// from Enter in the search box.
    fn run_search(&self) -> w::SysResult<()> {
        let query = self.search_edit.text()?;
        let mode = self
            .search_mode
            .items()
            .selected_text()?
            .unwrap_or_else(|| "Text".to_owned());
        let result = if mode == "Video Frames" {
            self.search_video_frames(query.trim())
        } else {
            self.workspace.borrow().search(query.trim(), 100)
        };
        self.present(
            result,
            if mode == "Video Frames" {
                "Frame finder results"
            } else {
                "Search results"
            },
        )
    }

    fn search_video_frames(&self, query: &str) -> GuiResult<String> {
        if query.trim().is_empty() {
            return Err(GuiError::new("Enter a frame-search description."));
        }
        let models = self.workspace.borrow().keyframe_models()?;
        let model = models.first().ok_or_else(|| {
            GuiError::new(
                "This case has no stored video-frame index. Run overnight video analysis first.",
            )
        })?;
        let broker_models = Self::broker_models("evidence-trt")?;
        let installed = broker_models.required(Operation::EmbedText, model)?;
        if installed.id != *model {
            return Err(GuiError::new(format!(
                "Stored frames require model `{model}`, which the broker does not expose for text embedding."
            )));
        }
        let mut client = Client::connect("evidence-trt")
            .map_err(|error| GuiError::new(format!("TensorRT broker is unavailable: {error}")))?;
        let ResultBody::Embedding { vector } = client
            .request(
                Request::Infer {
                    model: installed.id.clone(),
                    revision: installed.revision,
                    operation: Operation::EmbedText,
                    input: InputMetadata {
                        media_type: Some("text/plain; charset=utf-8".to_owned()),
                        ..InputMetadata::default()
                    },
                },
                query.as_bytes(),
            )
            .map_err(|error| GuiError::new(format!("Frame query failed: {error}")))?
        else {
            return Err(GuiError::new(
                "Broker returned a non-embedding frame query result.",
            ));
        };
        let hits = self.workspace.borrow().search_frames(model, &vector, 100)?;
        let rendered = hits
            .into_iter()
            .map(|hit| {
                let path = self
                    .workspace
                    .borrow()
                    .source_location(&hit.source_id)
                    .map_or_else(
                        |_| "<retained still unavailable>".to_owned(),
                        |location| location.path.display().to_string(),
                    );
                serde_json::json!({
                    "original": hit.source,
                    "original_sha256": hit.sha256,
                    "time_range": hit.locator,
                    "derived_still": path,
                    "still_sha256": hit.still_sha256,
                    "review_state": hit.review_state,
                    "machine_generated": hit.machine_generated,
                    "bears_on": hit.bears_on,
                })
            })
            .collect::<Vec<_>>();
        Ok(readable_text(&serde_json::Value::Array(rendered)))
    }

    /// Opens the keyboard enrichment sweep over one of the case's originals.
    fn show_enrichment(&self) -> w::SysResult<()> {
        let actor = self.actor_edit.text()?;
        SweepWindow::show(self, actor.trim())
    }

    fn show_processing_queue(&self) -> w::SysResult<()> {
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: "Processing Queue",
            size: gui::dpi(760, 520),
            ..Default::default()
        });
        let _job_label = label(&modal, "Job", 20, 22, 44, ANCHOR);
        let job_combo = gui::ComboBox::new(
            &modal,
            gui::ComboBoxOpts {
                position: gui::dpi(68, 18),
                width: gui::dpi_x(530),
                items: &[],
                ..Default::default()
            },
        );
        let refresh = button(&modal, "&Refresh", 612, 16, 126, ANCHOR);
        let output = gui::Edit::new(
            &modal,
            gui::EditOpts {
                text: "",
                position: gui::dpi(20, 62),
                width: gui::dpi_x(718),
                height: gui::dpi_y(386),
                control_style: co::ES::MULTILINE
                    | co::ES::AUTOVSCROLL
                    | co::ES::AUTOHSCROLL
                    | co::ES::READONLY,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::BORDER
                    | co::WS::VSCROLL
                    | co::WS::HSCROLL
                    | co::WS::TABSTOP,
                ..Default::default()
            },
        );
        let open_original = button(&modal, "Open &Original", 20, 468, 146, ANCHOR);
        let open_artifacts = button(&modal, "Open &Artifacts", 178, 468, 146, ANCHOR);
        let retry = button(&modal, "Ret&ry", 336, 468, 116, ANCHOR);
        let close = button(&modal, "Close", 622, 468, 116, ANCHOR);
        let jobs = Rc::new(RefCell::new(Vec::new()));

        let update = {
            let workspace = self.workspace.clone();
            let combo = job_combo.clone();
            let output = output.clone();
            let jobs = jobs.clone();
            move || refresh_queue_view(&workspace, &combo, &output, &jobs)
        };
        update()?;
        let update = Rc::new(update);
        let update_click = update.clone();
        refresh.on().bn_clicked(move || {
            update_click()?;
            Ok(())
        });
        let update_timer = update.clone();
        modal.on().wm_timer(1, move || {
            update_timer()?;
            Ok(())
        });
        let modal_timer = modal.clone();
        modal.on().wm_create(move |_| {
            modal_timer.hwnd().SetTimer(1, 1_000, None)?;
            Ok(0)
        });

        let jobs_for_open = jobs.clone();
        let combo_for_open = job_combo.clone();
        let owner = modal.clone();
        let output_for_open = output.clone();
        open_original.on().bn_clicked(move || {
            if let Some(job) = selected_queue_job(&jobs_for_open, &combo_for_open) {
                match hash_file(&job.original_path) {
                    Ok((hash, length))
                        if hash.eq_ignore_ascii_case(&job.original_sha256)
                            && length == job.original_byte_length => {}
                    Ok(_) => {
                        output_for_open.set_text(
                            "ERROR\r\n\r\nThe original no longer matches its queued SHA-256 and length.",
                        )?;
                        return Ok(());
                    }
                    Err(error) => {
                        output_for_open.set_text(&format!("ERROR\r\n\r\n{error}"))?;
                        return Ok(());
                    }
                }
                let path = job.original_path.to_string_lossy();
                owner
                    .hwnd()
                    .ShellExecute("open", &path, None, None, co::SW::SHOWNORMAL)?;
            }
            Ok(())
        });

        let workspace_for_artifacts = self.workspace.clone();
        let jobs_for_artifacts = jobs.clone();
        let combo_for_artifacts = job_combo.clone();
        let owner = modal.clone();
        open_artifacts.on().bn_clicked(move || {
            if let Some(job) = selected_queue_job(&jobs_for_artifacts, &combo_for_artifacts) {
                let artifacts = workspace_for_artifacts
                    .borrow()
                    .intake_artifacts(&job.id)
                    .unwrap_or_default();
                let path = artifacts
                    .first()
                    .and_then(|artifact| artifact.path.parent().map(Path::to_path_buf))
                    .unwrap_or(job.artifact_dir);
                let path = path.to_string_lossy();
                owner
                    .hwnd()
                    .ShellExecute("open", &path, None, None, co::SW::SHOWNORMAL)?;
            }
            Ok(())
        });

        let workspace_for_retry = self.workspace.clone();
        let coordinator = self.coordinator.clone();
        let jobs_for_retry = jobs.clone();
        let combo_for_retry = job_combo.clone();
        let output_for_retry = output.clone();
        let update_retry = update.clone();
        retry.on().bn_clicked(move || {
            let Some(job) = selected_queue_job(&jobs_for_retry, &combo_for_retry) else {
                return Ok(());
            };
            if !matches!(
                job.state,
                IntakeJobState::Failed | IntakeJobState::Interrupted
            ) {
                output_for_retry
                    .set_text("Retry is available only for failed or interrupted jobs.")?;
                return Ok(());
            }
            let result = (|| -> GuiResult<()> {
                let mut request: AdapterJobRequest = serde_json::from_str(&job.request_json)?;
                let root = job
                    .artifact_dir
                    .parent()
                    .ok_or_else(|| GuiError::new("Attempt directory has no job root."))?;
                request.artifacts_dir = root.join(format!("attempt-{:04}", job.attempt + 1));
                workspace_for_retry
                    .borrow_mut()
                    .retry_intake(&job.id, &request)?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    if let Some(coordinator) = coordinator.borrow().as_ref() {
                        coordinator.wake();
                    }
                    update_retry()?;
                }
                Err(error) => output_for_retry.set_text(&format!("ERROR\r\n\r\n{error}"))?,
            }
            Ok(())
        });
        let modal_close = modal.clone();
        close.on().bn_clicked(move || {
            modal_close.close();
            Ok(())
        });
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)
    }

    /// Shrinks the window onto the monitor it opened on.
    ///
    /// The layout is written in logical units and `gui::dpi` scales it by the
    /// system DPI, so the window grows with the user's display scaling: at 200%
    /// it wants roughly 2216x1466 device pixels, which does not fit a laptop
    /// panel. Windows will happily create a window larger than the screen, and
    /// the command row along the bottom would then sit past the edge with no
    /// way to reach it. Clamping to the work area keeps every command on
    /// screen; the layout itself is unchanged, since the resize behaviours
    /// already handle a smaller window.
    fn fit_to_work_area(&self) -> w::SysResult<()> {
        fit_window_to_work_area(self.wnd.hwnd(), MIN_WIDTH, MIN_HEIGHT)
    }

    /// Device pixels the frame adds around the client area, or `None` before
    /// the window has one.
    fn frame_margins(&self) -> Option<(i32, i32)> {
        frame_margins(self.wnd.hwnd())
    }
}

/// Shrinks one window onto the monitor it opened on.
///
/// The layout is written in logical units and `gui::dpi` scales it by the
/// system DPI, so a window grows with the user's display scaling: at 200% the
/// workspace wants roughly 2216x1466 device pixels, which does not fit a
/// laptop panel. Windows will happily create a window larger than the screen,
/// and the commands along the bottom would then sit past the edge with no way
/// to reach them.
fn fit_window_to_work_area(hwnd: &w::HWND, min_width: i32, min_height: i32) -> w::SysResult<()> {
    let work = hwnd
        .MonitorFromWindow(co::MONITOR::DEFAULTTONEAREST)
        .GetMonitorInfo()?
        .rcWork;
    let window = hwnd.GetWindowRect()?;
    let available = (work.right - work.left, work.bottom - work.top);
    let wanted = (window.right - window.left, window.bottom - window.top);
    if wanted.0 <= available.0 && wanted.1 <= available.1 {
        return Ok(());
    }

    // Never clamp below the minimum: on a screen too small for the layout even
    // at its floor, a window that overflows can still be moved to reach the
    // rest of it, whereas one shrunk past the floor has commands that overlap
    // and cannot be separated again.
    let floor = frame_margins(hwnd).unwrap_or((0, 0));
    let size = (
        wanted
            .0
            .min(available.0)
            .max(gui::dpi_x(min_width) + floor.0),
        wanted
            .1
            .min(available.1)
            .max(gui::dpi_y(min_height) + floor.1),
    );
    hwnd.SetWindowPos(
        w::HwndPlace::None,
        w::POINT::with(
            work.left + (available.0 - size.0) / 2,
            work.top + (available.1 - size.1) / 2,
        ),
        w::SIZE::with(size.0, size.1),
        co::SWP::NOZORDER | co::SWP::NOACTIVATE,
    )
}

/// Device pixels the frame adds around the client area, or `None` before the
/// window has one — `WM_GETMINMAXINFO` arrives during creation, when the
/// client area can still be empty.
fn frame_margins(hwnd: &w::HWND) -> Option<(i32, i32)> {
    let window = hwnd.GetWindowRect().ok()?;
    let client = hwnd.GetClientRect().ok()?;
    (client.right > 0 && client.bottom > 0).then(|| {
        (
            (window.right - window.left) - client.right,
            (window.bottom - window.top) - client.bottom,
        )
    })
}

impl MainWindow {
    /// Paints the window's own chrome: a drop shadow under every button, and
    /// the etched rules that separate the command groups.
    ///
    /// Button positions are read back from the controls rather than recomputed
    /// from the layout constants, so the chrome stays correct after a resize.
    fn paint_chrome(&self) -> w::SysResult<()> {
        let hwnd = self.wnd.hwnd();
        let hdc = hwnd.BeginPaint()?;
        let client = hwnd.GetClientRect()?;
        let unit = gui::dpi_x(1).max(1);

        // Only what is on screen. A hidden control still has a rectangle, and
        // painting its shadow would leave the Office pane's chrome showing
        // under the Court pane with no button beneath it.
        let buttons = self
            .buttons()
            .into_iter()
            .filter(|control| control.hwnd().IsWindowVisible())
            .map(|control| hwnd.ScreenToClientRc(control.hwnd().GetWindowRect()?))
            .collect::<w::SysResult<Vec<_>>>()?;
        for (steps, colour) in SHADOW_BANDS {
            let brush = w::HBRUSH::CreateSolidBrush(rgb(colour))?;
            let offset = steps * unit;
            for rect in &buttons {
                hdc.FillRect(
                    w::RECT {
                        left: rect.left + offset,
                        top: rect.top + offset,
                        right: rect.right + offset,
                        bottom: rect.bottom + offset,
                    },
                    &brush,
                )?;
            }
        }

        let shadow = w::HBRUSH::CreateSolidBrush(rgb(RULE_SHADOW))?;
        let highlight = w::HBRUSH::CreateSolidBrush(rgb(RULE_HIGHLIGHT))?;
        let rule = |left: i32, top: i32, right: i32, bottom: i32| -> w::SysResult<()> {
            let vertical = right - left <= unit;
            let (dx, dy) = if vertical { (unit, 0) } else { (0, unit) };
            hdc.FillRect(
                w::RECT {
                    left,
                    top,
                    right,
                    bottom,
                },
                &shadow,
            )?;
            hdc.FillRect(
                w::RECT {
                    left: left + dx,
                    top: top + dy,
                    right: right + dx,
                    bottom: bottom + dy,
                },
                &highlight,
            )
        };

        // The rail rules and the divider belong to the Office pane, which is
        // the only pane that has a rail. Painting them under the Court pane
        // would draw a groove down a window with nothing on either side of it.
        if self.output_edit.hwnd().IsWindowVisible() {
            for y in RAIL_RULES {
                rule(
                    gui::dpi_x(RAIL_X),
                    gui::dpi_y(y),
                    gui::dpi_x(RAIL_X + RAIL_WIDTH),
                    gui::dpi_y(y) + unit,
                )?;
            }
            rule(
                gui::dpi_x(DIVIDER_X),
                gui::dpi_y(DIVIDER_TOP),
                gui::dpi_x(DIVIDER_X) + unit,
                client.bottom - gui::dpi_y(14),
            )?;

            // The review panel slides with the window, so anchor its rule to
            // the panel's first field rather than to a fixed coordinate.
            let field = hwnd.ScreenToClientRc(self.actor_label.hwnd().GetWindowRect()?)?;
            let y = field.top - gui::dpi_y(12);
            rule(
                gui::dpi_x(PANE_X),
                y,
                client.right - gui::dpi_x(20),
                y + unit,
            )?;
        } else {
            // The Court pane's own rule: under the day's navigation, above the
            // docket, which is the only grouping that pane needs.
            let y = gui::dpi_y(COURT_GRID_TOP - 12);
            rule(gui::dpi_x(20), y, client.right - gui::dpi_x(20), y + unit)?;
        }
        Ok(())
    }

    fn open_new_case(&self) -> w::SysResult<()> {
        let Some(payload) = self.prompt_new_case()? else {
            return Ok(());
        };
        let result = self.workspace.borrow_mut().open_case_json(&payload);
        if result.is_ok() {
            self.refresh_case_combo()?;
        }
        self.present(result, "Case opened — it is now the selected case")
    }

    /// Asks for the new case in the fields a paralegal actually has — a name,
    /// a docket reference, a jurisdiction — instead of a JSON payload. The
    /// dialog returns the typed proposal as JSON for the platform-neutral
    /// [`Workspace::open_case_json`].
    fn prompt_new_case(&self) -> w::SysResult<Option<String>> {
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: "New Case",
            size: gui::dpi(520, 260),
            ..Default::default()
        });
        let _name_label = label(&modal, "Case name", 20, 26, 120, ANCHOR);
        let name_edit = edit(&modal, 150, 22, 346, ANCHOR);
        let _reference_label = label(&modal, "Docket / reference", 20, 64, 120, ANCHOR);
        let reference_edit = edit(&modal, 150, 60, 346, ANCHOR);
        let _jurisdiction_label = label(&modal, "Jurisdiction", 20, 102, 120, ANCHOR);
        let jurisdiction_edit = edit(&modal, 150, 98, 346, ANCHOR);
        let _hint = label(
            &modal,
            "The case opens with an initial production, ready to receive intake.",
            20,
            140,
            476,
            ANCHOR,
        );
        let open = button(&modal, "&Open Case", 260, 190, 116, ANCHOR);
        let cancel = button(&modal, "Cancel", 384, 190, 112, ANCHOR);

        let payload: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let record = {
            let payload = payload.clone();
            let name_edit = name_edit.clone();
            let reference_edit = reference_edit.clone();
            let jurisdiction_edit = jurisdiction_edit.clone();
            let modal = modal.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let name = name_edit.text()?.trim().to_owned();
                if name.is_empty() {
                    return Ok(());
                }
                let proposal = serde_json::json!({
                    "name": name,
                    "reference": optional_text(&reference_edit.text()?),
                    "jurisdiction": optional_text(&jurisdiction_edit.text()?),
                });
                *payload.borrow_mut() = Some(proposal.to_string());
                modal.close();
                Ok(())
            })
        };
        let record_click = record.clone();
        open.on().bn_clicked(move || record_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(
                record,
                vec![
                    Field::text(&name_edit),
                    Field::text(&reference_edit),
                    Field::text(&jurisdiction_edit),
                ],
            ),
        );
        let modal_cancel = modal.clone();
        cancel.on().bn_clicked(move || {
            modal_cancel.close();
            Ok(())
        });
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;
        let recorded = payload.borrow().clone();
        Ok(recorded)
    }

    /// Runs one office entry command, asking who is acting first if nobody is.
    fn office_command(&self, command: OfficeCommand) -> w::SysResult<()> {
        if self.workspace.borrow().pane() != Pane::Office {
            self.show_pane(Pane::Office)?;
        }
        if command != OfficeCommand::ActingAs && self.workspace.borrow().acting_user().is_none() {
            self.prompt_acting_user()?;
            if self.workspace.borrow().acting_user().is_none() {
                return self.present(
                    Err(GuiError::new(
                        "Nobody is acting. Choose who is entering records before writing.",
                    )),
                    "",
                );
            }
        }
        match command {
            OfficeCommand::ActingAs => self.prompt_acting_user(),
            OfficeCommand::NewClient => self.new_client(),
            OfficeCommand::NewMatter => self.new_matter(),
            OfficeCommand::NewSetting => self.new_setting(),
            OfficeCommand::NewDeadline => self.new_deadline(),
            OfficeCommand::NewNote => self.new_note(),
        }
    }

    /// Keeps the entry row saying who office writes are attributed to.
    fn refresh_acting_label(&self) -> w::SysResult<()> {
        let text = match self.workspace.borrow().acting_user() {
            Some(user) => format!("Acting as: {} ({})", user.display_name, user.role),
            None => "Acting as: nobody yet".to_owned(),
        };
        self.acting_label.hwnd().SetWindowText(&text)
    }

    /// Asks who is entering records, once per session.
    fn prompt_acting_user(&self) -> w::SysResult<()> {
        let users = self.workspace.borrow().office_users().unwrap_or_default();
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: "Acting As",
            size: gui::dpi(400, 210),
            ..Default::default()
        });
        let _who = label(&modal, "Who is entering records", 20, 26, 120, ANCHOR);
        let mut user_rows = vec!["(somebody new)".to_owned()];
        user_rows.extend(
            users
                .iter()
                .map(|(_, name, role)| format!("{name} — {role}")),
        );
        let user_refs = user_rows.iter().map(String::as_str).collect::<Vec<_>>();
        let users_combo = choice(&modal, 150, 22, 226, &user_refs, 0);
        let _name = label(&modal, "Name", 20, 64, 120, ANCHOR);
        let name_edit = edit(&modal, 150, 60, 226, ANCHOR);
        let _role = label(&modal, "Role", 20, 102, 120, ANCHOR);
        let role_combo = choice(&modal, 150, 98, 226, &USER_ROLES, 0);
        let (accept, cancel) = modal_buttons(&modal, "&Use", 150);

        // Picking an existing user fills the boxes, so Enter is immediate.
        {
            let users = users.clone();
            let users_combo = users_combo.clone();
            let name_edit = name_edit.clone();
            let role_combo = role_combo.clone();
            users_combo.clone().on().cbn_sel_change(move || {
                let Some(index) = users_combo.items().selected_index() else {
                    return Ok(());
                };
                if index == 0 {
                    return Ok(());
                }
                if let Some((_, name, role)) = users.get(index as usize - 1) {
                    name_edit.set_text(name)?;
                    let position = USER_ROLES.iter().position(|known| known == role);
                    role_combo
                        .items()
                        .select(position.and_then(|p| u32::try_from(p).ok()));
                }
                Ok(())
            });
        }

        let chosen: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));
        let record = {
            let chosen = chosen.clone();
            let name_edit = name_edit.clone();
            let role_combo = role_combo.clone();
            let modal = modal.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let name = name_edit.text()?.trim().to_owned();
                if name.is_empty() {
                    return Ok(());
                }
                let role = role_combo.items().selected_index().unwrap_or(0);
                let role = USER_ROLES[role as usize % USER_ROLES.len()].to_owned();
                *chosen.borrow_mut() = Some((name, role));
                modal.close();
                Ok(())
            })
        };
        let record_click = record.clone();
        accept.on().bn_clicked(move || record_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(
                record,
                vec![
                    Field::choice(&users_combo),
                    Field::text(&name_edit),
                    Field::choice(&role_combo),
                ],
            ),
        );
        wire_cancel(&modal, &cancel);
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;

        let chosen = chosen.borrow().clone();
        if let Some((name, role)) = chosen {
            let named = self.workspace.borrow_mut().set_acting_user(&name, &role);
            match named {
                Ok(message) => self.set_status(&message)?,
                Err(error) => self.present(Err(error), "")?,
            }
            self.refresh_acting_label()?;
        }
        Ok(())
    }

    /// The client entry form, with the possible-duplicate question before the
    /// write. f, m and a pick a sex; any language can be typed via `Other…`.
    fn new_client(&self) -> w::SysResult<()> {
        let languages = self
            .workspace
            .borrow()
            .office_languages()
            .unwrap_or_default();
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: NEW_CLIENT,
            size: gui::dpi(560, 440),
            ..Default::default()
        });
        let rows = [
            "Name",
            "Date of birth",
            "Sex",
            "Language",
            "Aliases",
            "Phone",
            "Email",
            "Address",
            "Notes",
        ];
        for (index, caption) in rows.iter().enumerate() {
            let y = 26 + i32::try_from(index).unwrap_or(0) * 38;
            label(&modal, caption, 20, y, 120, ANCHOR);
        }
        let name_edit = edit(&modal, 150, 22, 390, ANCHOR);
        let dob_edit = edit(&modal, 150, 60, 390, ANCHOR);
        let sex_items = [
            "(not recorded)",
            Sex::Female.as_str(),
            Sex::Male.as_str(),
            Sex::Another.as_str(),
        ];
        let sex_combo = choice(&modal, 150, 98, 180, &sex_items, 0);
        let mut language_rows = vec!["(not recorded)".to_owned()];
        language_rows.extend(languages);
        language_rows.push("Other…".to_owned());
        let language_refs = language_rows.iter().map(String::as_str).collect::<Vec<_>>();
        let language_combo = choice(&modal, 150, 136, 180, &language_refs, 0);
        let aliases_edit = edit(&modal, 150, 174, 390, ANCHOR);
        let _alias_hint = label(&modal, "separated by ;", 462, 200, 78, ANCHOR);
        let phone_edit = edit(&modal, 150, 212, 390, ANCHOR);
        let email_edit = edit(&modal, 150, 250, 390, ANCHOR);
        let address_edit = edit(&modal, 150, 288, 390, ANCHOR);
        let notes_edit = edit(&modal, 150, 326, 390, ANCHOR);
        let _hint = label(
            &modal,
            "Dates are written 2026-08-31. Enter records; Tab moves on.",
            20,
            364,
            520,
            ANCHOR,
        );
        let (accept, cancel) = modal_buttons(&modal, "&Create Client", 386);

        let recorded: Rc<RefCell<Option<ClientDraft>>> = Rc::new(RefCell::new(None));
        let record = {
            let me = self.clone();
            let recorded = recorded.clone();
            let modal = modal.clone();
            let name_edit = name_edit.clone();
            let dob_edit = dob_edit.clone();
            let sex_combo = sex_combo.clone();
            let language_combo = language_combo.clone();
            let language_rows = language_rows.clone();
            let aliases_edit = aliases_edit.clone();
            let phone_edit = phone_edit.clone();
            let email_edit = email_edit.clone();
            let address_edit = address_edit.clone();
            let notes_edit = notes_edit.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let display_name = name_edit.text()?.trim().to_owned();
                if display_name.is_empty() {
                    return Ok(());
                }
                let date_of_birth = optional_text(&dob_edit.text()?);
                if let Some(date) = date_of_birth.as_deref()
                    && office_core::CivilDate::parse(date).is_none()
                {
                    // A wrong date keeps the dialog open; the hint under the
                    // form says how a date is written.
                    return Ok(());
                }
                let sex = match sex_combo.items().selected_index().unwrap_or(0) {
                    1 => Some(Sex::Female),
                    2 => Some(Sex::Male),
                    3 => Some(Sex::Another),
                    _ => None,
                };
                let language_index = language_combo.items().selected_index().unwrap_or(0) as usize;
                let preferred_language = if language_index == 0 {
                    None
                } else if language_index + 1 == language_rows.len() {
                    // `Other…`: the one language line the list did not carry.
                    match prompt_text(
                        &modal,
                        "Language",
                        "The language they ask to be spoken to in.",
                        "",
                    )? {
                        Some(language) => Some(language),
                        None => return Ok(()),
                    }
                } else {
                    language_rows.get(language_index).cloned()
                };
                let aliases = aliases_edit
                    .text()?
                    .split(';')
                    .map(str::trim)
                    .filter(|alias| !alias.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let draft = ClientDraft {
                    display_name,
                    date_of_birth,
                    sex,
                    preferred_language,
                    aliases,
                    phone: optional_text(&phone_edit.text()?),
                    email: optional_text(&email_edit.text()?),
                    address: optional_text(&address_edit.text()?),
                    notes: optional_text(&notes_edit.text()?),
                };

                // The conservative-identity question, asked before the write
                // and never answered by the machine: candidates are shown, and
                // only a person's explicit choice creates the record anyway.
                let candidates = me
                    .workspace
                    .borrow()
                    .client_duplicates(&draft.display_name, &draft.contact_values())
                    .unwrap_or_default();
                if !candidates.is_empty() && !Self::prompt_possible_duplicates(&modal, &candidates)?
                {
                    return Ok(());
                }
                *recorded.borrow_mut() = Some(draft);
                modal.close();
                Ok(())
            })
        };
        let record_click = record.clone();
        accept.on().bn_clicked(move || record_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(
                record,
                vec![
                    Field::text(&name_edit),
                    Field::text(&dob_edit),
                    Field::choice(&sex_combo),
                    Field::choice(&language_combo),
                    Field::text(&aliases_edit),
                    Field::text(&phone_edit),
                    Field::text(&email_edit),
                    Field::text(&address_edit),
                    Field::text(&notes_edit),
                ],
            ),
        );
        wire_cancel(&modal, &cancel);
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;

        let recorded = recorded.borrow().clone();
        if let Some(draft) = recorded {
            let written = self.workspace.borrow_mut().create_client(&draft);
            self.present(written, "Client opened")?;
        }
        Ok(())
    }

    /// Shows who the office may already know, and asks rather than answers.
    fn prompt_possible_duplicates(
        parent: &gui::WindowModal,
        candidates: &[PossiblePerson],
    ) -> w::SysResult<bool> {
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: "Possibly Known",
            size: gui::dpi(520, 310),
            ..Default::default()
        });
        let _caption = label(
            &modal,
            "The office may already know this person. Nothing is merged either way.",
            20,
            20,
            476,
            ANCHOR,
        );
        let mut lines = String::new();
        for person in candidates {
            let born = person
                .date_of_birth
                .as_deref()
                .unwrap_or("birth date not recorded");
            let _ = writeln!(
                lines,
                "{} · {born} · {} matter(s) · matched on {}",
                person.display_name,
                person.matters,
                person.matched_on.join(" / ")
            );
        }
        let _listing = gui::Edit::new(
            &modal,
            gui::EditOpts {
                text: &windows_lines(&lines),
                position: gui::dpi(20, 48),
                width: gui::dpi_x(476),
                height: gui::dpi_y(170),
                control_style: co::ES::MULTILINE | co::ES::AUTOVSCROLL | co::ES::READONLY,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::BORDER
                    | co::WS::VSCROLL
                    | co::WS::TABSTOP,
                ..Default::default()
            },
        );
        // Cancel first: it takes the first tabstop, so a bare Enter walks away
        // rather than creating a possible double.
        let cancel = button(&modal, "Cancel", 268, 232, 112, ANCHOR);
        let create = button(&modal, "Create Any&way", 388, 232, 128, ANCHOR);

        let anyway = Rc::new(RefCell::new(false));
        {
            let anyway = anyway.clone();
            let modal = modal.clone();
            create.on().bn_clicked(move || {
                *anyway.borrow_mut() = true;
                modal.close();
                Ok(())
            });
        }
        wire_cancel(&modal, &cancel);
        modal
            .show_modal(parent)
            .map_err(|_| co::ERROR::INVALID_DATA)?;
        let decided = *anyway.borrow();
        Ok(decided)
    }

    /// The matter entry form: one flow opens the matter and its evidence case.
    fn new_matter(&self) -> w::SysResult<()> {
        let clients = self.workspace.borrow().office_clients().unwrap_or_default();
        if clients.is_empty() {
            return self.set_status("No clients yet. New Client is Ctrl+Shift+L.");
        }
        let courts: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(
            self.workspace.borrow().office_courts().unwrap_or_default(),
        ));
        let cases: Vec<(String, String)> = self
            .workspace
            .borrow()
            .cases()
            .iter()
            .map(|(id, name)| (id.0.clone(), name.clone()))
            .collect();

        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: NEW_MATTER,
            size: gui::dpi(560, 480),
            ..Default::default()
        });
        let rows = [
            "Client",
            "Caption",
            "Court number",
            "Court",
            "Status",
            "Custody",
            "Offer",
            "Charges",
            "Evidence",
            "Case",
        ];
        for (index, caption) in rows.iter().enumerate() {
            let y = 26 + i32::try_from(index).unwrap_or(0) * 38;
            label(&modal, caption, 20, y, 120, ANCHOR);
        }
        let client_rows = clients
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>();
        let client_combo = choice(&modal, 150, 22, 390, &client_rows, 0);
        let caption_edit = edit(&modal, 150, 60, 390, ANCHOR);
        let number_edit = edit(&modal, 150, 98, 390, ANCHOR);
        let initial_court_rows = court_rows(&courts.borrow());
        let initial_court_refs = initial_court_rows
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let court_combo = choice(&modal, 150, 136, 280, &initial_court_refs, 0);
        let status_combo = vocabulary(
            &modal,
            150,
            174,
            200,
            &MatterStatus::ALL.map(MatterStatus::as_str),
            0,
        );
        let custody_combo = vocabulary(
            &modal,
            150,
            212,
            200,
            &CustodyState::ALL.map(CustodyState::as_str),
            0,
        );
        let offer_combo = vocabulary(
            &modal,
            150,
            250,
            200,
            &OfferState::ALL.map(OfferState::as_str),
            0,
        );
        let charges_edit = edit(&modal, 150, 288, 390, ANCHOR);
        let evidence_combo = choice(
            &modal,
            150,
            326,
            280,
            &["Open a new case", "Link an existing case", "None yet"],
            0,
        );
        let case_rows = cases
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>();
        let case_combo = choice(&modal, 150, 364, 390, &case_rows, 0);
        case_combo.hwnd().EnableWindow(false);
        let (accept, cancel) = modal_buttons(&modal, "&Open Matter", 426);

        // The case chooser wakes only when an existing case is what is linked.
        {
            let evidence_combo = evidence_combo.clone();
            let case_combo = case_combo.clone();
            evidence_combo.clone().on().cbn_sel_change(move || {
                let linking = evidence_combo.items().selected_index() == Some(1);
                case_combo.hwnd().EnableWindow(linking);
                Ok(())
            });
        }
        // The last court row records a court the office has not met yet.
        {
            let me = self.clone();
            let courts = courts.clone();
            let court_combo = court_combo.clone();
            let modal = modal.clone();
            court_combo.clone().on().cbn_sel_change(move || {
                let Some(index) = court_combo.items().selected_index() else {
                    return Ok(());
                };
                let count = u32::try_from(courts.borrow().len()).unwrap_or(0);
                if index != count + 1 {
                    return Ok(());
                }
                let named = prompt_text(&modal, "New Court", "The court's name.", "")?;
                match named {
                    Some(name) => {
                        me.workspace.borrow_mut().create_court_named(&name).ok();
                        *courts.borrow_mut() =
                            me.workspace.borrow().office_courts().unwrap_or_default();
                        let rows = court_rows(&courts.borrow());
                        let refs = rows.iter().map(String::as_str).collect::<Vec<_>>();
                        court_combo.items().delete_all();
                        court_combo.items().add(&refs)?;
                        let position = courts
                            .borrow()
                            .iter()
                            .position(|(_, known)| known == &name)
                            .and_then(|p| u32::try_from(p + 1).ok());
                        court_combo.items().select(position);
                    }
                    None => court_combo.items().select(Some(0)),
                }
                Ok(())
            });
        }

        let recorded: Rc<RefCell<Option<MatterDraft>>> = Rc::new(RefCell::new(None));
        let record = {
            let recorded = recorded.clone();
            let modal = modal.clone();
            let clients = clients.clone();
            let courts = courts.clone();
            let cases = cases.clone();
            let client_combo = client_combo.clone();
            let caption_edit = caption_edit.clone();
            let number_edit = number_edit.clone();
            let court_combo = court_combo.clone();
            let status_combo = status_combo.clone();
            let custody_combo = custody_combo.clone();
            let offer_combo = offer_combo.clone();
            let charges_edit = charges_edit.clone();
            let evidence_combo = evidence_combo.clone();
            let case_combo = case_combo.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let caption = caption_edit.text()?.trim().to_owned();
                if caption.is_empty() {
                    return Ok(());
                }
                let client_index = client_combo.items().selected_index().unwrap_or(0) as usize;
                let Some((client_id, _)) = clients.get(client_index) else {
                    return Ok(());
                };
                let court_index = court_combo.items().selected_index().unwrap_or(0) as usize;
                let court_id = if court_index == 0 {
                    None
                } else {
                    courts
                        .borrow()
                        .get(court_index - 1)
                        .map(|(id, _)| id.clone())
                };
                let status_index = status_combo.items().selected_index().unwrap_or(0) as usize;
                let custody_index = custody_combo.items().selected_index().unwrap_or(0) as usize;
                let offer_index = offer_combo.items().selected_index().unwrap_or(0) as usize;
                let evidence = match evidence_combo.items().selected_index().unwrap_or(0) {
                    1 => {
                        let case_index = case_combo.items().selected_index().unwrap_or(0) as usize;
                        let Some((case_id, _)) = cases.get(case_index) else {
                            return Ok(());
                        };
                        EvidenceLink::Existing(crate::CaseId(case_id.clone()))
                    }
                    2 => EvidenceLink::None,
                    _ => EvidenceLink::OpenNewCase,
                };
                *recorded.borrow_mut() = Some(MatterDraft {
                    client_id: client_id.clone(),
                    caption,
                    court_number: optional_text(&number_edit.text()?),
                    court_id,
                    status: MatterStatus::ALL[status_index % MatterStatus::ALL.len()],
                    custody_state: CustodyState::ALL[custody_index % CustodyState::ALL.len()],
                    offer_state: OfferState::ALL[offer_index % OfferState::ALL.len()],
                    charge_summary: optional_text(&charges_edit.text()?),
                    evidence,
                });
                modal.close();
                Ok(())
            })
        };
        let record_click = record.clone();
        accept.on().bn_clicked(move || record_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(
                record,
                vec![
                    Field::choice(&client_combo),
                    Field::text(&caption_edit),
                    Field::text(&number_edit),
                    Field::choice(&court_combo),
                    Field::choice(&status_combo),
                    Field::choice(&custody_combo),
                    Field::choice(&offer_combo),
                    Field::text(&charges_edit),
                    Field::choice(&evidence_combo),
                    Field::choice(&case_combo),
                ],
            ),
        );
        wire_cancel(&modal, &cancel);
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;

        let recorded = recorded.borrow().clone();
        if let Some(draft) = recorded {
            let opened = self.workspace.borrow_mut().open_matter(&draft);
            let succeeded = opened.is_ok();
            self.present(opened, "Matter opened")?;
            if succeeded {
                self.refresh_case_combo()?;
                self.refresh_matter_combo()?;
                self.update_title()?;
            }
        }
        Ok(())
    }

    /// The setting entry form. One setting covers several matters of one
    /// client, which is why the matter list multi-selects.
    fn new_setting(&self) -> w::SysResult<()> {
        let clients = self.workspace.borrow().office_clients().unwrap_or_default();
        if clients.is_empty() {
            return self.set_status("No clients yet. New Client is Ctrl+Shift+L.");
        }
        let courts = self.workspace.borrow().office_courts().unwrap_or_default();
        let docket_date = self.workspace.borrow().docket_date().to_owned();

        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: NEW_SETTING,
            size: gui::dpi(560, 480),
            ..Default::default()
        });
        let _client = label(&modal, "Client", 20, 26, 120, ANCHOR);
        let client_rows = clients
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>();
        let client_combo = choice(&modal, 150, 22, 390, &client_rows, 0);
        let _matters = label(&modal, "Matters", 20, 64, 120, ANCHOR);
        let initial_matters = clients
            .first()
            .map(|(id, _)| {
                self.workspace
                    .borrow()
                    .office_matters_of_client(id)
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let initial_matter_refs = initial_matters
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>();
        let matter_list = gui::ListBox::new(
            &modal,
            gui::ListBoxOpts {
                position: gui::dpi(150, 60),
                size: gui::dpi(390, 96),
                control_style: co::LBS::NOTIFY | co::LBS::MULTIPLESEL,
                items: &initial_matter_refs,
                ..Default::default()
            },
        );
        let _how = label(
            &modal,
            "Space selects; one setting covers every matter it names.",
            150,
            160,
            390,
            ANCHOR,
        );
        let _date = label(&modal, "Date", 20, 190, 120, ANCHOR);
        let date_edit = gui::Edit::new(
            &modal,
            gui::EditOpts {
                text: &docket_date,
                position: gui::dpi(150, 186),
                width: gui::dpi_x(120),
                ..Default::default()
            },
        );
        let _time = label(&modal, "Time", 290, 190, 42, ANCHOR);
        let time_edit = edit(&modal, 340, 186, 70, ANCHOR);
        let _kind = label(&modal, "For", 20, 228, 120, ANCHOR);
        let type_combo = vocabulary(
            &modal,
            150,
            224,
            200,
            &AppearanceType::ALL.map(AppearanceType::as_str),
            1,
        );
        let _court = label(&modal, "Court", 20, 266, 120, ANCHOR);
        let court_labels = court_rows_plain(&courts);
        let court_refs = court_labels.iter().map(String::as_str).collect::<Vec<_>>();
        let court_combo = choice(&modal, 150, 262, 280, &court_refs, 0);
        let _judge = label(&modal, "Judge", 20, 304, 120, ANCHOR);
        let judge_edit = edit(&modal, 150, 300, 280, ANCHOR);
        let _notes = label(&modal, "Notes", 20, 342, 120, ANCHOR);
        let notes_edit = edit(&modal, 150, 338, 390, ANCHOR);
        let hint = label(&modal, "", 20, 380, 520, ANCHOR);
        let (accept, cancel) = modal_buttons(&modal, "&Schedule", 426);

        let listed: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(initial_matters));
        let refill = {
            let me = self.clone();
            let clients = clients.clone();
            let client_combo = client_combo.clone();
            let matter_list = matter_list.clone();
            let listed = listed.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let index = client_combo.items().selected_index().unwrap_or(0) as usize;
                let Some((client_id, _)) = clients.get(index) else {
                    return Ok(());
                };
                let matters = me
                    .workspace
                    .borrow()
                    .office_matters_of_client(client_id)
                    .unwrap_or_default();
                matter_list.items().delete_all();
                let refs = matters
                    .iter()
                    .map(|(_, label)| label.as_str())
                    .collect::<Vec<_>>();
                matter_list.items().add(&refs)?;
                *listed.borrow_mut() = matters;
                Ok(())
            })
        };
        client_combo.clone().on().cbn_sel_change(move || refill());

        let recorded: Rc<RefCell<Option<SettingDraft>>> = Rc::new(RefCell::new(None));
        let record = {
            let recorded = recorded.clone();
            let modal = modal.clone();
            let clients = clients.clone();
            let courts = courts.clone();
            let listed = listed.clone();
            let client_combo = client_combo.clone();
            let matter_list = matter_list.clone();
            let date_edit = date_edit.clone();
            let time_edit = time_edit.clone();
            let type_combo = type_combo.clone();
            let court_combo = court_combo.clone();
            let judge_edit = judge_edit.clone();
            let notes_edit = notes_edit.clone();
            let hint = hint.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let client_index = client_combo.items().selected_index().unwrap_or(0) as usize;
                let Some((client_id, _)) = clients.get(client_index) else {
                    return Ok(());
                };
                let mut matter_ids = Vec::new();
                for selected in matter_list.items().iter_selected()? {
                    let (index, _) = selected?;
                    if let Some((id, _)) = listed.borrow().get(index as usize) {
                        matter_ids.push(id.clone());
                    }
                }
                if matter_ids.is_empty() {
                    hint.hwnd().SetWindowText("Select at least one matter.")?;
                    return Ok(());
                }
                let date = date_edit.text()?.trim().to_owned();
                if office_core::CivilDate::parse(&date).is_none() {
                    hint.hwnd().SetWindowText("Dates are written 2026-08-31.")?;
                    return Ok(());
                }
                let type_index = type_combo.items().selected_index().unwrap_or(0) as usize;
                let court_index = court_combo.items().selected_index().unwrap_or(0) as usize;
                let court_id = if court_index == 0 {
                    None
                } else {
                    courts.get(court_index - 1).map(|(id, _)| id.clone())
                };
                *recorded.borrow_mut() = Some(SettingDraft {
                    client_id: client_id.clone(),
                    matter_ids,
                    court_id,
                    judge: optional_text(&judge_edit.text()?),
                    date,
                    time: optional_text(&time_edit.text()?),
                    appearance_type: AppearanceType::ALL[type_index % AppearanceType::ALL.len()],
                    notes: optional_text(&notes_edit.text()?),
                });
                modal.close();
                Ok(())
            })
        };
        let record_click = record.clone();
        accept.on().bn_clicked(move || record_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(
                record,
                vec![
                    Field::choice(&client_combo),
                    Field::list(&matter_list),
                    Field::text(&date_edit),
                    Field::text(&time_edit),
                    Field::choice(&type_combo),
                    Field::choice(&court_combo),
                    Field::text(&judge_edit),
                    Field::text(&notes_edit),
                ],
            ),
        );
        wire_cancel(&modal, &cancel);
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;

        let recorded = recorded.borrow().clone();
        if let Some(draft) = recorded {
            let scheduled = self.workspace.borrow_mut().schedule_setting(&draft);
            self.present(scheduled, "Setting scheduled")?;
        }
        Ok(())
    }

    /// The deadline entry form: what is owed, on which matter, by when.
    fn new_deadline(&self) -> w::SysResult<()> {
        let matters = self.workspace.borrow().office_matters().unwrap_or_default();
        if matters.is_empty() {
            return self.set_status("No matters yet. New Matter is Ctrl+Shift+M.");
        }
        let docket_date = self.workspace.borrow().docket_date().to_owned();
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: NEW_DEADLINE,
            size: gui::dpi(520, 280),
            ..Default::default()
        });
        let _matter = label(&modal, "Matter", 20, 26, 110, ANCHOR);
        let matter_rows = matters
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>();
        let matter_combo = choice(&modal, 140, 22, 360, &matter_rows, 0);
        let _what = label(&modal, "What is owed", 20, 64, 110, ANCHOR);
        let what_edit = edit(&modal, 140, 60, 360, ANCHOR);
        let _due = label(&modal, "Due", 20, 102, 110, ANCHOR);
        let due_edit = gui::Edit::new(
            &modal,
            gui::EditOpts {
                text: &docket_date,
                position: gui::dpi(140, 98),
                width: gui::dpi_x(120),
                ..Default::default()
            },
        );
        let _origin = label(&modal, "Origin", 20, 140, 110, ANCHOR);
        let origin_combo = vocabulary(
            &modal,
            140,
            136,
            200,
            &DeadlineOrigin::ALL.map(DeadlineOrigin::as_str),
            1,
        );
        let hint = label(&modal, "", 20, 178, 480, ANCHOR);
        let (accept, cancel) = modal_buttons(&modal, "&Record", 224);

        let recorded: Rc<RefCell<Option<DeadlineDraft>>> = Rc::new(RefCell::new(None));
        let record = {
            let recorded = recorded.clone();
            let modal = modal.clone();
            let matters = matters.clone();
            let matter_combo = matter_combo.clone();
            let what_edit = what_edit.clone();
            let due_edit = due_edit.clone();
            let origin_combo = origin_combo.clone();
            let hint = hint.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let description = what_edit.text()?.trim().to_owned();
                if description.is_empty() {
                    return Ok(());
                }
                let due_date = due_edit.text()?.trim().to_owned();
                if office_core::CivilDate::parse(&due_date).is_none() {
                    hint.hwnd().SetWindowText("Dates are written 2026-08-31.")?;
                    return Ok(());
                }
                let matter_index = matter_combo.items().selected_index().unwrap_or(0) as usize;
                let Some((matter_id, _)) = matters.get(matter_index) else {
                    return Ok(());
                };
                let origin_index = origin_combo.items().selected_index().unwrap_or(0) as usize;
                *recorded.borrow_mut() = Some(DeadlineDraft {
                    matter_id: matter_id.clone(),
                    description,
                    due_date,
                    origin: DeadlineOrigin::ALL[origin_index % DeadlineOrigin::ALL.len()],
                });
                modal.close();
                Ok(())
            })
        };
        let record_click = record.clone();
        accept.on().bn_clicked(move || record_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(
                record,
                vec![
                    Field::choice(&matter_combo),
                    Field::text(&what_edit),
                    Field::text(&due_edit),
                    Field::choice(&origin_combo),
                ],
            ),
        );
        wire_cancel(&modal, &cancel);
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;

        let recorded = recorded.borrow().clone();
        if let Some(draft) = recorded {
            let written = self.workspace.borrow_mut().record_office_deadline(&draft);
            self.present(written, "Deadline recorded")?;
        }
        Ok(())
    }

    /// The note entry form. A note is filed under exactly one client, matter,
    /// or setting; Enter writes it, so a note that wants paragraphs wants the
    /// CLI instead.
    fn new_note(&self) -> w::SysResult<()> {
        let docket_date = self.workspace.borrow().docket_date().to_owned();
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: NEW_NOTE,
            size: gui::dpi(560, 330),
            ..Default::default()
        });
        let _scope = label(&modal, "Filed under", 20, 26, 120, ANCHOR);
        let scope_combo = choice(&modal, 150, 22, 160, &["client", "matter", "setting"], 0);
        let _subject = label(&modal, "Subject", 20, 64, 120, ANCHOR);
        let initial_subjects = self.workspace.borrow().office_clients().unwrap_or_default();
        let initial_subject_refs = initial_subjects
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>();
        let subject_combo = choice(&modal, 150, 60, 390, &initial_subject_refs, 0);
        let _body = label(&modal, "Note", 20, 102, 120, ANCHOR);
        // Multiline without `ES::WANTRETURN`, so Enter reaches
        // `IsDialogMessage` and writes the note instead of a newline.
        let body_edit = gui::Edit::new(
            &modal,
            gui::EditOpts {
                text: "",
                position: gui::dpi(150, 98),
                width: gui::dpi_x(390),
                height: gui::dpi_y(150),
                control_style: co::ES::MULTILINE | co::ES::AUTOVSCROLL,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::BORDER
                    | co::WS::VSCROLL
                    | co::WS::TABSTOP,
                ..Default::default()
            },
        );
        let (accept, cancel) = modal_buttons(&modal, "&Write Note", 276);

        let subjects: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(initial_subjects));
        let refill = {
            let me = self.clone();
            let scope_combo = scope_combo.clone();
            let subject_combo = subject_combo.clone();
            let subjects = subjects.clone();
            let docket_date = docket_date.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let rows = match scope_combo.items().selected_index().unwrap_or(0) {
                    1 => me.workspace.borrow().office_matters().unwrap_or_default(),
                    2 => me
                        .workspace
                        .borrow()
                        .office_settings_on(&docket_date)
                        .unwrap_or_default(),
                    _ => me.workspace.borrow().office_clients().unwrap_or_default(),
                };
                subject_combo.items().delete_all();
                let refs = rows
                    .iter()
                    .map(|(_, label)| label.as_str())
                    .collect::<Vec<_>>();
                subject_combo.items().add(&refs)?;
                subject_combo.items().select(Some(0));
                *subjects.borrow_mut() = rows;
                Ok(())
            })
        };
        scope_combo.clone().on().cbn_sel_change(move || refill());

        let recorded: Rc<RefCell<Option<NoteDraft>>> = Rc::new(RefCell::new(None));
        let record = {
            let recorded = recorded.clone();
            let modal = modal.clone();
            let subjects = subjects.clone();
            let scope_combo = scope_combo.clone();
            let subject_combo = subject_combo.clone();
            let body_edit = body_edit.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let body = body_edit.text()?.trim().to_owned();
                if body.is_empty() {
                    return Ok(());
                }
                let subject_index = subject_combo.items().selected_index().unwrap_or(0) as usize;
                let Some((subject_id, _)) = subjects.borrow().get(subject_index).cloned() else {
                    return Ok(());
                };
                let scope = match scope_combo.items().selected_index().unwrap_or(0) {
                    1 => NoteScope::Matter,
                    2 => NoteScope::Appearance,
                    _ => NoteScope::Client,
                };
                *recorded.borrow_mut() = Some(NoteDraft {
                    scope,
                    subject_id,
                    body,
                });
                modal.close();
                Ok(())
            })
        };
        let record_click = record.clone();
        accept.on().bn_clicked(move || record_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(
                record,
                vec![
                    Field::choice(&scope_combo),
                    Field::choice(&subject_combo),
                    Field::text(&body_edit),
                ],
            ),
        );
        wire_cancel(&modal, &cancel);
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;

        let recorded = recorded.borrow().clone();
        if let Some(draft) = recorded {
            let written = self.workspace.borrow_mut().write_office_note(&draft);
            self.present(written, "Note written")?;
        }
        Ok(())
    }

    fn open_database(&self) -> w::SysResult<()> {
        if self.workspace.borrow().has_active_intake().unwrap_or(false) {
            return self.present(
                Err(GuiError::new(
                    "This database has an actively running intake job. Wait for it to finish before switching.",
                )),
                "Open database",
            );
        }
        let path = self.database_edit.text()?;
        match Workspace::open(path.trim()) {
            Ok(workspace) => {
                let coordinator = IntakeCoordinator::start(path.trim()).map_err(GuiError::from);
                let coordinator = match coordinator {
                    Ok(coordinator) => coordinator,
                    Err(error) => return self.present(Err(error), "Open database"),
                };
                *self.workspace.borrow_mut() = workspace;
                *self.coordinator.borrow_mut() = Some(coordinator);
                self.refresh_case_combo()?;
                self.show_view(WorkspaceView::Overview)?;
                self.set_status(&format!("Opened {}.", path.trim()))
            }
            Err(error) => self.present(Err(error), "Open database"),
        }
    }

    fn refresh_case_combo(&self) -> w::SysResult<()> {
        let workspace = self.workspace.borrow();
        let labels = case_labels(&workspace);
        let selected = workspace.active_case_index();
        drop(workspace);
        self.case_combo.items().delete_all();
        self.case_combo.items().add(&labels)?;
        self.case_combo
            .items()
            .select(selected.and_then(|index| u32::try_from(index).ok()));
        self.update_title()
    }

    /// Keeps the title bar naming the selected case, so two open workspaces
    /// are told apart in the taskbar rather than by their contents.
    fn update_title(&self) -> w::SysResult<()> {
        let title = self.workspace.borrow().active_case().map_or_else(
            || "Evidence Intake — Local Case Workspace".to_owned(),
            |(id, name)| format!("Evidence Intake — {name}  [{id}]"),
        );
        self.wnd.hwnd().SetWindowText(&title)
    }

    fn show_view(&self, view: WorkspaceView) -> w::SysResult<()> {
        let result = self.workspace.borrow().render(view);
        // Naming the chord is how the two views without an Alt letter teach
        // their own keyboard path.
        let caption = VIEW_BUTTONS
            .iter()
            .position(|(_, candidate)| *candidate == view)
            .map_or_else(
                || view.label().to_owned(),
                |index| {
                    let (shift, key) = view_accelerator(index);
                    format!("{}  ({})", view.label(), accelerator_label(shift, key))
                },
            );
        self.present(result, &caption)
    }

    fn apply_review(&self, state: ReviewState) -> w::SysResult<()> {
        let target_label = self
            .target_combo
            .items()
            .selected_text()?
            .unwrap_or_default();
        let Some(target) = review_target(&target_label) else {
            return self.present(Err(GuiError::new("Select a review target kind.")), "Review");
        };
        let target_id = self.target_edit.text()?;
        let actor = self.actor_edit.text()?;
        let basis_text = self.payload_edit.text()?;
        let locator_text = self.locator_edit.text()?;
        let basis = optional_text(&basis_text);
        let locator = optional_text(&locator_text);
        let result = self.workspace.borrow_mut().review(
            target,
            target_id.trim().to_owned(),
            state,
            actor.trim().to_owned(),
            basis,
            locator,
        );
        self.present(result, &format!("Review recorded as {}.", state.as_str()))
    }

    fn selected_author_kind(&self) -> w::SysResult<AuthorKind> {
        let label = self
            .author_combo
            .items()
            .selected_text()?
            .unwrap_or_default();
        AuthorKind::from_label(&label).ok_or(w::co::ERROR::INVALID_DATA)
    }

    fn save_export(&self) -> w::SysResult<()> {
        let workspace = self.workspace.borrow();
        let Some((case_id, _)) = workspace.active_case() else {
            return self.present(
                Err(GuiError::new("Select a case before exporting.")),
                "Save export",
            );
        };
        let database = workspace
            .database()
            .map_or_else(|| PathBuf::from("evidence.db"), Path::to_path_buf);
        let parent = database.parent().unwrap_or_else(|| Path::new("."));
        let stem = database
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("evidence");
        let path = parent.join(format!("{stem}-{}-disclosable.json", case_id.0));
        let result = workspace
            .save_export(&path, ExportAudience::Disclosable)
            .map(|()| format!("Saved source-linked export to {}", path.display()));
        drop(workspace);
        self.present(result, "Disclosable export saved")
    }

    fn present(&self, result: GuiResult<String>, success: &str) -> w::SysResult<()> {
        match result {
            Ok(text) => {
                self.output_edit.set_text(&windows_lines(&text))?;
                self.set_status(success)
            }
            Err(error) => {
                self.output_edit
                    .set_text(&format!("ERROR\r\n\r\n{error}"))?;
                self.set_status("Operation failed — no changes were hidden.")
            }
        }
    }

    fn set_status(&self, text: &str) -> w::SysResult<()> {
        self.status_label.hwnd().SetWindowText(text)
    }
}

/// Stays where it is when the window is resized.
const ANCHOR: (gui::Horz, gui::Vert) = (gui::Horz::None, gui::Vert::None);
/// Rides the right edge.
const SLIDE_X: (gui::Horz, gui::Vert) = (gui::Horz::Repos, gui::Vert::None);
/// Rides the bottom edge.
const SLIDE_Y: (gui::Horz, gui::Vert) = (gui::Horz::None, gui::Vert::Repos);
/// Rides the bottom-right corner.
const SLIDE_XY: (gui::Horz, gui::Vert) = (gui::Horz::Repos, gui::Vert::Repos);
/// Grows sideways while riding the bottom edge.
const STRETCH_X_SLIDE_Y: (gui::Horz, gui::Vert) = (gui::Horz::Resize, gui::Vert::Repos);

fn label(
    parent: &(impl GuiParent + 'static),
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    resize_behavior: (gui::Horz, gui::Vert),
) -> gui::Label {
    gui::Label::new(
        parent,
        gui::LabelOpts {
            text,
            position: gui::dpi(x, y),
            size: gui::dpi(width, 20),
            // A label that runs out of room ends in an ellipsis instead of a
            // word cut mid-stroke, which matters once the window is narrowed.
            control_style: co::SS::LEFT | co::SS::NOTIFY | co::SS::ENDELLIPSIS,
            resize_behavior,
            ..Default::default()
        },
    )
}

fn button(
    parent: &(impl GuiParent + 'static),
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    resize_behavior: (gui::Horz, gui::Vert),
) -> gui::Button {
    gui::Button::new(
        parent,
        gui::ButtonOpts {
            text,
            position: gui::dpi(x, y),
            width: gui::dpi_x(width),
            height: gui::dpi_y(BUTTON_HEIGHT),
            resize_behavior,
            ..Default::default()
        },
    )
}

fn view_button(
    parent: &(impl GuiParent + 'static),
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    resize_behavior: (gui::Horz, gui::Vert),
) -> gui::Button {
    gui::Button::new(
        parent,
        gui::ButtonOpts {
            text,
            position: gui::dpi(x, y),
            width: gui::dpi_x(width),
            height: gui::dpi_y(VIEW_BUTTON_HEIGHT),
            resize_behavior,
            ..Default::default()
        },
    )
}

fn edit(
    parent: &(impl GuiParent + 'static),
    x: i32,
    y: i32,
    width: i32,
    resize_behavior: (gui::Horz, gui::Vert),
) -> gui::Edit {
    gui::Edit::new(
        parent,
        gui::EditOpts {
            text: "",
            position: gui::dpi(x, y),
            width: gui::dpi_x(width),
            resize_behavior,
            ..Default::default()
        },
    )
}

const fn rgb((red, green, blue): (u8, u8, u8)) -> w::COLORREF {
    w::COLORREF::from_rgb(red, green, blue)
}

fn case_labels(workspace: &Workspace) -> Vec<String> {
    workspace
        .cases()
        .iter()
        .map(|(id, name)| format!("{name}  [{id}]"))
        .collect()
}

fn select_files(parent: &w::HWND, modality: IntakeModality) -> w::HrResult<Vec<PathBuf>> {
    let dialog = w::CoCreateInstance::<w::IFileOpenDialog>(
        &co::CLSID::FileOpenDialog,
        None::<&w::IUnknown>,
        co::CLSCTX::INPROC_SERVER,
    )?;
    dialog.SetOptions(
        dialog.GetOptions()?
            | co::FOS::FORCEFILESYSTEM
            | co::FOS::FILEMUSTEXIST
            | co::FOS::ALLOWMULTISELECT,
    )?;
    let types = match modality {
        IntakeModality::Document => [("PDF documents", "*.pdf"), ("All files", "*.*")],
        IntakeModality::Audio => [
            (
                "Audio recordings",
                "*.wav;*.mp3;*.m4a;*.flac;*.ogg;*.opus;*.aac",
            ),
            ("All files", "*.*"),
        ],
        IntakeModality::Video => [
            ("Video recordings", "*.mp4;*.mov;*.mkv;*.avi;*.webm;*.m4v"),
            ("All files", "*.*"),
        ],
    };
    dialog.SetFileTypes(&types)?;
    dialog.SetFileTypeIndex(1)?;
    if !dialog.Show(parent)? {
        return Ok(Vec::new());
    }
    dialog
        .GetResults()?
        .iter()?
        .map(|item| {
            item.and_then(|item| item.GetDisplayName(co::SIGDN::FILESYSPATH))
                .map(PathBuf::from)
        })
        .collect()
}

fn artifact_sidecar(database: &Path) -> PathBuf {
    let parent = database.parent().unwrap_or_else(|| Path::new("."));
    let stem = database
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("evidence.sqlite");
    parent.join(format!("{stem}.artifacts"))
}

fn companion(directory: &Path, name: &str) -> PathBuf {
    directory.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    })
}

fn hash_file(path: &Path) -> GuiResult<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    if length == 0 {
        return Err(GuiError::new(format!("{} is empty.", path.display())));
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok((format!("{:x}", digest.finalize()), length))
}

fn selected_queue_job(
    jobs: &Rc<RefCell<Vec<crate::IntakeJob>>>,
    combo: &gui::ComboBox,
) -> Option<crate::IntakeJob> {
    combo
        .items()
        .selected_index()
        .and_then(|index| jobs.borrow().get(index as usize).cloned())
}

fn refresh_queue_view(
    workspace: &Rc<RefCell<Workspace>>,
    combo: &gui::ComboBox,
    output: &gui::Edit,
    jobs: &Rc<RefCell<Vec<crate::IntakeJob>>>,
) -> w::SysResult<()> {
    let selected_id = selected_queue_job(jobs, combo).map(|job| job.id);
    match workspace.borrow().intake_jobs() {
        Ok(current) => {
            let labels = current
                .iter()
                .map(|job| {
                    format!(
                        "{}  {}  attempt {}  [{}]",
                        job.modality,
                        job.logical_name,
                        job.attempt,
                        job.state.as_str()
                    )
                })
                .collect::<Vec<_>>();
            let selected = selected_id
                .as_ref()
                .and_then(|id| current.iter().position(|job| &job.id == id))
                .or_else(|| (!current.is_empty()).then_some(0));
            combo.items().delete_all();
            combo.items().add(&labels)?;
            combo
                .items()
                .select(selected.and_then(|index| u32::try_from(index).ok()));
            let rows = current
                .iter()
                .map(|job| {
                    let artifacts = workspace
                        .borrow()
                        .intake_artifacts(&job.id)
                        .unwrap_or_default();
                    serde_json::json!({
                        "job": job.id,
                        "source": job.logical_name,
                        "modality": job.modality,
                        "profile": job.profile,
                        "state": job.state,
                        "attempt": job.attempt,
                        "stage": job.stage,
                        "completed": job.progress_completed,
                        "total": job.progress_total,
                        "message": job.message,
                        "error": job.error,
                        "original": job.original_path,
                        "artifact_directory": job.artifact_dir,
                        "artifacts": artifacts,
                    })
                })
                .collect::<Vec<_>>();
            *jobs.borrow_mut() = current;
            output.set_text(&windows_lines(&readable_text(&serde_json::Value::Array(
                rows,
            ))))?;
        }
        Err(error) => output.set_text(&format!("ERROR\r\n\r\n{error}"))?,
    }
    Ok(())
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn windows_lines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Starts the Windows native frontend for one database path.
pub fn run(database: &Path) {
    let _com_guard =
        w::CoInitializeEx(co::COINIT::APARTMENTTHREADED | co::COINIT::DISABLE_OLE1DDE).ok();
    if let Err(error) = MainWindow::create_and_run(database) {
        eprintln!("Evidence Intake GUI error: {error}");
    }
}

/// Sweep columns in the order their function keys select them.
///
/// One field at a time down the page is the throughput shape: attention stays
/// on a single question ("who is speaking here?") and the rhythm is one key
/// per row, instead of a form per passage.
const SWEEP_FIELDS: [(&str, EnrichmentField, &str); 7] = [
    ("F2  Form", EnrichmentField::ContentForm, "F2"),
    ("F3  Speaker", EnrichmentField::Speaker, "F3"),
    (
        "F4  Attributed person",
        EnrichmentField::AttributedPerson,
        "F4",
    ),
    ("F5  Temporal stance", EnrichmentField::TemporalStance, "F5"),
    ("F6  Time", EnrichmentField::Time, "F6"),
    ("F7  Location", EnrichmentField::Location, "F7"),
    (
        "F8  Perception basis",
        EnrichmentField::PerceptionBasis,
        "F8",
    ),
];

/// Grid columns, in logical units before DPI scaling.
const GRID_COLUMNS: [(&str, i32); 6] = [
    ("#", 44),
    ("Locator", 128),
    ("Passage", 470),
    ("Value", 150),
    ("Provenance", 176),
    ("Badge", 168),
];

/// Design size and floor of the enrichment window.
const SWEEP_WIDTH: i32 = 1180;
const SWEEP_HEIGHT: i32 = 740;
const SWEEP_MIN_WIDTH: i32 = 900;
const SWEEP_MIN_HEIGHT: i32 = 600;

/// Grid geometry. The grid is the window: everything above it decides once,
/// everything below it shows what the row under the cursor rests on.
///
/// A row is about twenty logical units tall and a caption twenty more, which
/// is what `the_sweep_window_fits_its_own_floor` measures the layout against.
const GRID_ROW_HEIGHT: i32 = 20;
const CAPTION_HEIGHT: i32 = 20;
const SWEEP_GRID_TOP: i32 = 122;
const SWEEP_GRID_HEIGHT: i32 = 276;
/// Distance from the window's bottom edge to each anchored row below the grid.
const SWEEP_PREVIEW_FROM_BOTTOM: i32 = 316;
const SWEEP_KEYS_FROM_BOTTOM: i32 = 196;
const SWEEP_STATUS_FROM_BOTTOM: i32 = 40;

/// The grid resizes with the window and everything below it is anchored to the
/// bottom edge, so these clearances hold at every height the window can take.
const _: () = assert!(
    SWEEP_GRID_TOP + SWEEP_GRID_HEIGHT <= SWEEP_HEIGHT - SWEEP_PREVIEW_FROM_BOTTOM - CAPTION_HEIGHT,
    "the grid overlaps the caption above the original"
);
const _: () = assert!(
    SWEEP_PREVIEW_FROM_BOTTOM > SWEEP_KEYS_FROM_BOTTOM,
    "the preview must sit above the key sheet"
);
const _: () = assert!(
    SWEEP_KEYS_FROM_BOTTOM > SWEEP_STATUS_FROM_BOTTOM,
    "the key sheet must sit above the status line"
);
/// Shrunk to its floor the grid still shows six rows, which is the least a
/// sweep can work with: the row under the cursor, and the run around it.
const _: () = assert!(
    SWEEP_GRID_HEIGHT - (SWEEP_HEIGHT - SWEEP_MIN_HEIGHT) >= 6 * GRID_ROW_HEIGHT,
    "at the floor the grid has no rows left"
);

/// The keyboard workspace over one source: grid on the left of the decision,
/// the original beside it, and one key per passage.
#[derive(Clone)]
struct SweepWindow {
    wnd: gui::WindowModal,
    workspace: Rc<RefCell<Workspace>>,
    sources: Rc<RefCell<Vec<crate::EnrichmentSource>>>,
    authors: Rc<RefCell<Vec<crate::EntityCandidate>>>,
    preview: Rc<RefCell<Option<PreviewDescriptor>>>,
    _labels: Vec<gui::Label>,
    actor_edit: gui::Edit,
    source_combo: gui::ComboBox,
    open_button: gui::Button,
    candidates_button: gui::Button,
    role_combo: gui::ComboBox,
    author_combo: gui::ComboBox,
    created_edit: gui::Edit,
    form_combo: gui::ComboBox,
    stance_combo: gui::ComboBox,
    basis_combo: gui::ComboBox,
    profile_button: gui::Button,
    field_combo: gui::ComboBox,
    counts_label: gui::Label,
    grid: gui::ListView,
    preview_edit: gui::Edit,
    original_button: gui::Button,
    context_button: gui::Button,
    accept_button: gui::Button,
    reject_button: gui::Button,
    keys_edit: gui::Edit,
    status_label: gui::Label,
    close_button: gui::Button,
}

impl SweepWindow {
    fn show(parent: &MainWindow, actor: &str) -> w::SysResult<()> {
        let wnd = gui::WindowModal::new(gui::WindowModalOpts {
            title: "Enrichment Sweep — decide once, then one key per passage",
            size: gui::dpi(SWEEP_WIDTH, SWEEP_HEIGHT),
            style: co::WS::CAPTION
                | co::WS::SYSMENU
                | co::WS::THICKFRAME
                | co::WS::CLIPCHILDREN
                | co::WS::VISIBLE
                | co::WS::BORDER,
            ..Default::default()
        });

        let mut labels = vec![label(&wnd, "Source", 20, 22, 50, ANCHOR)];
        let source_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(74, 18),
                width: gui::dpi_x(486),
                items: &[],
                resize_behavior: ANCHOR,
                ..Default::default()
            },
        );
        labels.push(label(&wnd, "Reviewer", 576, 22, 62, ANCHOR));
        let actor_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: actor,
                position: gui::dpi(642, 18),
                width: gui::dpi_x(170),
                ..Default::default()
            },
        );
        let open_button = button(&wnd, "&Open Sweep", 824, 16, 116, ANCHOR);
        let candidates_button = button(&wnd, "Propose &Candidates", 950, 16, 160, SLIDE_X);

        labels.push(label(&wnd, "Role", 20, 58, 34, ANCHOR));
        let role_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(58, 54),
                width: gui::dpi_x(190),
                items: &[],
                ..Default::default()
            },
        );
        labels.push(label(&wnd, "Author", 260, 58, 46, ANCHOR));
        let author_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(310, 54),
                width: gui::dpi_x(180),
                items: &[],
                ..Default::default()
            },
        );
        labels.push(label(&wnd, "Created", 502, 58, 52, ANCHOR));
        let created_edit = edit(&wnd, 556, 54, 120, ANCHOR);
        labels.push(label(&wnd, "Defaults", 688, 58, 56, ANCHOR));
        let form_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(748, 54),
                width: gui::dpi_x(174),
                items: &[],
                ..Default::default()
            },
        );
        let stance_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(20, 88),
                width: gui::dpi_x(228),
                items: &[],
                ..Default::default()
            },
        );
        let basis_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(260, 88),
                width: gui::dpi_x(230),
                items: &[],
                ..Default::default()
            },
        );
        let profile_button = button(&wnd, "&Save Profile", 932, 52, 178, SLIDE_X);

        labels.push(label(&wnd, "Sweep field", 502, 92, 74, ANCHOR));
        let field_combo = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(580, 88),
                width: gui::dpi_x(200),
                items: &SWEEP_FIELDS.map(|(caption, _, _)| caption),
                selected_item: Some(0),
                ..Default::default()
            },
        );
        let counts_label = label(&wnd, "No source is open.", 790, 92, 320, SLIDE_X);

        let columns = GRID_COLUMNS
            .map(|(caption, width)| (caption, gui::dpi_x(width)))
            .to_vec();
        let grid = gui::ListView::<()>::new(
            &wnd,
            gui::ListViewOpts {
                position: gui::dpi(20, SWEEP_GRID_TOP),
                size: gui::dpi(SWEEP_WIDTH - 40, SWEEP_GRID_HEIGHT),
                columns: &columns,
                control_ex_style: co::LVS_EX::FULLROWSELECT | co::LVS_EX::GRIDLINES,
                resize_behavior: (gui::Horz::Resize, gui::Vert::Resize),
                ..Default::default()
            },
        );

        let preview_top = SWEEP_HEIGHT - SWEEP_PREVIEW_FROM_BOTTOM;
        labels.push(label(
            &wnd,
            "The original under the cursor — check derived text against it before verifying",
            20,
            preview_top - 20,
            700,
            SLIDE_Y,
        ));
        let preview_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: "",
                position: gui::dpi(20, preview_top),
                width: gui::dpi_x(800),
                height: gui::dpi_y(96),
                control_style: co::ES::MULTILINE
                    | co::ES::AUTOVSCROLL
                    | co::ES::AUTOHSCROLL
                    | co::ES::READONLY,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::BORDER
                    | co::WS::VSCROLL
                    | co::WS::TABSTOP,
                resize_behavior: STRETCH_X_SLIDE_Y,
                ..Default::default()
            },
        );
        let original_button = button(&wnd, "Open Ori&ginal", 836, preview_top, 150, SLIDE_XY);
        let context_button = button(&wnd, "Open Conte&xt", 996, preview_top, 150, SLIDE_XY);
        let accept_button = button(
            &wnd,
            "&Accept (Enter)",
            836,
            preview_top + 34,
            150,
            SLIDE_XY,
        );
        let reject_button = button(&wnd, "&Reject (x)", 996, preview_top + 34, 150, SLIDE_XY);

        let keys_top = SWEEP_HEIGHT - SWEEP_KEYS_FROM_BOTTOM;
        let keys_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: "",
                position: gui::dpi(20, keys_top),
                width: gui::dpi_x(SWEEP_WIDTH - 40),
                height: gui::dpi_y(136),
                control_style: co::ES::MULTILINE
                    | co::ES::AUTOVSCROLL
                    | co::ES::AUTOHSCROLL
                    | co::ES::READONLY,
                window_style: co::WS::CHILD
                    | co::WS::VISIBLE
                    | co::WS::BORDER
                    | co::WS::VSCROLL
                    | co::WS::TABSTOP,
                resize_behavior: STRETCH_X_SLIDE_Y,
                ..Default::default()
            },
        );

        let status_top = SWEEP_HEIGHT - SWEEP_STATUS_FROM_BOTTOM;
        let status_label = label(
            &wnd,
            "Choose a source and press Open Sweep. Every key you press is written under your name.",
            20,
            status_top,
            940,
            STRETCH_X_SLIDE_Y,
        );
        let close_button = button(
            &wnd,
            "Close",
            SWEEP_WIDTH - 140,
            status_top - 6,
            120,
            SLIDE_XY,
        );

        let window = Self {
            wnd,
            workspace: parent.workspace.clone(),
            sources: Rc::new(RefCell::new(Vec::new())),
            authors: Rc::new(RefCell::new(Vec::new())),
            preview: Rc::new(RefCell::new(None)),
            _labels: labels,
            actor_edit,
            source_combo,
            open_button,
            candidates_button,
            role_combo,
            author_combo,
            created_edit,
            form_combo,
            stance_combo,
            basis_combo,
            profile_button,
            field_combo,
            counts_label,
            grid,
            preview_edit,
            original_button,
            context_button,
            accept_button,
            reject_button,
            keys_edit,
            status_label,
            close_button,
        };
        window.events();
        window
            .wnd
            .show_modal(&parent.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)
    }

    fn events(&self) {
        let me = self.clone();
        self.wnd.on().wm_create(move |_| {
            fit_window_to_work_area(me.wnd.hwnd(), SWEEP_MIN_WIDTH, SWEEP_MIN_HEIGHT)?;
            me.fill_vocabularies()?;
            me.reload_sources()?;
            me.refresh_keys()?;
            Ok(0)
        });

        let me = self.clone();
        self.wnd.on().wm_get_min_max_info(move |info| {
            if let Some((frame_width, frame_height)) = frame_margins(me.wnd.hwnd()) {
                info.info.ptMinTrackSize = w::POINT::with(
                    gui::dpi_x(SWEEP_MIN_WIDTH) + frame_width,
                    gui::dpi_y(SWEEP_MIN_HEIGHT) + frame_height,
                );
            }
            Ok(())
        });

        let me = self.clone();
        self.open_button.on().bn_clicked(move || {
            me.open_selected_source()?;
            Ok(())
        });

        let me = self.clone();
        self.source_combo.on().cbn_sel_change(move || {
            me.show_profile_of_selected_source()?;
            Ok(())
        });

        let me = self.clone();
        self.profile_button.on().bn_clicked(move || {
            me.save_profile()?;
            Ok(())
        });

        let me = self.clone();
        self.candidates_button.on().bn_clicked(move || {
            let outcome = me.workspace.borrow_mut().suggest_enrichment();
            match outcome {
                Ok(_) => {
                    me.reload_sources()?;
                    me.refresh_grid()?;
                    me.set_status(
                        "Deterministic rules proposed candidates; Enter accepts one, x refuses it.",
                    )?;
                }
                Err(error) => me.set_status(&format!("ERROR  {error}"))?,
            }
            Ok(())
        });

        let me = self.clone();
        self.field_combo.on().cbn_sel_change(move || {
            if let Some(index) = me.field_combo.items().selected_index() {
                let token = SWEEP_FIELDS
                    .get(index as usize)
                    .map_or("F2", |(_, _, token)| *token);
                me.send_key(token)?;
            }
            Ok(())
        });

        let me = self.clone();
        self.grid.on().lvn_key_down(move |key| {
            me.on_grid_key(key.wVKey)?;
            Ok(())
        });

        let me = self.clone();
        self.grid.on().nm_click(move |clicked| {
            if clicked.iItem >= 0 {
                let index = usize::try_from(clicked.iItem).unwrap_or(0);
                let outcome = me.workspace.borrow_mut().enrichment_select(index);
                me.after_command(outcome)?;
            }
            Ok(())
        });

        // `IsDialogMessage` turns Escape into `IDCANCEL` before the grid ever
        // sees it, so command mode is claimed here rather than in the grid's
        // key handler.
        let me = self.clone();
        self.wnd
            .on()
            .wm_command(co::DLGID::CANCEL, co::CMD::Menu, move || {
                me.send_key("Esc")?;
                Ok(())
            });

        let me = self.clone();
        self.accept_button.on().bn_clicked(move || {
            me.send_key("Enter")?;
            Ok(())
        });

        let me = self.clone();
        self.reject_button.on().bn_clicked(move || {
            me.send_key("x")?;
            Ok(())
        });

        let me = self.clone();
        self.original_button.on().bn_clicked(move || {
            me.open_preview_path(false)?;
            Ok(())
        });

        let me = self.clone();
        self.context_button.on().bn_clicked(move || {
            me.open_preview_path(true)?;
            Ok(())
        });

        let me = self.clone();
        self.close_button.on().bn_clicked(move || {
            me.wnd.close();
            Ok(())
        });
    }

    /// Fills the controlled vocabularies. Every value a person can enter comes
    /// from the kernel's own enums, so a frontend cannot invent a role.
    fn fill_vocabularies(&self) -> w::SysResult<()> {
        self.role_combo.items().delete_all();
        self.role_combo.items().add(
            &SourceRole::ALL
                .iter()
                .map(|role| role.as_str())
                .collect::<Vec<_>>(),
        )?;
        self.role_combo.items().select(Some(0));

        self.form_combo.items().delete_all();
        let mut forms = vec!["default form: none"];
        forms.extend(ContentForm::ALL.iter().map(|form| form.as_str()));
        self.form_combo.items().add(&forms)?;
        self.form_combo.items().select(Some(0));

        self.stance_combo.items().delete_all();
        let mut stances = vec!["default stance: none"];
        stances.extend(TemporalStance::ALL.iter().map(|stance| stance.as_str()));
        self.stance_combo.items().add(&stances)?;
        self.stance_combo.items().select(Some(0));

        self.basis_combo.items().delete_all();
        let mut bases = vec!["default basis: none"];
        bases.extend(PerceptionBasis::ALL.iter().map(|basis| basis.as_str()));
        self.basis_combo.items().add(&bases)?;
        self.basis_combo.items().select(Some(0));
        Ok(())
    }

    fn reload_sources(&self) -> w::SysResult<()> {
        let (sources, authors) = {
            let workspace = self.workspace.borrow();
            (
                workspace.enrichment_sources().unwrap_or_default(),
                workspace.entity_candidates("", 200).unwrap_or_default(),
            )
        };
        let captions = sources
            .iter()
            .map(|source| {
                format!(
                    "{}  [{}]  {} passages · {} awaiting a person · {} candidates{}",
                    source.name,
                    source.kind,
                    source.passages,
                    source.outstanding(),
                    source.candidates,
                    if source.has_profile() {
                        String::new()
                    } else {
                        "  · no profile".to_owned()
                    }
                )
            })
            .collect::<Vec<_>>();
        let selected = self.source_combo.items().selected_index();
        self.source_combo.items().delete_all();
        if !captions.is_empty() {
            self.source_combo.items().add(&captions)?;
            let index = selected
                .filter(|index| (*index as usize) < captions.len())
                .unwrap_or(0);
            self.source_combo.items().select(Some(index));
        }

        let author_captions = std::iter::once("author: not recorded".to_owned())
            .chain(
                authors
                    .iter()
                    .map(|entity| format!("{} ({})", entity.display_name, entity.kind)),
            )
            .collect::<Vec<_>>();
        self.author_combo.items().delete_all();
        self.author_combo.items().add(&author_captions)?;
        self.author_combo.items().select(Some(0));

        *self.sources.borrow_mut() = sources;
        *self.authors.borrow_mut() = authors;
        self.show_profile_of_selected_source()
    }

    fn selected_source(&self) -> Option<crate::EnrichmentSource> {
        let index = self.source_combo.items().selected_index()? as usize;
        self.sources.borrow().get(index).cloned()
    }

    /// Shows the current profile of the selected source, so the screen always
    /// reads as "the profile of this file" rather than a blank form.
    fn show_profile_of_selected_source(&self) -> w::SysResult<()> {
        let Some(source) = self.selected_source() else {
            return Ok(());
        };
        let profile = self
            .workspace
            .borrow()
            .source_profile(&source.source_id)
            .unwrap_or_default();
        let Some(profile) = profile else {
            self.created_edit.set_text("")?;
            return Ok(());
        };
        if let Some(index) = SourceRole::ALL
            .iter()
            .position(|role| *role == profile.source_role)
        {
            self.role_combo.items().select(u32::try_from(index).ok());
        }
        self.created_edit
            .set_text(profile.created_at_claim.as_deref().unwrap_or(""))?;
        let author_index = profile.author_entity_id.as_deref().and_then(|id| {
            self.authors
                .borrow()
                .iter()
                .position(|entity| entity.id == id)
        });
        self.author_combo.items().select(Some(
            author_index.map_or(0, |index| u32::try_from(index + 1).unwrap_or(0)),
        ));
        select_optional(
            &self.form_combo,
            ContentForm::ALL,
            profile.default_content_form,
        );
        select_optional(
            &self.stance_combo,
            TemporalStance::ALL,
            profile.default_temporal_stance,
        );
        select_optional(
            &self.basis_combo,
            PerceptionBasis::ALL,
            profile.default_perception_basis,
        );
        Ok(())
    }

    fn actor(&self) -> String {
        self.actor_edit.text().unwrap_or_default().trim().to_owned()
    }

    fn save_profile(&self) -> w::SysResult<()> {
        let Some(source) = self.selected_source() else {
            return self.set_status("Choose a source first.");
        };
        let role = self
            .role_combo
            .items()
            .selected_index()
            .and_then(|index| SourceRole::ALL.get(index as usize).copied())
            .unwrap_or(SourceRole::Other);
        let author_entity_id = self
            .author_combo
            .items()
            .selected_index()
            .filter(|index| *index > 0)
            .and_then(|index| {
                self.authors
                    .borrow()
                    .get(index as usize - 1)
                    .map(|entity| entity.id.clone())
            });
        let created = self.created_edit.text()?.trim().to_owned();
        let profile = ProposedSourceProfile {
            id: None,
            source_id: source.source_id.clone(),
            source_role: role,
            author_entity_id,
            created_at_claim: optional_text(&created),
            default_content_form: selected_optional(&self.form_combo, ContentForm::ALL),
            default_temporal_stance: selected_optional(&self.stance_combo, TemporalStance::ALL),
            default_perception_basis: selected_optional(&self.basis_combo, PerceptionBasis::ALL),
            clock_offset_ms: None,
            clock_offset_basis: None,
            review_state: ReviewState::Reviewed,
            created_by: self.actor(),
            supersedes_profile_id: None,
        };
        let outcome = self.workspace.borrow_mut().save_source_profile(&profile);
        match outcome {
            Ok(_) => {
                self.reload_sources()?;
                self.refresh_grid()?;
                self.set_status(
                    "Profile saved. Every passage in this source inherits it until a reading overrides it.",
                )
            }
            Err(error) => self.set_status(&format!("ERROR  {error}")),
        }
    }

    fn open_selected_source(&self) -> w::SysResult<()> {
        let Some(source) = self.selected_source() else {
            return self.set_status("This case has no extracted passages to read yet.");
        };
        let actor = self.actor();
        let outcome = self
            .workspace
            .borrow_mut()
            .open_enrichment(&source.source_id, &actor);
        match outcome {
            Ok(()) => {
                self.refresh_grid()?;
                self.grid.focus()?;
                self.set_status(&format!(
                    "{} open under {actor}. j and k move; one key per row enters a value.",
                    source.name
                ))
            }
            Err(error) => self.set_status(&format!("ERROR  {error}")),
        }
    }

    /// Turns one physical key into a token of the sweep grammar.
    ///
    /// The frontend owns the physical keyboard and the kernel owns the
    /// grammar, so a layout that puts `@` somewhere else changes this function
    /// and nothing else.
    fn on_grid_key(&self, vkey: co::VK) -> w::SysResult<()> {
        let shift = w::GetAsyncKeyState(co::VK::SHIFT);
        let token = match vkey {
            co::VK::UP => Some(if shift { "K" } else { "k" }.to_owned()),
            co::VK::DOWN => Some(if shift { "J" } else { "j" }.to_owned()),
            co::VK::RETURN => Some("Enter".to_owned()),
            co::VK::OEM_PERIOD if !shift => Some(".".to_owned()),
            co::VK::OEM_2 => Some(if shift { "?" } else { "/" }.to_owned()),
            co::VK::CHAR_2 if shift => Some("@".to_owned()),
            other => function_key(other)
                .map(str::to_owned)
                .or_else(|| letter_key(other, shift)),
        };
        let Some(token) = token else {
            return Ok(());
        };
        match token.as_str() {
            "@" => self.enter_entity(),
            "t" => self.enter_time_or_location(),
            "s" => self.enter_span(),
            "?" => self.refresh_keys(),
            _ => self.send_key(&token),
        }
    }

    fn send_key(&self, token: &str) -> w::SysResult<()> {
        let outcome = self.workspace.borrow_mut().enrichment_key(token);
        self.after_command(outcome)
    }

    /// `@`: choose a person the case already knows, or write down a new one.
    fn enter_entity(&self) -> w::SysResult<()> {
        if let Err(error) = self.workspace.borrow_mut().enrichment_key("@") {
            return self.set_status(&format!("ERROR  {error}"));
        }
        let Some(choice) = self.choose_entity()? else {
            return self.set_status("No entity chosen; the passage is unchanged.");
        };
        let outcome = self
            .workspace
            .borrow_mut()
            .enrichment_submit_entity(&choice);
        self.after_command(outcome)
    }

    /// `t`: a time with its sticky basis, or — in the location sweep — the
    /// wording the source itself used.
    fn enter_time_or_location(&self) -> w::SysResult<()> {
        let field = self
            .workspace
            .borrow()
            .enrichment_session()
            .map(|session| session.field);
        if let Err(error) = self.workspace.borrow_mut().enrichment_key("t") {
            return self.set_status(&format!("ERROR  {error}"));
        }
        if field == Some(EnrichmentField::Location) {
            let Some(text) = prompt_text(
                &self.wnd,
                "Location",
                "Type the location exactly as the source words it.",
                "",
            )?
            else {
                return self.set_status("No location entered.");
            };
            let outcome = self
                .workspace
                .borrow_mut()
                .enrichment_submit_location(&text);
            return self.after_command(outcome);
        }
        let Some(entry) = self.prompt_time()? else {
            return self.set_status("No time entered.");
        };
        let outcome = self.workspace.borrow_mut().enrichment_submit_time(&entry);
        self.after_command(outcome)
    }

    /// `s`: mark the part of a passage the reading is really about.
    fn enter_span(&self) -> w::SysResult<()> {
        let text = self
            .workspace
            .borrow()
            .enrichment_session()
            .and_then(|session| session.selected().map(|row| row.text.clone()))
            .unwrap_or_default();
        if let Err(error) = self.workspace.borrow_mut().enrichment_key("s") {
            return self.set_status(&format!("ERROR  {error}"));
        }
        let Some(quoted) = prompt_text(
            &self.wnd,
            "Sub-span",
            "Paste the exact words this reading covers. They must appear in the passage.",
            "",
        )?
        else {
            return self.set_status("No sub-span marked.");
        };
        let Some(start) = text.find(quoted.trim()) else {
            return self
                .set_status("Those words are not in the passage; the sub-span must be verbatim.");
        };
        let start = u32::try_from(start).unwrap_or(0);
        let end = start + u32::try_from(quoted.trim().len()).unwrap_or(0);
        let outcome = self
            .workspace
            .borrow_mut()
            .enrichment_submit_span(start, end);
        self.after_command(outcome)
    }

    fn after_command(&self, outcome: GuiResult<String>) -> w::SysResult<()> {
        match outcome {
            Ok(_) => {
                self.refresh_grid()?;
                self.sync_field_combo();
                self.refresh_keys()?;
                let summary = self.summary();
                self.set_status(&summary)
            }
            Err(error) => self.set_status(&format!("ERROR  {error}")),
        }
    }

    fn summary(&self) -> String {
        let workspace = self.workspace.borrow();
        let Some(session) = workspace.enrichment_session() else {
            return "No source is open.".to_owned();
        };
        let waiting = session
            .rows
            .iter()
            .filter(|row| row.needs(session.field))
            .count();
        format!(
            "{} · passage {} of {} · {waiting} still need this field · {} keystrokes recorded under {}{}",
            session.source_id,
            session.cursor + 1,
            session.rows.len(),
            session.keystrokes,
            session.actor,
            if session.command_mode {
                " · command mode: u undoes"
            } else {
                ""
            }
        )
    }

    fn refresh_grid(&self) -> w::SysResult<()> {
        let (rows, cursor) = {
            let workspace = self.workspace.borrow();
            let rows = workspace.enrichment_rows().unwrap_or_default();
            let cursor = workspace
                .enrichment_session()
                .map_or(0, |session| session.cursor);
            (rows, cursor)
        };
        self.grid.set_redraw(false);
        self.grid.items().delete_all()?;
        for row in &rows {
            self.grid.items().add(&grid_texts(row), None, ())?;
        }
        self.grid.set_redraw(true);
        if let Ok(index) = u32::try_from(cursor)
            && index < self.grid.items().count()
        {
            let item = self.grid.items().get(index);
            item.select(true)?;
            item.focus()?;
            item.ensure_visible()?;
        }
        self.counts_label
            .hwnd()
            .SetWindowText(&Self::counts_text(&rows))?;
        self.refresh_preview()
    }

    fn counts_text(rows: &[EnrichmentRow]) -> String {
        let candidates = rows.iter().filter(|row| row.candidate).count();
        let needing = rows.iter().filter(|row| row.needs).count();
        format!(
            "{} passages · {needing} need this field · {candidates} candidates",
            rows.len()
        )
    }

    fn sync_field_combo(&self) {
        let field = self
            .workspace
            .borrow()
            .enrichment_session()
            .map(|session| session.field);
        if let Some(field) = field
            && let Some(index) = SWEEP_FIELDS
                .iter()
                .position(|(_, candidate, _)| *candidate == field)
        {
            self.field_combo
                .items()
                .select(Some(u32::try_from(index).unwrap_or(0)));
        }
    }

    fn refresh_preview(&self) -> w::SysResult<()> {
        let descriptor = self.workspace.borrow().enrichment_preview().ok();
        let text = describe_preview(descriptor.as_ref());
        *self.preview.borrow_mut() = descriptor;
        self.preview_edit.set_text(&windows_lines(&text))
    }

    fn refresh_keys(&self) -> w::SysResult<()> {
        let field = self
            .workspace
            .borrow()
            .enrichment_session()
            .map_or(EnrichmentField::ContentForm, |session| session.field);
        let mut text = String::new();
        for (key, meaning) in key_sheet(field) {
            let _ = writeln!(text, "{key:<14}{meaning}");
        }
        text.push('\n');
        for (key, meaning) in COMMAND_SHEET {
            let _ = writeln!(text, "{key:<14}{meaning}");
        }
        self.keys_edit.set_text(&windows_lines(&text))
    }

    /// Opens the untouched original, or the retained context beside it.
    fn open_preview_path(&self, context: bool) -> w::SysResult<()> {
        let path = {
            let preview = self.preview.borrow();
            let Some(descriptor) = preview.as_ref() else {
                return self.set_status("No passage is selected.");
            };
            if context {
                context_path(descriptor)
            } else {
                original_path(descriptor)
            }
        };
        let Some(path) = path else {
            return self.set_status(
                "That file is not available. The locator on screen is still the citation.",
            );
        };
        let path = path.to_string_lossy().to_string();
        self.wnd
            .hwnd()
            .ShellExecute("open", &path, None, None, co::SW::SHOWNORMAL)?;
        Ok(())
    }

    /// `@` autocomplete: the case's own names, or a new record for one it does
    /// not have. `possibly the same person` is offered instead of a merge.
    fn choose_entity(&self) -> w::SysResult<Option<String>> {
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: "Choose a person, organization, object, or place",
            size: gui::dpi(560, 400),
            ..Default::default()
        });
        let _find_label = label(&modal, "Find", 20, 24, 34, ANCHOR);
        let find_edit = edit(&modal, 58, 20, 350, ANCHOR);
        let search = button(&modal, "&Find", 420, 18, 116, ANCHOR);
        let list = gui::ComboBox::new(
            &modal,
            gui::ComboBoxOpts {
                position: gui::dpi(20, 62),
                width: gui::dpi_x(516),
                items: &[],
                ..Default::default()
            },
        );
        let _list_hint = label(
            &modal,
            "Names the case already holds. Find narrows the list; the selection is what Use records.",
            20,
            96,
            516,
            ANCHOR,
        );
        let _new_label = label(
            &modal,
            "A name the case does not hold yet — created as its own record, never merged.",
            20,
            270,
            516,
            ANCHOR,
        );
        let new_edit = edit(&modal, 20, 292, 350, ANCHOR);
        let create = button(&modal, "&Create New", 380, 290, 156, ANCHOR);
        let same_person = button(
            &modal,
            "Create, &Possibly Same as Selected",
            20,
            330,
            300,
            ANCHOR,
        );
        let use_selected = button(&modal, "&Use Selected", 330, 330, 116, ANCHOR);
        let cancel = button(&modal, "Cancel", 456, 330, 80, ANCHOR);

        let candidates = Rc::new(RefCell::new(
            self.workspace
                .borrow()
                .entity_candidates("", 200)
                .unwrap_or_default(),
        ));
        let chosen: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let fill = {
            let list = list.clone();
            let candidates = candidates.clone();
            move || -> w::SysResult<()> {
                list.items().delete_all();
                let captions = candidates
                    .borrow()
                    .iter()
                    .map(|entity| format!("{} ({})", entity.display_name, entity.kind))
                    .collect::<Vec<_>>();
                if !captions.is_empty() {
                    list.items().add(&captions)?;
                    list.items().select(Some(0));
                }
                Ok(())
            }
        };
        fill()?;
        let fill = Rc::new(fill);

        let run_find = {
            let workspace = self.workspace.clone();
            let candidates = candidates.clone();
            let find_edit = find_edit.clone();
            let fill = fill.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let prefix = find_edit.text()?;
                *candidates.borrow_mut() = workspace
                    .borrow()
                    .entity_candidates(prefix.trim(), 200)
                    .unwrap_or_default();
                fill()?;
                Ok(())
            })
        };
        let find_click = run_find.clone();
        search.on().bn_clicked(move || find_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(run_find, vec![Field::text(&find_edit)]),
        );

        let candidates_for_use = candidates.clone();
        let list_for_use = list.clone();
        let chosen_for_use = chosen.clone();
        let modal_for_use = modal.clone();
        use_selected.on().bn_clicked(move || {
            if let Some(index) = list_for_use.items().selected_index()
                && let Some(entity) = candidates_for_use.borrow().get(index as usize)
            {
                *chosen_for_use.borrow_mut() = Some(entity.id.clone());
                modal_for_use.close();
            }
            Ok(())
        });

        for (control, link_to_selected) in [(create, false), (same_person, true)] {
            let workspace = self.workspace.clone();
            let candidates = candidates.clone();
            let list = list.clone();
            let new_edit = new_edit.clone();
            let chosen = chosen.clone();
            let modal_close = modal.clone();
            let actor = self.actor();
            control.on().bn_clicked(move || {
                let name = new_edit.text()?.trim().to_owned();
                if name.is_empty() {
                    return Ok(());
                }
                let other = if link_to_selected {
                    list.items().selected_index().and_then(|index| {
                        candidates
                            .borrow()
                            .get(index as usize)
                            .map(|entity| entity.id.clone())
                    })
                } else {
                    None
                };
                let written = workspace.borrow_mut().create_entity(
                    &name,
                    EntityKind::Person,
                    &actor,
                    other.as_deref(),
                );
                if let Ok(entity) = written {
                    *chosen.borrow_mut() = Some(entity.id);
                    modal_close.close();
                }
                Ok(())
            });
        }

        let modal_cancel = modal.clone();
        cancel.on().bn_clicked(move || {
            modal_cancel.close();
            Ok(())
        });
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;
        let picked = chosen.borrow().clone();
        Ok(picked)
    }

    /// `t`: the expression, whether it is the source's own claim or the
    /// reviewer's alignment, and the basis that alignment rests on.
    fn prompt_time(&self) -> w::SysResult<Option<crate::TimeEntry>> {
        let sticky = self
            .workspace
            .borrow()
            .enrichment_session()
            .and_then(|session| session.sticky_time_basis.clone())
            .unwrap_or_default();
        let modal = gui::WindowModal::new(gui::WindowModalOpts {
            title: "Time",
            size: gui::dpi(560, 260),
            ..Default::default()
        });
        let _value_label = label(
            &modal,
            "22:42 · 2024-03-02 22:42 · +57m from the last anchor",
            20,
            22,
            420,
            ANCHOR,
        );
        let value_edit = edit(&modal, 20, 46, 380, ANCHOR);
        let asserted = gui::CheckBox::new(
            &modal,
            gui::CheckBoxOpts {
                text: "The source &asserts this time",
                position: gui::dpi(20, 84),
                ..Default::default()
            },
        );
        let approximate = gui::CheckBox::new(
            &modal,
            gui::CheckBoxOpts {
                text: "A&pproximate",
                position: gui::dpi(300, 84),
                ..Default::default()
            },
        );
        let _basis_label = label(
            &modal,
            "Alignment basis — required for a normalized case time, kept for the rest of the sweep",
            20,
            116,
            520,
            ANCHOR,
        );
        let basis_edit = gui::Edit::new(
            &modal,
            gui::EditOpts {
                text: &sticky,
                position: gui::dpi(20, 140),
                width: gui::dpi_x(516),
                ..Default::default()
            },
        );
        let accept = button(&modal, "&Record", 300, 196, 116, ANCHOR);
        let cancel = button(&modal, "Cancel", 424, 196, 112, ANCHOR);
        let entry: Rc<RefCell<Option<crate::TimeEntry>>> = Rc::new(RefCell::new(None));

        let record = {
            let entry = entry.clone();
            let value_edit = value_edit.clone();
            let basis_edit = basis_edit.clone();
            let asserted = asserted.clone();
            let approximate = approximate.clone();
            let modal = modal.clone();
            Rc::new(move || -> w::AnyResult<()> {
                let value = value_edit.text()?.trim().to_owned();
                if value.is_empty() {
                    return Ok(());
                }
                *entry.borrow_mut() = Some(crate::TimeEntry {
                    value,
                    asserted: asserted.is_checked(),
                    basis: optional_text(basis_edit.text()?.trim()),
                    approximate: approximate.is_checked(),
                });
                modal.close();
                Ok(())
            })
        };
        let record_click = record.clone();
        accept.on().bn_clicked(move || record_click());
        modal.on().wm_command(
            co::DLGID::OK,
            co::CMD::Menu,
            enter_submits(
                record,
                vec![Field::text(&value_edit), Field::text(&basis_edit)],
            ),
        );
        let modal_cancel = modal.clone();
        cancel.on().bn_clicked(move || {
            modal_cancel.close();
            Ok(())
        });
        modal
            .show_modal(&self.wnd)
            .map_err(|_| co::ERROR::INVALID_DATA)?;
        let recorded = entry.borrow().clone();
        Ok(recorded)
    }

    fn set_status(&self, text: &str) -> w::SysResult<()> {
        self.status_label.hwnd().SetWindowText(text)
    }
}

/// A dropdown list, quick-selected by typing an item's first letter — which
/// is why every vocabulary shown through one keeps its stored words.
fn choice(
    parent: &(impl GuiParent + 'static),
    x: i32,
    y: i32,
    width: i32,
    items: &[&str],
    selected: u32,
) -> gui::ComboBox {
    gui::ComboBox::new(
        parent,
        gui::ComboBoxOpts {
            position: gui::dpi(x, y),
            width: gui::dpi_x(width),
            items,
            selected_item: Some(selected),
            ..Default::default()
        },
    )
}

/// A [`choice`] over a stored vocabulary's own words.
fn vocabulary<const N: usize>(
    parent: &(impl GuiParent + 'static),
    x: i32,
    y: i32,
    width: i32,
    items: &[&str; N],
    selected: u32,
) -> gui::ComboBox {
    choice(parent, x, y, width, items, selected)
}

/// Accept and Cancel, in the corner every entry dialog puts them.
fn modal_buttons(modal: &gui::WindowModal, accept: &str, y: i32) -> (gui::Button, gui::Button) {
    let accept = button(modal, accept, 296, y, 132, ANCHOR);
    let cancel = button(modal, "Cancel", 436, y, 104, ANCHOR);
    (accept, cancel)
}

/// Wires a Cancel button to close its dialog and record nothing.
fn wire_cancel(modal: &gui::WindowModal, cancel: &gui::Button) {
    let modal = modal.clone();
    cancel.on().bn_clicked(move || {
        modal.close();
        Ok(())
    });
}

/// Court chooser rows: none, every court, and a row that records a new one.
fn court_rows(courts: &[(String, String)]) -> Vec<String> {
    let mut rows = vec!["(none)".to_owned()];
    rows.extend(courts.iter().map(|(_, name)| name.clone()));
    rows.push("New court…".to_owned());
    rows
}

/// Court chooser rows without the recording row, where a new court would be a
/// digression rather than a step.
fn court_rows_plain(courts: &[(String, String)]) -> Vec<String> {
    let mut rows = vec!["(none)".to_owned()];
    rows.extend(courts.iter().map(|(_, name)| name.clone()));
    rows
}

/// A control Enter may submit from.
///
/// Held as the control rather than its window handle, because copying a raw
/// `HWND` needs `unsafe` and the crate forbids it; the controls are `Clone`.
#[derive(Clone)]
enum Field {
    /// A single-line (or submitting multiline) text box.
    Text(gui::Edit),
    /// A dropdown list.
    Choice(gui::ComboBox),
    /// A list box, including a multi-select one.
    List(gui::ListBox),
}

impl Field {
    fn text(control: &gui::Edit) -> Self {
        Self::Text(control.clone())
    }

    fn choice(control: &gui::ComboBox) -> Self {
        Self::Choice(control.clone())
    }

    fn list(control: &gui::ListBox) -> Self {
        Self::List(control.clone())
    }

    fn holds_focus(&self, focused: Option<&w::HWND>) -> bool {
        let own = match self {
            Self::Text(control) => control.hwnd(),
            Self::Choice(control) => control.hwnd(),
            Self::List(control) => control.hwnd(),
        };
        focused == Some(own)
    }
}

/// Enter as submit. `IsDialogMessage` turns an Enter no control claims into
/// `IDOK`; the returned handler accepts it only while focus is in one of the
/// named fields, so Enter over a Cancel button records nothing.
fn enter_submits(
    submit: Rc<dyn Fn() -> w::AnyResult<()>>,
    fields: Vec<Field>,
) -> impl Fn() -> w::AnyResult<()> {
    move || {
        let focused = w::HWND::GetFocus();
        if fields
            .iter()
            .any(|field| field.holds_focus(focused.as_ref()))
        {
            submit()?;
        }
        Ok(())
    }
}

/// One free-text answer, used where a value has no controlled vocabulary.
fn prompt_text(
    parent: &gui::WindowModal,
    title: &str,
    guidance: &str,
    initial: &str,
) -> w::SysResult<Option<String>> {
    let modal = gui::WindowModal::new(gui::WindowModalOpts {
        title,
        size: gui::dpi(560, 200),
        ..Default::default()
    });
    let _guidance = label(&modal, guidance, 20, 22, 516, ANCHOR);
    let value = gui::Edit::new(
        &modal,
        gui::EditOpts {
            text: initial,
            position: gui::dpi(20, 56),
            width: gui::dpi_x(516),
            ..Default::default()
        },
    );
    let accept = button(&modal, "&Record", 300, 130, 116, ANCHOR);
    let cancel = button(&modal, "Cancel", 424, 130, 112, ANCHOR);
    let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let record = {
        let answer = answer.clone();
        let value = value.clone();
        let modal = modal.clone();
        Rc::new(move || -> w::AnyResult<()> {
            let text = value.text()?.trim().to_owned();
            if !text.is_empty() {
                *answer.borrow_mut() = Some(text);
            }
            modal.close();
            Ok(())
        })
    };
    let record_click = record.clone();
    accept.on().bn_clicked(move || record_click());
    modal.on().wm_command(
        co::DLGID::OK,
        co::CMD::Menu,
        enter_submits(record, vec![Field::text(&value)]),
    );
    let modal_cancel = modal.clone();
    cancel.on().bn_clicked(move || {
        modal_cancel.close();
        Ok(())
    });
    modal
        .show_modal(parent)
        .map_err(|_| co::ERROR::INVALID_DATA)?;
    let recorded = answer.borrow().clone();
    Ok(recorded)
}

/// `F2`…`F8`, which is how a sweep changes column.
fn function_key(vkey: co::VK) -> Option<&'static str> {
    match vkey {
        co::VK::F2 => Some("F2"),
        co::VK::F3 => Some("F3"),
        co::VK::F4 => Some("F4"),
        co::VK::F5 => Some("F5"),
        co::VK::F6 => Some("F6"),
        co::VK::F7 => Some("F7"),
        co::VK::F8 => Some("F8"),
        _ => None,
    }
}

/// A letter, in the case the reviewer typed it: `Shift` is the range apply.
fn letter_key(vkey: co::VK, shift: bool) -> Option<String> {
    let raw = u16::from(vkey);
    if !(0x41..=0x5a).contains(&raw) {
        return None;
    }
    let letter = char::from(u8::try_from(raw).ok()?);
    Some(if shift {
        letter.to_string()
    } else {
        letter.to_ascii_lowercase().to_string()
    })
}

/// A letter of the alphabet, for the `Ctrl+Shift` chords the pane switch and
/// the Court commands carry.
///
/// Spelled out rather than computed from the letter's ASCII code, because
/// `co::VK` can only be built from a raw number through an `unsafe` call, and
/// `unsafe_code = "forbid"` rules that out. The companion `digit_key` does the
/// same thing for the view accelerators, for the same reason.
fn chord_key(letter: char) -> co::VK {
    match letter.to_ascii_uppercase() {
        'B' => co::VK::CHAR_B,
        'C' => co::VK::CHAR_C,
        'D' => co::VK::CHAR_D,
        'E' => co::VK::CHAR_E,
        'F' => co::VK::CHAR_F,
        'G' => co::VK::CHAR_G,
        'H' => co::VK::CHAR_H,
        'I' => co::VK::CHAR_I,
        'J' => co::VK::CHAR_J,
        'K' => co::VK::CHAR_K,
        'L' => co::VK::CHAR_L,
        'M' => co::VK::CHAR_M,
        'N' => co::VK::CHAR_N,
        'O' => co::VK::CHAR_O,
        'P' => co::VK::CHAR_P,
        'Q' => co::VK::CHAR_Q,
        'R' => co::VK::CHAR_R,
        'S' => co::VK::CHAR_S,
        'T' => co::VK::CHAR_T,
        'U' => co::VK::CHAR_U,
        'V' => co::VK::CHAR_V,
        'W' => co::VK::CHAR_W,
        'X' => co::VK::CHAR_X,
        'Y' => co::VK::CHAR_Y,
        'Z' => co::VK::CHAR_Z,
        // 'A' among them: the table is exhaustive over the alphabet, and a
        // caller passing anything else is a mistake in this file rather than
        // something a user can reach.
        _ => co::VK::CHAR_A,
    }
}

/// The digit row, for the `Ctrl` view accelerators.
fn digit_key(digit: char) -> co::VK {
    match digit {
        '0' => co::VK::CHAR_0,
        '2' => co::VK::CHAR_2,
        '3' => co::VK::CHAR_3,
        '4' => co::VK::CHAR_4,
        '5' => co::VK::CHAR_5,
        '6' => co::VK::CHAR_6,
        '7' => co::VK::CHAR_7,
        '8' => co::VK::CHAR_8,
        '9' => co::VK::CHAR_9,
        _ => co::VK::CHAR_1,
    }
}

/// One docket row as its grid cells.
fn docket_texts(row: &DocketGridRow) -> [String; 11] {
    [
        row.time.clone(),
        row.what_for.clone(),
        row.court.clone(),
        row.client.clone(),
        row.matters.clone(),
        row.charges.clone(),
        row.custody.clone(),
        row.offer.clone(),
        row.last_contact.clone(),
        row.posture.clone(),
        row.open_work.clone(),
    ]
}

/// One owed thing as its grid cells.
fn deadline_texts(row: &DeadlineGridRow) -> [String; 6] {
    [
        row.due.clone(),
        row.within.clone(),
        row.matter.clone(),
        row.client.clone(),
        row.what.clone(),
        row.origin.clone(),
    ]
}

fn grid_texts(row: &EnrichmentRow) -> [String; 6] {
    [
        row.number.to_string(),
        row.locator.clone(),
        row.passage.clone(),
        row.value.clone(),
        row.provenance.clone(),
        row.badge.clone(),
    ]
}

/// The preview in words: what the passage points at, and whether the file
/// backing it still matches what was imported.
fn describe_preview(descriptor: Option<&PreviewDescriptor>) -> String {
    let Some(descriptor) = descriptor else {
        return "No passage is selected.".to_owned();
    };
    let mut text = String::new();
    match descriptor {
        PreviewDescriptor::Document {
            original_path,
            page,
            bounding_box,
            page_image,
            ..
        } => {
            let _ = writeln!(text, "Document · page {page}");
            if let Some(region) = bounding_box {
                let _ = writeln!(
                    text,
                    "Region on the page: {:.3}, {:.3} to {:.3}, {:.3}",
                    region[0], region[1], region[2], region[3]
                );
            }
            write_path(&mut text, "Original", original_path.as_deref());
            write_path(&mut text, "Retained page image", page_image.as_deref());
        }
        PreviewDescriptor::Audio {
            original_path,
            start_ms,
            end_ms,
            waveform,
            ..
        } => {
            let _ = writeln!(text, "Audio · {} to {}", clock(*start_ms), clock(*end_ms));
            write_path(&mut text, "Original", original_path.as_deref());
            write_path(&mut text, "Retained waveform", waveform.as_deref());
        }
        PreviewDescriptor::Video {
            original_path,
            start_ms,
            end_ms,
            frames,
            ..
        } => {
            let _ = writeln!(
                text,
                "Video · {} to {} · {} retained still(s)",
                clock(*start_ms),
                clock(*end_ms),
                frames.len()
            );
            write_path(&mut text, "Original", original_path.as_deref());
            write_path(
                &mut text,
                "First still",
                frames.first().map(PathBuf::as_path),
            );
        }
        PreviewDescriptor::Unavailable { locator, reason } => {
            let _ = writeln!(text, "The original cannot be opened from here.");
            let _ = writeln!(text, "Locator: {locator}");
            let _ = writeln!(text, "Why: {reason}");
        }
    }
    if descriptor.verified_context() {
        text.push_str("The file still matches the hash and length recorded at intake.\n");
    } else {
        text.push_str(
            "Unverified context: a `verified` decision still needs the original itself.\n",
        );
    }
    text
}

fn write_path(text: &mut String, caption: &str, path: Option<&Path>) {
    if let Some(path) = path {
        let _ = writeln!(text, "{caption}: {}", path.display());
    }
}

fn clock(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60,
        milliseconds % 1_000
    )
}

fn original_path(descriptor: &PreviewDescriptor) -> Option<PathBuf> {
    match descriptor {
        PreviewDescriptor::Document { original_path, .. }
        | PreviewDescriptor::Audio { original_path, .. }
        | PreviewDescriptor::Video { original_path, .. } => original_path.clone(),
        PreviewDescriptor::Unavailable { .. } => None,
    }
}

fn context_path(descriptor: &PreviewDescriptor) -> Option<PathBuf> {
    match descriptor {
        PreviewDescriptor::Document { page_image, .. } => page_image.clone(),
        PreviewDescriptor::Audio { waveform, .. } => waveform.clone(),
        PreviewDescriptor::Video { frames, .. } => frames.first().cloned(),
        PreviewDescriptor::Unavailable { .. } => None,
    }
}

/// Reads an optional controlled value out of a combo whose first row is "none".
fn selected_optional<T: Copy>(combo: &gui::ComboBox, all: &'static [T]) -> Option<T> {
    combo
        .items()
        .selected_index()
        .filter(|index| *index > 0)
        .and_then(|index| all.get(index as usize - 1).copied())
}

/// Selects the row of a "none"-headed combo that holds one optional value.
fn select_optional<T: Copy + PartialEq>(
    combo: &gui::ComboBox,
    all: &'static [T],
    value: Option<T>,
) {
    let index = value
        .and_then(|value| all.iter().position(|candidate| *candidate == value))
        .map_or(0, |index| index + 1);
    combo
        .items()
        .select(Some(u32::try_from(index).unwrap_or(0)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command the window offers, whether or not it claims a letter.
    fn commands() -> Vec<&'static str> {
        VIEW_BUTTONS
            .iter()
            .map(|(caption, _)| *caption)
            .chain(PANE_BUTTONS.iter().map(|(pane, _)| pane.label()))
            .chain(COURT_COMMANDS.iter().map(|(caption, _, _)| *caption))
            .chain(OFFICE_COMMANDS.iter().map(|(caption, _, _)| *caption))
            .chain([
                OPEN_DATABASE,
                NEW_CASE,
                INTAKE_EVIDENCE,
                PROCESSING_QUEUE,
                ENRICHMENT_SWEEP,
                RUN_COLLATION,
                DISCLOSABLE_EXPORT,
                WORK_FILE_EXPORT,
                SAVE_EXPORT,
                FIND,
                MARK_REVIEWED,
                VERIFY,
                REJECT,
                NEW_FROM_TEMPLATE,
                SAVE_AUTHORED,
                IMPORT_BATCH,
            ])
            .collect()
    }

    /// The letter a caption claims for `Alt`, if it claims one. A doubled `&&`
    /// is an escaped ampersand and claims nothing.
    fn mnemonic(caption: &str) -> Option<char> {
        let mut characters = caption.chars().peekable();
        while let Some(character) = characters.next() {
            if character != '&' {
                continue;
            }
            match characters.peek() {
                Some('&') => {
                    characters.next();
                }
                Some(next) => return Some(next.to_ascii_uppercase()),
                None => return None,
            }
        }
        None
    }

    /// Win32 resolves a mnemonic across the whole window: two buttons offering
    /// the same letter make `Alt` cycle focus between them instead of pressing
    /// either, which silently costs the keyboard path through the workspace.
    #[test]
    fn every_alt_key_is_claimed_once() {
        let mut claimed: Vec<(char, &str)> = Vec::new();
        for caption in commands() {
            let Some(letter) = mnemonic(caption) else {
                continue;
            };
            if let Some((_, other)) = claimed.iter().find(|(taken, _)| *taken == letter) {
                panic!("Alt+{letter} is claimed by both {other:?} and {caption:?}");
            }
            claimed.push((letter, caption));
        }
    }

    /// The Alt namespace ran out before the views did, so a view that carries
    /// no letter is reached by a `Ctrl` chord instead. Every view has exactly
    /// one chord, no chord is shared, and no command is unreachable.
    #[test]
    fn every_command_has_its_own_accelerator() {
        let mut chords: Vec<(String, &str)> = Vec::new();
        for (index, (caption, _)) in VIEW_BUTTONS.iter().enumerate() {
            let (shift, key) = view_accelerator(index);
            let chord = accelerator_label(shift, key);
            if let Some((_, other)) = chords.iter().find(|(taken, _)| *taken == chord) {
                panic!("{chord} is claimed by both {other:?} and {caption:?}");
            }
            chords.push((chord, caption));
        }
        assert_eq!(chords.len(), VIEW_BUTTONS.len());

        // The pane switch and the Court commands take `Ctrl+Shift` letters,
        // because the Alt namespace has nothing left to give them. They share
        // that space with the last two views, which take `Ctrl+Shift` digits.
        for (caption, key) in PANE_BUTTONS
            .iter()
            .map(|(pane, key)| (pane.label(), *key))
            .chain(
                COURT_COMMANDS
                    .iter()
                    .map(|(caption, key, _)| (*caption, *key)),
            )
            .chain(
                OFFICE_COMMANDS
                    .iter()
                    .map(|(caption, key, _)| (*caption, *key)),
            )
        {
            let chord = accelerator_label(true, key);
            if let Some((_, other)) = chords.iter().find(|(taken, _)| *taken == chord) {
                panic!("{chord} is claimed by both {other:?} and {caption:?}");
            }
            chords.push((chord, caption));
        }

        for caption in commands() {
            let has_letter = mnemonic(caption).is_some();
            let has_chord = chords.iter().any(|(_, claimed)| *claimed == caption);
            assert!(
                has_letter || has_chord,
                "{caption:?} offers neither an Alt key nor a Ctrl chord"
            );
        }
    }

    /// A `Ctrl+Shift` chord has to reach a real virtual key, or the
    /// accelerator table silently binds every unrecognized letter to the same
    /// one and the second command added is unreachable.
    #[test]
    fn every_chord_reaches_its_own_virtual_key() {
        let mut keys: Vec<(co::VK, &str)> = Vec::new();
        for (caption, letter) in PANE_BUTTONS
            .iter()
            .map(|(pane, key)| (pane.label(), *key))
            .chain(
                COURT_COMMANDS
                    .iter()
                    .map(|(caption, key, _)| (*caption, *key)),
            )
            .chain(
                OFFICE_COMMANDS
                    .iter()
                    .map(|(caption, key, _)| (*caption, *key)),
            )
        {
            let key = chord_key(letter);
            assert_eq!(
                letter_key(key, true).as_deref(),
                Some(letter.to_string().as_str()),
                "{caption:?} asked for Ctrl+Shift+{letter} and got something else"
            );
            if let Some((_, other)) = keys.iter().find(|(taken, _)| *taken == key) {
                panic!("Ctrl+Shift+{letter} is claimed by both {other:?} and {caption:?}");
            }
            keys.push((key, caption));
        }
    }

    /// Both panes are on the switch, and every Court command has a button.
    #[test]
    fn both_panes_and_every_court_command_have_a_control() {
        for pane in Pane::ALL {
            assert!(
                PANE_BUTTONS.iter().any(|(candidate, _)| *candidate == pane),
                "{} has no way to reach it",
                pane.label()
            );
        }
        assert_eq!(PANE_BUTTONS.len(), Pane::ALL.len());

        // The Court pane has no rail, so nothing anchors its layout: the two
        // grids and the detail box stack, and at the design size each has to
        // clear the caption of the one below it.
        const {
            assert!(
                COURT_GRID_TOP + COURT_GRID_HEIGHT < COURT_DEADLINES_TOP - 20,
                "the docket grid runs into the deadlines caption"
            );
            assert!(
                COURT_DEADLINES_TOP + COURT_DEADLINES_HEIGHT < COURT_DETAIL_TOP - 20,
                "the deadlines grid runs into the detail caption"
            );
            assert!(
                COURT_DETAIL_TOP + COURT_DETAIL_HEIGHT < COURT_STATUS_TOP,
                "the detail box runs into the status line"
            );
            assert!(
                COURT_STATUS_TOP + 20 <= HEIGHT,
                "the status line starts below the window"
            );
        }

        // And at the floor, where everything riding the bottom edge has moved
        // up by the difference and the docket grid has given up that height.
        const {
            let shrink = HEIGHT - MIN_HEIGHT;
            assert!(
                COURT_GRID_HEIGHT - shrink > 60,
                "the docket collapses to nothing before the window stops shrinking"
            );
            assert!(
                COURT_STATUS_TOP - shrink + 20 <= MIN_HEIGHT,
                "the Court status line falls off the bottom at the minimum height"
            );
        }
    }

    /// Every office entry command is on the table exactly once, so it has a
    /// button, hides with the Office pane, and answers to its chord — the same
    /// guarantee the Court pane's own table test gives.
    #[test]
    fn every_office_command_has_a_control() {
        for command in OfficeCommand::ALL {
            assert_eq!(
                OFFICE_COMMANDS
                    .iter()
                    .filter(|(_, _, candidate)| *candidate == command)
                    .count(),
                1,
                "{command:?} must appear exactly once on the entry row"
            );
        }
        assert_eq!(OFFICE_COMMANDS.len(), OfficeCommand::ALL.len());
    }

    /// The Office pane's output box, entry row and status line stack without
    /// touching, at the design size and at the floor.
    #[test]
    fn the_office_pane_stacks_without_collisions() {
        const {
            assert!(
                OFFICE_OUTPUT_TOP + OFFICE_OUTPUT_HEIGHT < OFFICE_ENTRY_TOP,
                "the output box runs into the entry row"
            );
            assert!(
                OFFICE_ENTRY_TOP + BUTTON_HEIGHT <= OFFICE_STATUS_TOP,
                "the entry row runs into the status line"
            );
        }
        const {
            let shrink = HEIGHT - MIN_HEIGHT;
            assert!(
                OFFICE_OUTPUT_HEIGHT - shrink > 60,
                "the output box collapses before the window stops shrinking"
            );
            assert!(
                OFFICE_STATUS_TOP - shrink + 20 <= MIN_HEIGHT,
                "the status line falls off the bottom at the minimum height"
            );
        }
    }

    /// Every workspace view the kernel offers has a control. A read model with
    /// no way to reach it is a read model nobody uses.
    #[test]
    fn every_workspace_view_is_on_the_rail() {
        for view in WorkspaceView::ALL {
            assert!(
                VIEW_BUTTONS.iter().any(|(_, candidate)| *candidate == view),
                "{} has no navigation control",
                view.label()
            );
        }
        assert_eq!(VIEW_BUTTONS.len(), WorkspaceView::ALL.len());
    }

    /// The rail is anchored and does not move, so everything on it has to fit
    /// inside the smallest client area the window will accept.
    #[test]
    fn the_rail_fits_above_the_minimum_height() {
        let last_view_row = i32::try_from(VIEW_BUTTONS.len() - 1).expect("a small rail");
        let last_view_bottom = VIEW_RAIL_TOP + last_view_row * VIEW_RAIL_PITCH + VIEW_BUTTON_HEIGHT;
        assert!(
            last_view_bottom < RAIL_RULES[1],
            "view controls end at {last_view_bottom}, colliding with the rule at {}",
            RAIL_RULES[1]
        );
        assert!(
            RAIL_RULES[1] < ACTION_RAIL_TOP,
            "the group rule must remain above the first action"
        );

        let intake_bottom = INTAKE_RAIL_TOP + 2 * INTAKE_RAIL_PITCH + BUTTON_HEIGHT;
        assert!(
            intake_bottom < RAIL_RULES[0],
            "intake commands end at {intake_bottom}, colliding with the rule at {}",
            RAIL_RULES[0]
        );
        assert!(
            RAIL_RULES[0] < VIEW_RAIL_TOP,
            "the first rule must remain above the view rail"
        );

        let last_action_bottom = ACTION_RAIL_TOP + 3 * ACTION_RAIL_PITCH + BUTTON_HEIGHT;
        assert!(
            last_action_bottom <= MIN_HEIGHT,
            "the export commands end at {last_action_bottom}, below the {MIN_HEIGHT} floor"
        );
    }

    /// Each sweep column is reachable by exactly one function key, and the
    /// tokens are the ones the platform-neutral grammar answers to.
    #[test]
    fn every_sweep_field_has_its_own_function_key() {
        let mut seen: Vec<&str> = Vec::new();
        for (_, field, token) in SWEEP_FIELDS {
            assert!(!seen.contains(&token), "{token} selects two sweep fields");
            seen.push(token);
            let key = match token {
                "F2" => co::VK::F2,
                "F3" => co::VK::F3,
                "F4" => co::VK::F4,
                "F5" => co::VK::F5,
                "F6" => co::VK::F6,
                "F7" => co::VK::F7,
                _ => co::VK::F8,
            };
            assert_eq!(function_key(key), Some(token));
            assert!(
                SWEEP_FIELDS
                    .iter()
                    .filter(|(_, candidate, _)| *candidate == field)
                    .count()
                    == 1,
                "{field:?} appears twice in the sweep table"
            );
        }
    }

    /// A letter reaches the grammar in the case it was typed, because `Shift`
    /// is the range apply and not a different value.
    #[test]
    fn shift_reaches_the_grammar_as_an_uppercase_letter() {
        assert_eq!(letter_key(co::VK::CHAR_Q, false).as_deref(), Some("q"));
        assert_eq!(letter_key(co::VK::CHAR_Q, true).as_deref(), Some("Q"));
        assert_eq!(letter_key(co::VK::F2, false), None);
        assert_eq!(letter_key(co::VK::CHAR_1, false), None);
    }

    /// The sweep window's rows are laid out from its bottom edge, so they have
    /// to stay inside the floor it will not shrink past.
    #[test]
    fn the_sweep_window_fits_its_own_floor() {
        let columns = GRID_COLUMNS.iter().map(|(_, width)| width).sum::<i32>();
        assert!(
            columns <= SWEEP_WIDTH - 40,
            "the grid's {columns} units of columns do not fit the window it was drawn for"
        );
        // Narrowed to the floor the passage column is the one that gives way.
        // Everything a decision needs — the locator, the value, where the
        // value came from, and the badge — stays on screen without scrolling.
        let decided = columns - GRID_COLUMNS[2].1;
        assert!(
            decided <= SWEEP_MIN_WIDTH - 40,
            "at the floor the decision columns still need {decided} units"
        );
    }
}
