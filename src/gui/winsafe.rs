//! `WinSafe` adapter for the platform-neutral evidence workspace.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use winsafe::{self as w, co, gui, prelude::*};

use super::{AuthorKind, GuiError, GuiResult, Workspace, WorkspaceView, review_target};
use crate::{DemoFixture, ExportAudience, ReviewState};

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
const SEED_VEHICLE: &str = "Seed &Vehicle Stop";
const SEED_HIT_RUN: &str = "Seed Hit-and-&Run";
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

#[derive(Clone)]
struct MainWindow {
    wnd: gui::WindowMain,
    workspace: Rc<RefCell<Workspace>>,
    _labels: Vec<gui::Label>,
    actor_label: gui::Label,
    database_edit: gui::Edit,
    open_button: gui::Button,
    case_combo: gui::ComboBox,
    new_case_button: gui::Button,
    vehicle_button: gui::Button,
    hit_run_button: gui::Button,
    view_buttons: Vec<(WorkspaceView, gui::Button)>,
    suggest_button: gui::Button,
    safe_export_button: gui::Button,
    work_export_button: gui::Button,
    persist_export_button: gui::Button,
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

        // --- Rail group 1: put a case in the workspace ------------------------
        let vehicle_button = button(&wnd, SEED_VEHICLE, RAIL_X, 78, RAIL_WIDTH, ANCHOR);
        let hit_run_button = button(&wnd, SEED_HIT_RUN, RAIL_X, 110, RAIL_WIDTH, ANCHOR);

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
        let search_edit = gui::Edit::new(
            &wnd,
            gui::EditOpts {
                text: "",
                position: gui::dpi(PANE_X, 81),
                width: gui::dpi_x(736),
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
            _labels: labels,
            actor_label,
            database_edit,
            open_button,
            case_combo,
            new_case_button,
            vehicle_button,
            hit_run_button,
            view_buttons,
            suggest_button,
            safe_export_button,
            work_export_button,
            persist_export_button,
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
            &self.vehicle_button,
            &self.hit_run_button,
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
        self.vehicle_button.on().bn_clicked(move || {
            me.seed(DemoFixture::VehicleStop)?;
            Ok(())
        });
        let me = self.clone();
        self.hit_run_button.on().bn_clicked(move || {
            me.seed(DemoFixture::HitAndRun)?;
            Ok(())
        });

        let me = self.clone();
        self.search_button.on().bn_clicked(move || {
            let query = me.search_edit.text()?;
            let result = me.workspace.borrow().search(query.trim(), 100);
            me.present(result, "Search results")?;
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
        let path = self.database_edit.text()?;
        match Workspace::open(path.trim()) {
            Ok(workspace) => {
                *self.workspace.borrow_mut() = workspace;
                self.refresh_case_combo()?;
                self.show_view(WorkspaceView::Overview)?;
                self.set_status(&format!("Opened {}.", path.trim()))
            }
            Err(error) => self.present(Err(error), "Open database"),
        }
    }

    fn seed(&self, fixture: DemoFixture) -> w::SysResult<()> {
        let result = self
            .workspace
            .borrow_mut()
            .seed(fixture)
            .map(|id| format!("Seeded {id}."));
        if result.is_ok() {
            self.refresh_case_combo()?;
        }
        self.present(result, "Demonstration case ready")
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

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn windows_lines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Starts the Windows native frontend for one database path.
pub fn run(database: &Path) {
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
            SEED_VEHICLE,
            SEED_HIT_RUN,
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
