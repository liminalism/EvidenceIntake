//! `WinSafe` adapter for the platform-neutral evidence workspace.

use std::cell::RefCell;
use std::collections::HashMap;
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

use super::{AuthorKind, GuiError, GuiResult, Workspace, WorkspaceView, review_target};
use crate::{ExportAudience, IntakeCoordinator, IntakeJobState, ReviewState};

/// Design size of the client area, in logical units. `gui::dpi` scales every
/// coordinate below by the system DPI, so at 200% scaling this window wants
/// 2200x1400 device pixels — more than most screens have. `fit_to_work_area`
/// is what keeps it reachable.
const WIDTH: i32 = 1100;
const HEIGHT: i32 = 700;

/// Smallest logical client area the layout survives. Below `MIN_WIDTH` the
/// right-anchored authoring pair collides with `Reject` on the bottom row.
/// `MIN_HEIGHT` is set by the rail rather than by the output pane: the rail is
/// anchored and does not move, so its last button's lower edge — 578 — is the
/// floor, and anything less hides the export commands off the bottom.
const MIN_WIDTH: i32 = 940;
const MIN_HEIGHT: i32 = 600;

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
/// Read-view controls are slightly denser so ten views fit above the action rail.
const VIEW_BUTTON_HEIGHT: i32 = 25;
const VIEW_RAIL_TOP: i32 = 162;
const VIEW_RAIL_PITCH: i32 = 28;
const ACTION_RAIL_TOP: i32 = 454;

/// Rail rules. The rules are the only thing that groups the rail's commands —
/// a heading over each group was tried and read as clutter.
const RAIL_RULES: [i32; 2] = [150, 442];

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
const NEW_CASE: &str = "&Untitled Case";
const INTAKE_EVIDENCE: &str = "Intake E&vidence";
const PROCESSING_QUEUE: &str = "P&rocessing Queue";
const RUN_COLLATION: &str = "Run Co&llation";
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
const VIEW_BUTTONS: [(&str, WorkspaceView); 10] = [
    ("&Overview", WorkspaceView::Overview),
    ("Case &Standing", WorkspaceView::Standing),
    ("&Discovery Ledger", WorkspaceView::Discovery),
    ("Element &Matrix", WorkspaceView::Elements),
    ("Contested &Timeline", WorkspaceView::Timeline),
    ("Collation &Groups", WorkspaceView::Collation),
    ("&Issue Workspaces", WorkspaceView::Issues),
    ("Offense &Comparison", WorkspaceView::Offenses),
    ("Review &Queue", WorkspaceView::ReviewQueue),
    ("Review &History", WorkspaceView::ReviewHistory),
];

const STATUS_HINT: &str = "Alt + the underlined letter runs a command. All derived material must be checked against the original.";

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
    _labels: Vec<gui::Label>,
    actor_label: gui::Label,
    database_edit: gui::Edit,
    open_button: gui::Button,
    case_combo: gui::ComboBox,
    new_case_button: gui::Button,
    intake_button: gui::Button,
    queue_button: gui::Button,
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

        let wnd = gui::WindowMain::new(gui::WindowMainOpts {
            title: "Evidence Intake — Local Case Workspace",
            size: gui::dpi(WIDTH, HEIGHT),
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

        // --- Header: which database, which case ------------------------------
        let mut labels = vec![
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

        labels.push(label(&wnd, "Case", 674, 41, 34, SLIDE_X));
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

        // --- Rail group 1: put evidence in the workspace ---------------------
        let intake_button = button(&wnd, INTAKE_EVIDENCE, RAIL_X, 78, RAIL_WIDTH, ANCHOR);
        let queue_button = button(&wnd, PROCESSING_QUEUE, RAIL_X, 110, RAIL_WIDTH, ANCHOR);

        // --- Rail group 2: read the case -------------------------------------
        let view_buttons = VIEW_BUTTONS
            .iter()
            .enumerate()
            .map(|(index, (caption, view))| {
                let row = i32::try_from(index).expect("navigation has only ten rows");
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
            ACTION_RAIL_TOP + 32,
            RAIL_WIDTH,
            ANCHOR,
        );
        let work_export_button = button(
            &wnd,
            WORK_FILE_EXPORT,
            RAIL_X,
            ACTION_RAIL_TOP + 64,
            RAIL_WIDTH,
            ANCHOR,
        );
        let persist_export_button = button(
            &wnd,
            SAVE_EXPORT,
            RAIL_X,
            ACTION_RAIL_TOP + 96,
            RAIL_WIDTH,
            ANCHOR,
        );

        // --- Pane: search over originals -------------------------------------
        let search_mode = gui::ComboBox::new(
            &wnd,
            gui::ComboBoxOpts {
                position: gui::dpi(PANE_X, 78),
                width: gui::dpi_x(140),
                items: &["Text", "Video Frames"],
                selected_item: Some(0),
                ..Default::default()
            },
        );
        let search_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: "",
                position: gui::dpi(364, 81),
                width: gui::dpi_x(588),
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
                position: gui::dpi(PANE_X, 118),
                width: gui::dpi_x(PANE_WIDTH),
                height: gui::dpi_y(320),
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
            446,
            PANE_WIDTH,
            STRETCH_X_SLIDE_Y,
        );

        // --- Pane: the human review and authoring panel ----------------------
        // The rule above this panel is anchored to the first field label, so it
        // follows the panel when the window grows.
        let actor_label = label(&wnd, "Actor", PANE_X, 482, 60, SLIDE_Y);
        labels.extend([
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
        labels.extend([
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

        let new_self = Self {
            wnd,
            workspace,
            coordinator,
            _labels: labels,
            actor_label,
            database_edit,
            open_button,
            case_combo,
            new_case_button,
            intake_button,
            queue_button,
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
        let mut all = vec![
            &self.open_button,
            &self.new_case_button,
            &self.intake_button,
            &self.queue_button,
        ];
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

    fn events(&self) {
        let me = self.clone();
        self.wnd.on().wm_create(move |_| {
            me.fit_to_work_area()?;
            me.show_view(WorkspaceView::Overview)?;
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
            let query = me.search_edit.text()?;
            let mode = me
                .search_mode
                .items()
                .selected_text()?
                .unwrap_or_else(|| "Text".to_owned());
            let result = if mode == "Video Frames" {
                me.search_video_frames(query.trim())
            } else {
                me.workspace.borrow().search(query.trim(), 100)
            };
            me.present(
                result,
                if mode == "Video Frames" {
                    "Frame finder results"
                } else {
                    "Search results"
                },
            )?;
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
                let result = me.workspace.borrow().export(audience);
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
        serde_json::to_string_pretty(&rendered).map_err(GuiError::from)
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
        let hwnd = self.wnd.hwnd();
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

        // Never clamp below the minimum: on a screen too small for the layout
        // even at its floor, a window that overflows can still be moved to
        // reach the rest of it, whereas one shrunk past the floor has commands
        // that overlap and cannot be separated again.
        let floor = self.frame_margins().unwrap_or((0, 0));
        let size = (
            wanted
                .0
                .min(available.0)
                .max(gui::dpi_x(MIN_WIDTH) + floor.0),
            wanted
                .1
                .min(available.1)
                .max(gui::dpi_y(MIN_HEIGHT) + floor.1),
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

    /// Device pixels the frame adds around the client area, or `None` before
    /// the window has one — `WM_GETMINMAXINFO` arrives during creation, when
    /// the client area can still be empty.
    fn frame_margins(&self) -> Option<(i32, i32)> {
        let hwnd = self.wnd.hwnd();
        let window = hwnd.GetWindowRect().ok()?;
        let client = hwnd.GetClientRect().ok()?;
        (client.right > 0 && client.bottom > 0).then(|| {
            (
                (window.right - window.left) - client.right,
                (window.bottom - window.top) - client.bottom,
            )
        })
    }

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

        let buttons = self
            .buttons()
            .into_iter()
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

        // The review panel slides with the window, so anchor its rule to the
        // panel's first field rather than to a fixed coordinate.
        let field = hwnd.ScreenToClientRc(self.actor_label.hwnd().GetWindowRect()?)?;
        let y = field.top - gui::dpi_y(12);
        rule(
            gui::dpi_x(PANE_X),
            y,
            client.right - gui::dpi_x(20),
            y + unit,
        )
    }

    fn open_new_case(&self) -> w::SysResult<()> {
        let payload = self.payload_edit.text()?;
        let result = self.workspace.borrow_mut().open_case_json(&payload);
        if result.is_ok() {
            self.refresh_case_combo()?;
        }
        self.present(result, "Case opened")
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
        Ok(())
    }

    fn show_view(&self, view: WorkspaceView) -> w::SysResult<()> {
        let result = self.workspace.borrow().render(view);
        self.present(result, view.label())
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
            output.set_text(&windows_lines(
                &serde_json::to_string_pretty(&rows).unwrap_or_else(|error| error.to_string()),
            ))?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn every_command_has_its_own_alt_key() {
        let commands = VIEW_BUTTONS.iter().map(|(caption, _)| *caption).chain([
            OPEN_DATABASE,
            NEW_CASE,
            INTAKE_EVIDENCE,
            PROCESSING_QUEUE,
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
        ]);

        let mut claimed: Vec<(char, &str)> = Vec::new();
        for caption in commands {
            let letter =
                mnemonic(caption).unwrap_or_else(|| panic!("{caption:?} offers no Alt key"));
            if let Some((_, other)) = claimed.iter().find(|(taken, _)| *taken == letter) {
                panic!("Alt+{letter} is claimed by both {other:?} and {caption:?}");
            }
            claimed.push((letter, caption));
        }
    }

    #[test]
    fn ten_view_controls_stay_above_the_action_rail() {
        let last_row = i32::try_from(VIEW_BUTTONS.len() - 1).expect("small navigation rail");
        let last_bottom = VIEW_RAIL_TOP + last_row * VIEW_RAIL_PITCH + VIEW_BUTTON_HEIGHT;
        assert!(
            last_bottom < RAIL_RULES[1],
            "view controls end at {last_bottom}, colliding with the rule at {}",
            RAIL_RULES[1]
        );
        assert!(
            RAIL_RULES[1] < ACTION_RAIL_TOP,
            "the group rule must remain above the first action"
        );
    }
}
