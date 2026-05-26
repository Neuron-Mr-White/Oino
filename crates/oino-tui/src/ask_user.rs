#![forbid(unsafe_code)]

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AskUserOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AskUserQuestion {
    pub question: String,
    #[serde(default)]
    pub header: String,
    pub options: Vec<AskUserOption>,
    #[serde(default, alias = "multiSelect")]
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AskUserRequest {
    pub questions: Vec<AskUserQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AskUserAnswer {
    pub question_index: usize,
    pub question: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AskUserOutcome {
    pub answers: Vec<AskUserAnswer>,
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskUserOverlayState {
    pub request: AskUserRequest,
    pub current: usize,
    pub cursor: usize,
    pub selected: Vec<Vec<usize>>,
    pub custom_input: String,
    pub custom_active: bool,
}

impl AskUserOverlayState {
    pub fn new(request: AskUserRequest) -> Self {
        let selected = request.questions.iter().map(|_| Vec::new()).collect();
        Self {
            request,
            current: 0,
            cursor: 0,
            selected,
            custom_input: String::new(),
            custom_active: false,
        }
    }

    pub fn question(&self) -> Option<&AskUserQuestion> {
        self.request.questions.get(self.current)
    }

    pub fn selected_option(&self) -> Option<&AskUserOption> {
        self.question()?.options.get(self.cursor)
    }

    pub(crate) fn move_cursor(&mut self, delta: isize) {
        let len = self.question().map_or(0, |question| question.options.len());
        self.cursor = move_index(self.cursor, len, delta);
    }

    pub(crate) fn toggle_current(&mut self) {
        let Some(question) = self.question() else {
            return;
        };
        if self.cursor >= question.options.len() {
            return;
        }
        let selected = &mut self.selected[self.current];
        if let Some(index) = selected.iter().position(|value| *value == self.cursor) {
            selected.remove(index);
        } else {
            selected.push(self.cursor);
        }
    }

    fn advance_or_finish(&mut self, answer: AskUserAnswer) -> Option<AskUserOutcome> {
        if self.current + 1 >= self.request.questions.len() {
            let mut answers = self.collected_answers_until_current();
            answers.push(answer);
            return Some(AskUserOutcome {
                answers,
                cancelled: false,
                error: None,
            });
        }
        self.current += 1;
        self.cursor = 0;
        self.custom_input.clear();
        self.custom_active = false;
        None
    }

    fn collected_answers_until_current(&self) -> Vec<AskUserAnswer> {
        (0..self.current)
            .filter_map(|index| self.answer_for_question(index))
            .collect()
    }

    fn answer_for_question(&self, index: usize) -> Option<AskUserAnswer> {
        let question = self.request.questions.get(index)?;
        let selected = self.selected.get(index).cloned().unwrap_or_default();
        if question.multi_select {
            let labels = selected
                .iter()
                .filter_map(|option| question.options.get(*option))
                .map(|option| option.label.clone())
                .collect::<Vec<_>>();
            Some(AskUserAnswer {
                question_index: index,
                question: question.question.clone(),
                kind: "multi".into(),
                answer: labels.first().cloned(),
                selected: labels,
                notes: None,
                preview: None,
            })
        } else {
            let option = selected
                .first()
                .and_then(|option| question.options.get(*option))?;
            Some(AskUserAnswer {
                question_index: index,
                question: question.question.clone(),
                kind: "option".into(),
                answer: Some(option.label.clone()),
                selected: vec![option.label.clone()],
                notes: None,
                preview: option.preview.clone(),
            })
        }
    }

    pub(crate) fn answer_current_option(&mut self) -> Option<AskUserOutcome> {
        let question = self.question()?.clone();
        if question.multi_select {
            if self.selected[self.current].is_empty() {
                self.toggle_current();
            }
            let answer = self.answer_for_question(self.current)?;
            self.advance_or_finish(answer)
        } else {
            self.selected[self.current] = vec![self.cursor];
            let option = question.options.get(self.cursor)?.clone();
            let answer = AskUserAnswer {
                question_index: self.current,
                question: question.question,
                kind: "option".into(),
                answer: Some(option.label.clone()),
                selected: vec![option.label],
                notes: None,
                preview: option.preview,
            };
            self.advance_or_finish(answer)
        }
    }

    pub(crate) fn answer_custom(&mut self) -> Option<AskUserOutcome> {
        let question = self.question()?.clone();
        let text = self.custom_input.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let answer = AskUserAnswer {
            question_index: self.current,
            question: question.question,
            kind: "custom".into(),
            answer: Some(text),
            selected: Vec::new(),
            notes: None,
            preview: None,
        };
        self.advance_or_finish(answer)
    }

    pub(crate) fn answer_chat(&mut self) -> Option<AskUserOutcome> {
        let question = self.question()?.clone();
        let answer = AskUserAnswer {
            question_index: self.current,
            question: question.question,
            kind: "chat".into(),
            answer: Some("Chat about this".into()),
            selected: Vec::new(),
            notes: None,
            preview: None,
        };
        self.advance_or_finish(answer)
    }
}

fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len.saturating_sub(1) as isize;
    (current as isize + delta).clamp(0, last) as usize
}
