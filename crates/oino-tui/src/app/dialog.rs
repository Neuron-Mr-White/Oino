//! Extension dialogs (ui-protocol §4.1, §4.3): selection, confirmation, text input, editor,
//! multi-select, form, approval and custom OUS dialogs, answered with pi response forms.

use std::collections::HashMap;
use std::time::Instant;

use oino_ai::JsonValue;
use oino_core::ui::UiAnswer;
use oino_view::composer::Composer;
use oino_view::keys::{Key, KeyId};
use oino_view::ous::Interactive;
use oino_view::ui_state::{Dialog, DialogKind};
use serde_json::json;

use super::overlay::line_edit;
use super::{App, EditPurpose, Effect};

/// Per-question form state.
#[derive(Clone, Debug, Default)]
pub(crate) struct FormAnswer {
    pub(crate) checked: Vec<bool>,
    pub(crate) other: Composer,
    pub(crate) answered: bool,
}

/// Widget state of the active dialog.
#[derive(Clone, Debug)]
pub(crate) enum DialogUi {
    /// `select`, `confirm` (Yes/No), `approval` option list.
    List {
        /// Dialog id.
        id: String,
        /// Selected row.
        selected: usize,
        /// Feedback input (`approval` with feedback).
        feedback: Option<Composer>,
        /// Focus is on the feedback field.
        feedback_focus: bool,
        /// When the dialog opened (timeout countdown).
        opened: Instant,
    },
    /// `input`.
    Input {
        /// Dialog id.
        id: String,
        /// Text.
        input: Composer,
        /// Opened.
        opened: Instant,
    },
    /// `editor`.
    Editor {
        /// Dialog id.
        id: String,
        /// Text.
        input: Composer,
    },
    /// `multiSelect`.
    Multi {
        /// Dialog id.
        id: String,
        /// Cursor.
        selected: usize,
        /// Checked flags.
        checked: Vec<bool>,
        /// Validation message.
        error: Option<String>,
    },
    /// `form`.
    Form {
        /// Dialog id.
        id: String,
        /// Question index.
        question: usize,
        /// Cursor within the question (options, then "Other").
        selected: usize,
        /// Answers per question.
        answers: Vec<FormAnswer>,
        /// Typing into "Other".
        editing_other: bool,
    },
    /// `custom`.
    Custom {
        /// Dialog id.
        id: String,
        /// Focused interactive control.
        focus: usize,
        /// Current control values by field id.
        values: HashMap<String, JsonValue>,
        /// Text input of the focused `input` control.
        input: Option<Composer>,
        /// Cursor inside a focused selectable list.
        choice: usize,
    },
    /// Waiting on the host (clipboard, external editor).
    Pending {
        /// Dialog id.
        id: String,
    },
}

impl DialogUi {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::List { id, .. }
            | Self::Input { id, .. }
            | Self::Editor { id, .. }
            | Self::Multi { id, .. }
            | Self::Form { id, .. }
            | Self::Custom { id, .. }
            | Self::Pending { id } => id,
        }
    }
}

fn composer_with(text: &str) -> Composer {
    let mut c = Composer::new();
    c.set_text(text);
    c
}

fn wrap(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let len_i = isize::try_from(len).unwrap_or(isize::MAX);
    usize::try_from((isize::try_from(current).unwrap_or(0) + delta).rem_euclid(len_i)).unwrap_or(0)
}

impl App {
    /// Aligns the dialog widget with the front of the dialog queue.
    pub(crate) fn sync_dialog(&mut self) {
        let Some(dialog) = self.ui.active_dialog().cloned() else {
            self.dialog_ui = None;
            return;
        };
        if self.dialog_ui.as_ref().is_some_and(|d| d.id() == dialog.id) {
            return;
        }
        self.dialog_ui = Some(self.init_dialog(&dialog));
    }

    fn init_dialog(&mut self, dialog: &Dialog) -> DialogUi {
        let id = dialog.id.clone();
        let opened = Instant::now();
        match &dialog.kind {
            DialogKind::Select { .. } | DialogKind::Confirm { .. } => DialogUi::List {
                id,
                selected: 0,
                feedback: None,
                feedback_focus: false,
                opened,
            },
            DialogKind::Approval { feedback, .. } => DialogUi::List {
                id,
                selected: 0,
                feedback: feedback.as_ref().map(|_| Composer::new()),
                feedback_focus: false,
                opened,
            },
            DialogKind::Input { prefill, .. } => DialogUi::Input {
                id,
                input: composer_with(prefill.as_deref().unwrap_or("")),
                opened,
            },
            DialogKind::Editor { prefill, .. } => DialogUi::Editor {
                id,
                input: composer_with(prefill),
            },
            DialogKind::MultiSelect { options, .. } => DialogUi::Multi {
                id,
                selected: 0,
                checked: options.iter().map(|o| o.selected).collect(),
                error: None,
            },
            DialogKind::Form { questions, .. } => DialogUi::Form {
                id,
                question: 0,
                selected: 0,
                answers: questions
                    .iter()
                    .map(|q| FormAnswer {
                        checked: vec![false; q.options.len()],
                        ..FormAnswer::default()
                    })
                    .collect(),
                editing_other: false,
            },
            DialogKind::Custom { spec, .. } => {
                let mut values = HashMap::new();
                for control in spec.interactive() {
                    match control {
                        Interactive::Input { id, value, .. } => {
                            values.insert(id, JsonValue::String(value));
                        }
                        Interactive::Toggle { id, value, .. } => {
                            values.insert(id, JsonValue::Bool(value));
                        }
                        Interactive::Select {
                            id, value, options, ..
                        } => {
                            let v = value.or_else(|| options.first().map(|o| o.value.clone()));
                            values.insert(id, v.map_or(JsonValue::Null, JsonValue::String));
                        }
                        Interactive::Button { .. } | Interactive::Choice { .. } => {}
                    }
                }
                DialogUi::Custom {
                    id,
                    focus: 0,
                    values,
                    input: None,
                    choice: 0,
                }
            }
            DialogKind::ClipboardReadImage => {
                self.effects.push(Effect::ClipboardImage(Some(id.clone())));
                DialogUi::Pending { id }
            }
            DialogKind::EditExternally {
                text,
                extension_hint,
            } => {
                self.effects.push(Effect::ExternalEditor {
                    text: text.clone(),
                    extension: extension_hint.clone().unwrap_or_else(|| "md".into()),
                    purpose: EditPurpose::External(id.clone()),
                });
                DialogUi::Pending { id }
            }
            DialogKind::Unsupported { .. } => {
                self.effects
                    .push(Effect::Respond(oino_core::ui::UiResponse {
                        id: id.clone(),
                        answer: UiAnswer::Cancelled,
                    }));
                DialogUi::Pending { id }
            }
        }
    }

    /// Key handling while a dialog is shown.
    #[allow(
        clippy::too_many_lines,
        reason = "one block per dialog kind (ui-protocol §4)"
    )]
    pub(crate) fn dialog_key(&mut self, key: KeyId) {
        let Some(dialog) = self.ui.active_dialog().cloned() else {
            return;
        };
        let Some(mut state) = self.dialog_ui.take() else {
            return;
        };
        let up = self.keys.matches("tui.select.up", &key);
        let down = self.keys.matches("tui.select.down", &key);
        let confirm = self.keys.matches("tui.select.confirm", &key);
        let cancel = self.keys.matches("tui.select.cancel", &key) && key.key == Key::Escape
            || (key == KeyId::ctrl('c'));
        let id = dialog.id.clone();
        let mut answer: Option<UiAnswer> = None;
        match (&dialog.kind, &mut state) {
            (DialogKind::Select { options, .. }, DialogUi::List { selected, .. }) => {
                if cancel {
                    answer = Some(UiAnswer::Cancelled);
                } else if up || down {
                    *selected = wrap(*selected, if up { -1 } else { 1 }, options.len());
                } else if confirm {
                    if let Some(option) = options.get(*selected) {
                        answer = Some(UiAnswer::Value(JsonValue::String(option.label.clone())));
                    }
                } else if let Some(d) = key.printable().and_then(|c| c.to_digit(10)) {
                    let index = usize::try_from(d).unwrap_or(0);
                    if (1..=options.len()).contains(&index) {
                        answer = Some(UiAnswer::Value(JsonValue::String(
                            options[index - 1].label.clone(),
                        )));
                    }
                }
            }
            (DialogKind::Confirm { .. }, DialogUi::List { selected, .. }) => {
                if cancel {
                    answer = Some(UiAnswer::Cancelled);
                } else if up
                    || down
                    || key.key == Key::Left
                    || key.key == Key::Right
                    || key.key == Key::Tab
                {
                    *selected = 1 - (*selected).min(1);
                } else if confirm {
                    answer = Some(UiAnswer::Confirmed(*selected == 0));
                } else if key.printable() == Some('y') {
                    answer = Some(UiAnswer::Confirmed(true));
                } else if key.printable() == Some('n') {
                    answer = Some(UiAnswer::Confirmed(false));
                }
            }
            (
                DialogKind::Approval { options, .. },
                DialogUi::List {
                    selected,
                    feedback,
                    feedback_focus,
                    ..
                },
            ) => {
                if cancel {
                    answer = Some(UiAnswer::Cancelled);
                } else if key.key == Key::Tab && feedback.is_some() {
                    *feedback_focus = !*feedback_focus;
                } else if *feedback_focus && !confirm {
                    if let Some(input) = feedback {
                        line_edit(input, &key);
                    }
                } else if up || down {
                    *selected = wrap(*selected, if up { -1 } else { 1 }, options.len());
                } else if confirm && let Some(option) = options.get(*selected) {
                    let mut value = json!({"optionId": option.option_id});
                    if let Some(text) = feedback
                        .as_ref()
                        .map(Composer::text)
                        .filter(|t| !t.trim().is_empty())
                    {
                        value["feedback"] = JsonValue::String(text);
                    }
                    answer = Some(UiAnswer::Value(value));
                }
            }
            (DialogKind::Input { .. }, DialogUi::Input { input, .. }) => {
                if cancel {
                    answer = Some(UiAnswer::Cancelled);
                } else if self.keys.matches("tui.input.submit", &key) {
                    answer = Some(UiAnswer::Value(JsonValue::String(input.text())));
                } else {
                    line_edit(input, &key);
                }
            }
            (DialogKind::Editor { language, .. }, DialogUi::Editor { input, .. }) => {
                if cancel {
                    answer = Some(UiAnswer::Cancelled);
                } else if self.keys.matches("tui.input.newLine", &key) {
                    input.newline();
                } else if self.keys.matches("tui.input.submit", &key) {
                    answer = Some(UiAnswer::Value(JsonValue::String(input.text())));
                } else if self.keys.matches("app.editor.external", &key) {
                    self.effects.push(Effect::ExternalEditor {
                        text: input.text(),
                        extension: language.clone().unwrap_or_else(|| "md".into()),
                        purpose: EditPurpose::Dialog(id.clone()),
                    });
                } else if up {
                    input.up();
                } else if down {
                    input.down();
                } else {
                    line_edit(input, &key);
                }
            }
            (
                DialogKind::MultiSelect {
                    options, min, max, ..
                },
                DialogUi::Multi {
                    selected,
                    checked,
                    error,
                    ..
                },
            ) => {
                if cancel {
                    answer = Some(UiAnswer::Cancelled);
                } else if up || down {
                    *selected = wrap(*selected, if up { -1 } else { 1 }, options.len());
                } else if key.key == Key::Space {
                    if let Some(flag) = checked.get_mut(*selected) {
                        *flag = !*flag;
                    }
                    *error = None;
                } else if confirm {
                    let count = checked.iter().filter(|c| **c).count();
                    if min.is_some_and(|m| count < m) {
                        *error = Some(format!("Select at least {}", min.unwrap_or(0)));
                    } else if max.is_some_and(|m| count > m) {
                        *error = Some(format!("Select at most {}", max.unwrap_or(0)));
                    } else {
                        let values: Vec<JsonValue> = options
                            .iter()
                            .zip(checked.iter())
                            .filter(|(_, c)| **c)
                            .map(|(o, _)| {
                                JsonValue::String(
                                    o.value.clone().unwrap_or_else(|| o.label.clone()),
                                )
                            })
                            .collect();
                        answer = Some(UiAnswer::Value(JsonValue::Array(values)));
                    }
                }
            }
            (
                DialogKind::Form { questions, .. },
                DialogUi::Form {
                    question,
                    selected,
                    answers,
                    editing_other,
                    ..
                },
            ) => {
                let Some(q) = questions.get(*question) else {
                    self.dialog_ui = Some(state);
                    return;
                };
                let rows = q.options.len() + usize::from(q.other);
                let on_other = q.other && *selected == q.options.len();
                if cancel {
                    if *editing_other {
                        *editing_other = false;
                    } else {
                        answer = Some(UiAnswer::Cancelled);
                    }
                } else if *editing_other && !confirm {
                    line_edit(&mut answers[*question].other, &key);
                } else if up || down {
                    *selected = wrap(*selected, if up { -1 } else { 1 }, rows);
                } else if key.key == Key::Left || (key.key == Key::Tab && key.mods.shift) {
                    *question = question.saturating_sub(1);
                    *selected = 0;
                } else if key.key == Key::Right || key.key == Key::Tab {
                    *question = (*question + 1).min(questions.len() - 1);
                    *selected = 0;
                } else if key.key == Key::Space && q.multi_select && !on_other {
                    if let Some(flag) = answers[*question].checked.get_mut(*selected) {
                        *flag = !*flag;
                    }
                } else if confirm || key.printable().and_then(|c| c.to_digit(10)).is_some() {
                    if let Some(d) = key.printable().and_then(|c| c.to_digit(10)) {
                        let index = usize::try_from(d).unwrap_or(0);
                        if (1..=q.options.len()).contains(&index) {
                            *selected = index - 1;
                        }
                    }
                    let on_other = q.other && *selected == q.options.len();
                    if on_other && !*editing_other {
                        *editing_other = true;
                    } else {
                        *editing_other = false;
                        let entry = &mut answers[*question];
                        if !q.multi_select && !on_other {
                            entry.checked.iter_mut().for_each(|c| *c = false);
                            if let Some(flag) = entry.checked.get_mut(*selected) {
                                *flag = true;
                            }
                        }
                        entry.answered = true;
                        if *question + 1 < questions.len() {
                            *question += 1;
                            *selected = 0;
                        } else {
                            answer = Some(UiAnswer::Value(form_value(questions, answers)));
                        }
                    }
                }
            }
            (
                DialogKind::Custom { spec, .. },
                DialogUi::Custom {
                    focus,
                    values,
                    input,
                    choice,
                    ..
                },
            ) => {
                let controls = spec.interactive();
                if cancel {
                    if input.is_some() {
                        *input = None;
                    } else {
                        answer = Some(UiAnswer::Cancelled);
                    }
                } else if let Some(text) = input {
                    if confirm {
                        if let Some(Interactive::Input {
                            id: field,
                            action_id,
                            ..
                        }) = controls.get(*focus)
                        {
                            let value = JsonValue::String(text.text());
                            values.insert(field.clone(), value.clone());
                            answer = Some(UiAnswer::Value(
                                json!({"actionId": action_id, "value": value}),
                            ));
                        }
                        *input = None;
                    } else {
                        line_edit(text, &key);
                    }
                } else if key.key == Key::Tab {
                    *focus = wrap(*focus, if key.mods.shift { -1 } else { 1 }, controls.len());
                    *choice = 0;
                } else if let Some(control) = controls.get(*focus) {
                    match control {
                        Interactive::Button { action_id, .. } if confirm => {
                            answer = Some(UiAnswer::Value(json!({"actionId": action_id})));
                        }
                        Interactive::Input { id: field, .. }
                            if confirm || key.printable().is_some() =>
                        {
                            let current = values
                                .get(field)
                                .and_then(JsonValue::as_str)
                                .unwrap_or("")
                                .to_owned();
                            let mut c = composer_with(&current);
                            if let Some(ch) = key.printable() {
                                c.insert_char(ch);
                            }
                            *input = Some(c);
                        }
                        Interactive::Toggle { id: field, .. }
                            if confirm || key.key == Key::Space =>
                        {
                            let next = !values
                                .get(field)
                                .and_then(JsonValue::as_bool)
                                .unwrap_or(false);
                            values.insert(field.clone(), JsonValue::Bool(next));
                        }
                        Interactive::Select {
                            id: field, options, ..
                        } if confirm
                            || key.key == Key::Space
                            || key.key == Key::Right
                            || key.key == Key::Left =>
                        {
                            let current =
                                values.get(field).and_then(JsonValue::as_str).unwrap_or("");
                            let pos = options.iter().position(|o| o.value == current).unwrap_or(0);
                            let next = wrap(
                                pos,
                                if key.key == Key::Left { -1 } else { 1 },
                                options.len(),
                            );
                            if let Some(option) = options.get(next) {
                                values
                                    .insert(field.clone(), JsonValue::String(option.value.clone()));
                            }
                        }
                        Interactive::Choice { action_id, ids, .. } => {
                            if up || down {
                                *choice = wrap(*choice, if up { -1 } else { 1 }, ids.len());
                            } else if confirm && let Some(item) = ids.get(*choice) {
                                answer = Some(UiAnswer::Value(
                                    json!({"actionId": action_id, "value": item}),
                                ));
                            }
                        }
                        _ => {
                            if up || down {
                                *focus = wrap(*focus, if up { -1 } else { 1 }, controls.len());
                            }
                        }
                    }
                } else if confirm {
                    answer = Some(UiAnswer::Value(json!({"actionId": "$submit"})));
                }
            }
            (_, DialogUi::Pending { .. }) if cancel => {
                answer = Some(UiAnswer::Cancelled);
            }
            _ => {}
        }
        self.dialog_ui = Some(state);
        if let Some(answer) = answer {
            self.answer(&id, answer);
        }
    }
}

fn form_value(
    questions: &[oino_view::ui_state::FormQuestion],
    answers: &[FormAnswer],
) -> JsonValue {
    let mut action: Option<String> = None;
    let list: Vec<JsonValue> = questions
        .iter()
        .zip(answers)
        .map(|(q, a)| {
            let selected: Vec<JsonValue> = q
                .options
                .iter()
                .zip(&a.checked)
                .filter(|(_, c)| **c)
                .map(|(o, _)| {
                    if o.action.is_some() {
                        action.clone_from(&o.action);
                    }
                    JsonValue::String(o.value.clone().unwrap_or_else(|| o.label.clone()))
                })
                .collect();
            let mut value = json!({"id": q.id, "selected": selected});
            let other = a.other.text();
            if !other.trim().is_empty() {
                value["other"] = JsonValue::String(other);
            }
            value
        })
        .collect();
    let mut value = json!({"answers": list});
    if let Some(action) = action {
        value["action"] = JsonValue::String(action);
    }
    value
}
