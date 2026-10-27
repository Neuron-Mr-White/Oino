//! Automations (design.md §8 row 49, unipi look A2): the cron job list with state colour +
//! icon + word, next/last run, run history with links to run sessions, a create/edit form
//! (schedule presets + raw cron), run now / stop / pause / resume / enable / disable.
//!
//! Data comes from the host's scheduler (`SessionHost::cron_host`: the embedded scheduler,
//! or the daemon client) — the same ops the CLI and `/cron` use. Keys (view focused):
//! `n` new, `r` run now, `s` stop, `e`/`d` enable/disable, `Backspace`/`Delete` remove,
//! `Esc` closes the overlay / leaves.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, px,
};
use oino_extension_api::RawJson;
use serde_json::{Value, json};

use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::dialog::modal_frame;
use crate::components::empty::empty_state;
use crate::components::page_header::PageHeader;
use crate::components::toast::ToastKind;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::host::SessionHost;
use crate::model::cron::{self, CronJob, CronRun, CronSnapshot, short_time};
use crate::rt::Tokio;
use crate::views::shell::{AppShell as Workspace, Route};

/// A scheduler op's wire payload.
type OpFuture = futures::future::BoxFuture<'static, Result<Value, String>>;

fn call_op(host: &dyn SessionHost, op: &str, payload: Value) -> OpFuture {
    let Some(cron) = host.cron_host() else {
        return Box::pin(futures::future::ready(Err(
            "the cron scheduler is not available in this host".to_owned(),
        )));
    };
    let op = op.to_owned();
    let payload = RawJson::from_json_value(&payload);
    Box::pin(async move {
        cron.call(&op, payload)
            .await
            .map(|v| v.to_value())
            .map_err(|e| e.to_string())
    })
}

/// The create/edit form state.
pub(crate) struct EditForm {
    /// The job being edited (`None` = creating).
    id: Option<String>,
    name: Entity<InputState>,
    schedule: Entity<InputState>,
    prompt: Entity<InputState>,
    tz: Entity<InputState>,
    cwd: Entity<InputState>,
    model: Entity<InputState>,
    /// The preset picker currently open.
    preset: bool,
}

/// Automations view state kept by the workspace.
#[derive(Default)]
pub struct CronUi {
    /// The last snapshot (None = not loaded yet).
    pub snapshot: Option<CronSnapshot>,
    /// The selected job id.
    selected: Option<String>,
    /// Run history of the selected job.
    runs: Vec<CronRun>,
    /// The create/edit dialog.
    pub(crate) edit: Option<EditForm>,
    /// The view's keyboard focus (the shell focuses it on route change).
    pub focus: Option<FocusHandle>,
    /// A one-shot refresh poller (spawned while the page is visible).
    poll: Option<Task<()>>,
}

impl CronUi {
    /// The selected job.
    fn selected_job(&self) -> Option<&CronJob> {
        let id = self.selected.as_deref()?;
        self.snapshot.as_ref()?.jobs.iter().find(|j| j.id == id)
    }
}

impl Workspace {
    fn cron_host_available(&self) -> bool {
        self.host.cron_host().is_some()
    }

    /// Runs one scheduler op, then refreshes and toasts errors.
    fn cron_op(&mut self, op: &str, payload: Value, cx: &mut Context<Self>) {
        let task = Tokio::spawn(cx, call_op(self.host.as_ref(), op, payload));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                if let Ok(Err(e)) = result {
                    ws.toast(ToastKind::Error, None, e, cx);
                }
                ws.cron_refresh(cx);
            });
        })
        .detach();
    }

    /// Refreshes the snapshot and the selected job's runs.
    pub fn cron_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.cron_host_available() {
            return;
        }
        let host = self.host.clone();
        let status = Tokio::spawn(cx, call_op(host.as_ref(), "status", json!({})));
        let list = Tokio::spawn(cx, call_op(host.as_ref(), "list", json!({})));
        let selected = self.cron_ui.selected.clone();
        let runs = match &selected {
            Some(id) => {
                let payload = json!({ "id": id, "limit": 20 });
                Some(Tokio::spawn(cx, call_op(host.as_ref(), "runs", payload)))
            }
            None => None,
        };
        cx.spawn(async move |this, cx| {
            let status = status.await;
            let list = list.await;
            let runs = futures::future::OptionFuture::from(runs).await;
            let _ = this.update(cx, |ws, cx| {
                match (status, list) {
                    (Ok(Ok(status)), Ok(Ok(list))) => {
                        ws.cron_ui.snapshot = Some(cron::snapshot(&status, &list));
                    }
                    (Ok(Err(e)), _) | (_, Ok(Err(e))) => {
                        ws.toast(ToastKind::Error, None, e, cx);
                    }
                    _ => {}
                }
                if let Some(Ok(Ok(reply))) = runs {
                    ws.cron_ui.runs = cron::runs(&reply);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Loads the selected job's run history.
    fn cron_load_runs(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.cron_ui.selected.clone() else {
            return;
        };
        let task = Tokio::spawn(
            cx,
            call_op(self.host.as_ref(), "runs", json!({ "id": id, "limit": 20 })),
        );
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                if let Ok(Ok(reply)) = result {
                    ws.cron_ui.runs = cron::runs(&reply);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Starts the periodic refresh while the page is visible.
    fn cron_poll(&mut self, cx: &mut Context<Self>) {
        self.cron_ui.poll = None;
        self.cron_refresh(cx);
        self.cron_ui.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                this.update(cx, |ws, cx| {
                    if ws.route == Route::Cron {
                        ws.cron_refresh_soft(cx);
                    }
                })
                .ok();
            }
        }));
    }

    /// Refresh without restarting the poller.
    fn cron_refresh_soft(&mut self, cx: &mut Context<Self>) {
        let task_status = Tokio::spawn(cx, call_op(self.host.as_ref(), "status", json!({})));
        let task_list = Tokio::spawn(cx, call_op(self.host.as_ref(), "list", json!({})));
        cx.spawn(async move |this, cx| {
            let status = task_status.await;
            let list = task_list.await;
            let _ = this.update(cx, |ws, cx| {
                if let (Ok(Ok(status)), Ok(Ok(list))) = (status, list) {
                    ws.cron_ui.snapshot = Some(cron::snapshot(&status, &list));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Opens the Automations page.
    pub(crate) fn open_cron(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_view = None;
        self.route = Route::Cron;
        self.cron_poll(cx);
        let focus = self
            .cron_ui
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    // ── actions ────────────────────────────────────────────────────────────

    fn cron_select(&mut self, id: &str, cx: &mut Context<Self>) {
        self.cron_ui.selected = Some(id.to_owned());
        self.cron_load_runs(cx);
        cx.notify();
    }

    fn cron_run_now(&mut self, id: &str, cx: &mut Context<Self>) {
        self.cron_op("run", json!({ "id": id }), cx);
    }

    fn cron_stop(&mut self, id: &str, cx: &mut Context<Self>) {
        self.cron_op("stop", json!({ "id": id }), cx);
    }

    fn cron_toggle_enabled(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        self.cron_op(
            if enabled { "enable" } else { "disable" },
            json!({ "id": id }),
            cx,
        );
    }

    fn cron_pause_all(&mut self, paused: bool, cx: &mut Context<Self>) {
        self.cron_op(if paused { "pause" } else { "resume" }, json!({}), cx);
    }

    fn cron_remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.cron_op("remove", json!({ "id": id }), cx);
        if self.cron_ui.selected.as_deref() == Some(id) {
            self.cron_ui.selected = None;
            self.cron_ui.runs.clear();
        }
    }

    /// Opens a run's session (the run page of the Automations UX).
    fn cron_open_session(&mut self, run: &CronRun, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = run.session_path.clone() else {
            self.toast(ToastKind::Info, None, "This run has no saved session.", cx);
            return;
        };
        let cwd = self
            .cron_ui
            .selected_job()
            .and_then(|j| j.cwd.clone())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            });
        self.open(
            crate::host::OpenRequest::Resume {
                path: path.into(),
                cwd: cwd.into(),
            },
            window,
            cx,
        );
    }

    /// Opens the create form (`n` or the header button).
    fn cron_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cron_ui.edit = Some(EditForm {
            id: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Daily digest")),
            schedule: cx.new(|cx| InputState::new(window, cx).placeholder("every day at 9:00")),
            prompt: cx.new(|cx| {
                InputState::new(window, cx).placeholder("The full, self-contained prompt…")
            }),
            tz: cx.new(|cx| InputState::new(window, cx).placeholder("this PC's zone")),
            cwd: cx.new(|cx| {
                InputState::new(window, cx).placeholder(self.active_cwd(cx).unwrap_or_default())
            }),
            model: cx.new(|cx| InputState::new(window, cx).placeholder("session model")),
            preset: false,
        });
        cx.notify();
    }

    /// Opens the edit form for `id`.
    fn cron_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(job) = self
            .cron_ui
            .snapshot
            .as_ref()
            .and_then(|s| s.jobs.iter().find(|j| j.id == id))
            .cloned()
        else {
            return;
        };
        let name = cx.new(|cx| InputState::new(window, cx));
        let schedule = cx.new(|cx| InputState::new(window, cx));
        let prompt = cx.new(|cx| InputState::new(window, cx));
        let tz = cx.new(|cx| InputState::new(window, cx));
        let cwd = cx.new(|cx| InputState::new(window, cx));
        let model = cx.new(|cx| InputState::new(window, cx));
        name.update(cx, |s, cx| s.set_value(job.name.clone(), window, cx));
        schedule.update(cx, |s, cx| s.set_value(job.schedule.clone(), window, cx));
        prompt.update(cx, |s, cx| {
            s.set_value(job.prompt.clone().unwrap_or_default(), window, cx);
        });
        tz.update(cx, |s, cx| s.set_value(job.tz.clone(), window, cx));
        cwd.update(cx, |s, cx| {
            s.set_value(job.cwd.clone().unwrap_or_default(), window, cx);
        });
        if let Some(value) = &job.model {
            model.update(cx, |s, cx| s.set_value(value.clone(), window, cx));
        }
        self.cron_ui.edit = Some(EditForm {
            id: Some(job.id.clone()),
            name,
            schedule,
            prompt,
            tz,
            cwd,
            model,
            preset: false,
        });
        cx.notify();
    }

    fn active_cwd(&self, cx: &App) -> Option<String> {
        let cwd = self.active_pane().map(|p| p.read(cx).model.cwd.clone())?;
        Some(cwd)
    }

    /// Applies the create/edit form.
    fn cron_save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.cron_ui.edit.take() else {
            return;
        };
        let name = form.name.read(cx).value().trim().to_owned();
        let schedule = form.schedule.read(cx).value().trim().to_owned();
        let prompt = form.prompt.read(cx).value().trim().to_owned();
        if name.is_empty() || schedule.is_empty() || prompt.is_empty() {
            self.toast(
                ToastKind::Error,
                None,
                "Name, schedule and prompt are required.",
                cx,
            );
            self.cron_ui.edit = Some(form);
            return;
        }
        let mut payload = json!({
            "name": name,
            "schedule": schedule,
            "prompt": prompt,
        });
        let tz = form.tz.read(cx).value().trim().to_owned();
        if !tz.is_empty() {
            payload["tz"] = Value::String(tz);
        }
        let cwd = form.cwd.read(cx).value().trim().to_owned();
        if !cwd.is_empty() {
            payload["cwd"] = Value::String(cwd);
        }
        let model = form.model.read(cx).value().trim().to_owned();
        if !model.is_empty() {
            payload["model"] = Value::String(model);
        }
        let edit_id = form.id.clone();
        let editing = edit_id.is_some();
        if editing {
            payload["id"] = Value::String(edit_id.clone().unwrap_or_default());
        }
        let op_name = if editing { "edit" } else { "add" };
        let task = Tokio::spawn(cx, call_op(self.host.as_ref(), op_name, payload));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |ws, _window, cx| {
                match result {
                    Ok(Ok(job)) => {
                        let id = job
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        ws.toast(
                            ToastKind::Success,
                            None,
                            if editing {
                                format!("Updated job {id}.")
                            } else {
                                format!("Added job {id}.")
                            },
                            cx,
                        );
                        if let Some(id) = id.strip_prefix("").map(str::to_owned) {
                            ws.cron_ui.selected = Some(id);
                        }
                        ws.cron_refresh(cx);
                    }
                    Ok(Err(e)) => ws.toast(ToastKind::Error, None, e, cx),
                    Err(e) => ws.toast(ToastKind::Error, None, e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ── keyboard ───────────────────────────────────────────────────────────

    pub(crate) fn on_cron_key(
        &mut self,
        e: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cron_ui.edit.is_some() {
            return; // the dialog owns keys
        }
        let selected = self.cron_ui.selected.clone();
        let Some(id) = selected else {
            return;
        };
        match e.keystroke.key.as_str() {
            "n" => self.cron_new(window, cx),
            "e" => self.cron_edit(&id, window, cx),
            "r" => self.cron_run_now(&id, cx),
            "s" => self.cron_stop(&id, cx),
            "d" => self.cron_toggle_enabled(&id, false, cx),
            "backspace" | "delete" => self.cron_remove(&id, cx),
            _ => {}
        }
    }

    // ── rendering ──────────────────────────────────────────────────────────

    /// Renders the Automations page.
    #[allow(clippy::too_many_lines, reason = "one page layout")]
    pub(crate) fn render_cron(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let focus = self.cron_ui.focus.clone();
        if let Some(f) = &focus
            && window.focused(cx).is_none()
        {
            window.focus(f, cx);
        }
        if !self.cron_host_available() {
            return empty_page(
                &t,
                "Automations",
                "The scheduler is not available in this host.",
            );
        }
        if self.cron_ui.snapshot.is_none() {
            self.cron_refresh(cx);
        }
        let Some(snapshot) = self.cron_ui.snapshot.clone() else {
            return empty_page(&t, "Automations", "Loading jobs…");
        };

        // Header: paused banner + actions.
        let paused = snapshot.paused;
        let header = PageHeader::new("Automations")
            .meta(SharedString::from(if paused {
                format!("paused · {} job(s)", snapshot.jobs.len())
            } else {
                format!(
                    "{} job(s) · concurrency {}",
                    snapshot.jobs.len(),
                    snapshot.concurrency
                )
            }))
            .window_controls(window, &t)
            .tool(
                Btn::new("cron-pause")
                    .label(if paused { "Resume all" } else { "Pause all" })
                    .icon(if paused {
                        IconName::Play
                    } else {
                        IconName::Pause
                    })
                    .size(BtnSize::Sm)
                    .on_click(
                        cx.listener(move |this, _: &gpui_kit::ClickEvent, _window, cx| {
                            this.cron_pause_all(!paused, cx);
                        }),
                    ),
            )
            .tool(
                Btn::new("cron-new")
                    .label("New job")
                    .icon(IconName::Plus)
                    .variant(Variant::Primary)
                    .size(BtnSize::Sm)
                    .on_click(cx.listener(|this, _: &gpui_kit::ClickEvent, window, cx| {
                        this.cron_new(window, cx);
                    })),
            )
            .render(&t);

        // Job list.
        let rows: Vec<AnyElement> = snapshot
            .jobs
            .iter()
            .map(|job| self.cron_job_row(&t, job, cx))
            .collect();
        let list = if snapshot.jobs.is_empty() {
            empty_state(
                &t,
                IconName::CalendarClock,
                "No scheduled jobs yet",
                Some("Run Oino unattended on a schedule — a daily digest, a nightly check, a one-time reminder.".into()),
                vec![
                    Btn::new("cron-empty-new")
                        .label("New job")
                        .icon(IconName::Plus)
                        .variant(Variant::Primary)
                        .size(BtnSize::Xl)
                        .on_click(cx.listener(|this, _: &gpui_kit::ClickEvent, window, cx| {
                            this.cron_new(window, cx);
                        }))
                        .into_any_element(),
                ],
            )
            .mt(px(size::EMPTY_TOP))
            .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(space::S2))
                .children(rows)
                .into_any_element()
        };

        // Detail: selected job's runs.
        let detail = self.cron_detail(&t, cx);
        let dialog = self.cron_form_dialog(&t, window, cx);

        let focus_key = self
            .cron_ui
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        div()
            .id("cron-page")
            .key_context("Cron")
            .track_focus(&focus_key)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                this.on_cron_key(e, window, cx);
            }))
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _: &gpui_kit::MouseDownEvent, _window, cx| {
                    // A click focuses the page (keys need the focus handle).
                    this.cron_ui.focus.get_or_insert_with(|| cx.focus_handle());
                }),
            )
            .flex()
            .flex_col()
            .size_full()
            .gap(px(space::S4))
            .px(px(size::PAGE_GUTTER))
            .py(px(space::S4))
            .min_h_0()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(px(space::S4))
                    .child(
                        div()
                            .id("cron-list")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(list),
                    )
                    .children(detail),
            )
            .children(dialog)
            .into_any_element()
    }

    /// One job row: state colour + icon + word, next/last run, actions.
    fn cron_job_row(&mut self, t: &OinoTheme, job: &CronJob, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.cron_ui.selected.as_deref() == Some(job.id.as_str());
        let (word, color, icon_name) = if job.running {
            ("running", t.working(), IconName::Loader)
        } else if !job.enabled {
            ("off", t.fg_subtlest(), IconName::CircleSlash)
        } else if self
            .cron_ui
            .runs
            .first()
            .is_some_and(|r| r.job_id == job.id && r.is_failure())
        {
            ("failing", t.danger(), IconName::TriangleAlert)
        } else if self
            .cron_ui
            .runs
            .first()
            .is_some_and(|r| r.job_id == job.id && r.status == "ok")
        {
            ("ok", t.success(), IconName::Check)
        } else {
            ("scheduled", t.info(), IconName::CalendarClock)
        };
        let last = self
            .cron_ui
            .runs
            .iter()
            .find(|r| r.job_id == job.id)
            .map_or_else(
                || "—".to_owned(),
                |r| {
                    format!(
                        "{} {}",
                        r.status,
                        short_time(chrono_millis(r.fired_at).as_deref())
                    )
                },
            );
        let id = job.id.clone();
        let id2 = job.id.clone();
        let enabled = job.enabled;
        let run_id = job.id.clone();
        let stop_id = job.id.clone();
        div()
            .id(ElementId::Name(job.id.clone().into()))
            .flex()
            .items_center()
            .gap(px(space::S3))
            .px(px(space::S3))
            .py(px(space::S2_5))
            .rounded(px(radius::R_ROW))
            .bg(if selected { t.selected() } else { t.card() })
            .border_1()
            .border_color(if selected {
                t.border_hover()
            } else {
                t.card_border()
            })
            .hover(|s| s.border_color(t.border_hover()))
            .cursor_pointer()
            .on_click(
                cx.listener(move |this, _: &gpui_kit::ClickEvent, _window, cx| {
                    this.cron_select(&id, cx);
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1_5))
                    .w(px(size::CRON_STATE))
                    .text_color(color)
                    .text_role(t, TextRole::Xs)
                    .weight_semibold()
                    .child(icon(icon_name, size::ICON, color))
                    .child(word),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .flex_1()
                    .gap(px(space::HALF))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(space::S2))
                            .child(
                                div()
                                    .text_role(t, TextRole::Base)
                                    .weight_semibold()
                                    .text_color(t.fg())
                                    .truncate()
                                    .child(job.name.clone()),
                            )
                            .child(
                                div()
                                    .text_role(t, TextRole::Xs)
                                    .text_color(t.fg_subtlest())
                                    .child(job.id.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_role(t, TextRole::Xs)
                            .text_color(t.fg_subtle())
                            .child(format!("{} · {}", job.schedule_pretty(), job.tz)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(space::HALF))
                    .w(px(size::CRON_TIMES))
                    .child(
                        div()
                            .text_role(t, TextRole::Xs)
                            .text_color(if job.enabled {
                                t.fg_subtle()
                            } else {
                                t.fg_subtlest()
                            })
                            .child(if job.enabled {
                                format!("next {}", job.next_fire_short())
                            } else {
                                "off".to_owned()
                            }),
                    )
                    .child(
                        div()
                            .text_role(t, TextRole::Xs)
                            .text_color(t.fg_subtlest())
                            .child(format!("last {last}")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1_5))
                    .child(
                        Btn::new(SharedString::from(format!("cron-run-{run_id}")))
                            .icon(IconName::Play)
                            .size(BtnSize::Xs)
                            .on_click(cx.listener(
                                move |this, _: &gpui_kit::ClickEvent, _w, cx| {
                                    this.cron_run_now(&run_id, cx);
                                },
                            )),
                    )
                    .when(job.running, |el| {
                        el.child(
                            Btn::new(SharedString::from(format!("cron-stop-{stop_id}")))
                                .icon(IconName::Square)
                                .size(BtnSize::Xs)
                                .on_click(cx.listener(
                                    move |this, _: &gpui_kit::ClickEvent, _w, cx| {
                                        this.cron_stop(&stop_id, cx);
                                    },
                                )),
                        )
                    })
                    .child(
                        Btn::new(SharedString::from(format!("cron-toggle-{id2}")))
                            .label(if enabled { "Disable" } else { "Enable" })
                            .size(BtnSize::Xs)
                            .on_click(cx.listener(
                                move |this, _: &gpui_kit::ClickEvent, _w, cx| {
                                    this.cron_toggle_enabled(&id2, !enabled, cx);
                                },
                            )),
                    ),
            )
            .into_any_element()
    }

    /// The selected job's detail column: fields + run history.
    fn cron_detail(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let job = self.cron_ui.selected_job()?.clone();
        let runs: Vec<CronRun> = self
            .cron_ui
            .runs
            .iter()
            .filter(|r| r.job_id == job.id)
            .cloned()
            .collect();
        let edit_id = job.id.clone();
        let remove_id = job.id.clone();
        let mut body = vec![
            field_row(t, "Prompt", job.prompt.as_deref().unwrap_or("")),
            field_row(t, "Folder", job.cwd.as_deref().unwrap_or("—")),
            field_row(t, "Model", job.model.as_deref().unwrap_or("session model")),
        ];
        body.push(
            div()
                .flex()
                .items_center()
                .gap(px(space::S1_5))
                .pt(px(space::S2))
                .child(
                    Btn::new("cron-detail-edit")
                        .label("Edit")
                        .size(BtnSize::Xs)
                        .on_click(cx.listener(
                            move |this, _: &gpui_kit::ClickEvent, window, cx| {
                                this.cron_edit(&edit_id, window, cx);
                            },
                        )),
                )
                .child(
                    Btn::new("cron-detail-remove")
                        .label("Remove")
                        .size(BtnSize::Xs)
                        .on_click(cx.listener(move |this, _: &gpui_kit::ClickEvent, _w, cx| {
                            this.cron_remove(&remove_id, cx);
                        })),
                ),
        );
        let run_rows: Vec<AnyElement> = if runs.is_empty() {
            vec![
                div()
                    .text_role(t, TextRole::Xs)
                    .text_color(t.fg_subtlest())
                    .py(px(space::S2))
                    .child("No runs yet.")
                    .into_any_element(),
            ]
        } else {
            runs.iter()
                .map(|run| {
                    let run = run.clone();
                    div()
                        .id(ElementId::Name(run.id.clone().into()))
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .px(px(space::S2))
                        .py(px(space::S1_5))
                        .rounded(px(radius::R_ROW))
                        .bg(t.card())
                        .border_1()
                        .border_color(t.card_border())
                        .when(run.is_failure(), |el| el.border_color(t.danger()))
                        .child(
                            div()
                                .w(px(size::CRON_LABEL))
                                .text_role(t, TextRole::Xs)
                                .weight_semibold()
                                .text_color(if run.is_failure() {
                                    t.danger()
                                } else if run.status == "ok" {
                                    t.success()
                                } else {
                                    t.fg_subtle()
                                })
                                .child(run.status.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_role(t, TextRole::Xs)
                                .text_color(t.fg_subtle())
                                .child(run.summary_text().to_owned()),
                        )
                        .when(run.missed, |el| {
                            el.child(
                                div()
                                    .text_role(t, TextRole::Xs)
                                    .text_color(t.warning())
                                    .child("missed"),
                            )
                        })
                        .when(run.session_path.is_some(), |el| {
                            el.child(
                                Btn::new(SharedString::from(format!("cron-open-{}", run.id)))
                                    .label("Session")
                                    .icon(IconName::ExternalLink)
                                    .size(BtnSize::Xs)
                                    .on_click(cx.listener(
                                        move |this, _: &gpui_kit::ClickEvent, window, cx| {
                                            this.cron_open_session(&run, window, cx);
                                        },
                                    )),
                            )
                        })
                        .into_any_element()
                })
                .collect()
        };
        Some(
            div()
                .flex()
                .flex_col()
                .w(px(size::CRON_DETAIL))
                .flex_shrink_0()
                .min_h_0()
                .gap(px(space::S2))
                .child(
                    div()
                        .text_role(t, TextRole::Sm)
                        .weight_semibold()
                        .text_color(t.fg())
                        .child(job.name.clone()),
                )
                .children(body)
                .child(
                    div()
                        .text_role(t, TextRole::Xs)
                        .weight_semibold()
                        .text_color(t.fg_subtle())
                        .pt(px(space::S1))
                        .child("Run history"),
                )
                .child(
                    div()
                        .id("cron-runs")
                        .flex()
                        .flex_col()
                        .gap(px(space::S1))
                        .overflow_y_scroll()
                        .min_h_0()
                        .children(run_rows),
                )
                .into_any_element(),
        )
    }

    /// The create/edit dialog (schedule presets + raw field).
    fn cron_form_dialog(
        &mut self,
        t: &OinoTheme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.cron_ui.edit.as_ref()?;
        let presets = [
            ("Every 5 minutes", "every 5 minutes"),
            ("Hourly", "hourly"),
            ("Daily at 9:00", "every day at 9:00"),
            ("Weekdays at 18:30", "weekdays at 18:30"),
            ("Every Monday 8:00", "every monday 08:00"),
            ("In 2 hours (once)", "in 2 hours"),
        ];
        let preset_buttons: Vec<AnyElement> = presets
            .iter()
            .map(|(label, value)| {
                let value = (*value).to_owned();
                Btn::new(SharedString::from(format!("cron-preset-{value}")))
                    .label((*label).to_owned())
                    .size(BtnSize::Xs)
                    .on_click(
                        cx.listener(move |this, _: &gpui_kit::ClickEvent, window, cx| {
                            let value = value.clone();
                            if let Some(form) = this.cron_ui.edit.as_ref() {
                                form.schedule
                                    .update(cx, |s, cx| s.set_value(value, window, cx));
                            }
                            if let Some(f) = this.cron_ui.edit.as_mut() {
                                f.preset = false;
                            }
                            cx.notify();
                        }),
                    )
                    .into_any_element()
            })
            .collect();
        let tz_hint = form.tz.read(cx).value().trim().is_empty();
        let title: SharedString = if form.id.is_some() {
            "Edit job".into()
        } else {
            "New job".into()
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(space::S3))
            .child(
                div()
                    .text_role(t, TextRole::Lg)
                    .weight_semibold()
                    .text_color(t.fg())
                    .child(title.clone()),
            )
            .child(labeled_input(t, "Name", &form.name, cx))
            .child(labeled_input(t, "Prompt", &form.prompt, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S1))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S2))
                            .child(
                                div()
                                    .text_role(t, TextRole::Xs)
                                    .weight_semibold()
                                    .text_color(t.fg_subtle())
                                    .child("Schedule"),
                            )
                            .child(
                                Btn::new("cron-preset-toggle")
                                    .label("Presets")
                                    .size(BtnSize::Xs)
                                    .on_click(cx.listener(
                                        |this, _: &gpui_kit::ClickEvent, _w, cx| {
                                            if let Some(f) = this.cron_ui.edit.as_mut() {
                                                f.preset = !f.preset;
                                            }
                                            cx.notify();
                                        },
                                    )),
                            ),
                    )
                    .child(labeled_input(
                        t,
                        "Plain language or 6-field cron",
                        &form.schedule,
                        cx,
                    ))
                    .when(form.preset, |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(px(space::S1_5))
                                .children(preset_buttons),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(space::S2))
                    .child(div().flex_1().child(labeled_input(
                        t,
                        if tz_hint {
                            "Time zone (default: this PC's)"
                        } else {
                            "Time zone"
                        },
                        &form.tz,
                        cx,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .child(labeled_input(t, "Folder", &form.cwd, cx)),
                    ),
            )
            .child(labeled_input(t, "Model (optional)", &form.model, cx))
            .into_any_element();
        let cancel = Btn::new("cron-form-cancel")
            .label("Cancel")
            .size(BtnSize::Xl)
            .on_click(cx.listener(|this, _: &gpui_kit::ClickEvent, _w, cx| {
                this.cron_ui.edit = None;
                cx.notify();
            }))
            .into_any_element();
        let save = Btn::new("cron-form-save")
            .label("Save job")
            .variant(Variant::Primary)
            .size(BtnSize::Xl)
            .on_click(cx.listener(|this, _: &gpui_kit::ClickEvent, window, cx| {
                this.cron_save_form(window, cx);
            }))
            .into_any_element();
        let shell = cx.entity().downgrade();
        let close: crate::components::dialog::OnClose = Rc::new(move |window, cx| {
            let _ = window;
            let _ = shell.update(cx, |ws, cx| {
                ws.cron_ui.edit = None;
                cx.notify();
            });
        });
        Some(modal_frame(
            t,
            "cron-form",
            title,
            560.0,
            body,
            vec![cancel, save],
            close,
        ))
    }
}

fn empty_page(t: &OinoTheme, title: &str, message: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .pt(px(size::EMPTY_TOP))
        .gap(px(space::S2))
        .child(
            div()
                .text_role(t, TextRole::Xl)
                .weight_semibold()
                .text_color(t.fg())
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(message.to_owned()),
        )
        .into_any_element()
}

fn field_row(t: &OinoTheme, key: &str, value: &str) -> gpui_kit::Div {
    div()
        .flex()
        .gap(px(space::S2))
        .child(
            div()
                .w(px(size::CRON_LABEL))
                .flex_shrink_0()
                .text_role(t, TextRole::Xs)
                .text_color(t.fg_subtlest())
                .child(key.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_role(t, TextRole::Xs)
                .text_color(t.fg_subtle())
                .truncate()
                .child(value.to_owned()),
        )
}

fn labeled_input(
    t: &OinoTheme,
    label: &str,
    input: &Entity<InputState>,
    cx: &App,
) -> gpui_kit::Div {
    let _ = cx;
    div()
        .flex()
        .flex_col()
        .gap(px(space::S1))
        .child(
            div()
                .text_role(t, TextRole::Xs)
                .weight_semibold()
                .text_color(t.fg_subtle())
                .child(label.to_owned()),
        )
        .child(Input::new(&input.clone()))
}

/// `fired_at` unix ms → RFC3339 (approximation for the short list form; the scheduler
/// already reports RFC3339 for `next_fire`, and runs sort newest-first regardless).
fn chrono_millis(ms: u64) -> Option<String> {
    let secs = i64::try_from(ms / 1000).ok()?;
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (h, m) = (rest / 3600, (rest % 3600) / 60);
    // Epoch-day → Y-M-D (civil from days algorithm).
    let z = 719_468 + days;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    Some(format!("{y:04}-{mth:02}-{d:02}T{h:02}:{m:02}:00Z"))
}
