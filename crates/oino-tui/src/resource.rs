#![forbid(unsafe_code)]

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptResource {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    pub source: String,
    pub scope: String,
    pub content: String,
}

impl PromptResource {
    #[must_use]
    pub fn command(&self) -> String {
        format!("/{}", self.name)
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        match &self.argument_hint {
            Some(hint) if !hint.trim().is_empty() => format!("/{} {}", self.name, hint),
            _ => self.command(),
        }
    }

    #[must_use]
    pub fn expand(&self, args: &str) -> String {
        expand_template(&self.content, args)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillResource {
    pub name: String,
    pub description: String,
    pub source: String,
    pub scope: String,
    pub content: String,
}

impl SkillResource {
    #[must_use]
    pub fn command(&self) -> String {
        format!("/skill:{}", self.name)
    }

    #[must_use]
    pub fn invocation_prompt(&self, args: &str) -> String {
        if args.trim().is_empty() {
            format!(
                "Use the `{}` skill.\n\nSkill file: {}\n\n{}",
                self.name, self.source, self.content
            )
        } else {
            format!(
                "Use the `{}` skill with this user input:\n\n{}\n\nSkill file: {}\n\n{}",
                self.name,
                args.trim(),
                self.source,
                self.content
            )
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceBrowserState {
    pub cursor: usize,
    pub loading: bool,
    pub filtered_indices: Vec<usize>,
    pub search: String,
    pub search_active: bool,
}

fn expand_template(template: &str, input: &str) -> String {
    let args = shell_words(input);
    let mut expanded = template
        .replace("$ARGUMENTS", input.trim())
        .replace("$@", input.trim());
    for index in (1..=9).rev() {
        let needle = format!("${index}");
        let value = args.get(index - 1).map_or("", String::as_str);
        expanded = expanded.replace(&needle, value);
    }
    expanded
}

fn shell_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in input.chars() {
        match (quote, ch) {
            (Some(active), c) if c == active => quote = None,
            (None, '\"' | '\'') => quote = Some(ch),
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_expands_arguments() {
        let prompt = PromptResource {
            name: "review".into(),
            description: "Review".into(),
            argument_hint: None,
            source: "test".into(),
            scope: "project".into(),
            content: "Review $1 and $ARGUMENTS".into(),
        };
        assert_eq!(
            prompt.expand("bugs security"),
            "Review bugs and bugs security"
        );
    }
}
