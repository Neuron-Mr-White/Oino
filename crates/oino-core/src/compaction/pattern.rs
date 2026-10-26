//! Minimal case-insensitive backtracking matcher for pi's error-classification regexes.
//!
//! pi's tables use JavaScript syntax including negative look-behind, which the `regex` crate
//! does not support, so the subset they need is implemented here: literals, `.`, `\d \D \w
//! \W \s \S`, escaped punctuation, classes with ranges, `^`/`$`, groups `( )`/`(?: )` with
//! alternation, `(?! )`, `(?<! )`, and greedy `? * +`. Semantics follow JS `RegExp#test`
//! with the `i` flag (ASCII case folding, otherwise exact).

use thiserror::Error;

/// A pattern that is not in the supported subset.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("unsupported pattern {source_text:?} at offset {offset}: {reason}")]
pub(crate) struct PatternError {
    source_text: String,
    offset: usize,
    reason: &'static str,
}

/// A compiled pattern (search semantics, case-insensitive).
#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    alternatives: Vec<Vec<Node>>,
}

#[derive(Clone, Debug)]
enum Atom {
    Char(char),
    Any,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
}

#[derive(Clone, Debug)]
enum ClassItem {
    Char(char),
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

#[derive(Clone, Debug)]
enum Node {
    Atom(Atom),
    Start,
    End,
    Group(Vec<Vec<Node>>),
    NotAhead(Vec<Vec<Node>>),
    NotBehind(Vec<Vec<Node>>),
    Repeat {
        node: Box<Node>,
        min: usize,
        max: Option<usize>,
    },
}

const fn is_line_terminator(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

const fn is_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_space(ch: char) -> bool {
    ch.is_whitespace() || ch == '\u{feff}'
}

fn fold(ch: char) -> char {
    ch.to_ascii_lowercase()
}

impl ClassItem {
    fn matches(&self, ch: char) -> bool {
        match *self {
            Self::Char(c) => fold(c) == fold(ch),
            Self::Range(low, high) => {
                let in_range = |c: char| low <= c && c <= high;
                in_range(ch)
                    || in_range(ch.to_ascii_lowercase())
                    || in_range(ch.to_ascii_uppercase())
            }
            Self::Digit(negated) => ch.is_ascii_digit() != negated,
            Self::Word(negated) => is_word(ch) != negated,
            Self::Space(negated) => is_space(ch) != negated,
        }
    }
}

impl Atom {
    fn matches(&self, ch: char) -> bool {
        match self {
            Self::Char(c) => fold(*c) == fold(ch),
            Self::Any => !is_line_terminator(ch),
            Self::Class { negated, items } => items.iter().any(|item| item.matches(ch)) != *negated,
        }
    }
}

struct Parser<'a> {
    source: &'a str,
    chars: Vec<char>,
    pos: usize,
}

impl Parser<'_> {
    fn error(&self, reason: &'static str) -> PatternError {
        PatternError {
            source_text: self.source.to_owned(),
            offset: self.pos,
            reason,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn eat(&mut self, expected: &str) -> bool {
        let wanted: Vec<char> = expected.chars().collect();
        if self.chars[self.pos..].starts_with(&wanted) {
            self.pos += wanted.len();
            true
        } else {
            false
        }
    }

    fn alternation(&mut self) -> Result<Vec<Vec<Node>>, PatternError> {
        let mut alternatives = vec![self.sequence()?];
        while self.eat("|") {
            alternatives.push(self.sequence()?);
        }
        Ok(alternatives)
    }

    fn sequence(&mut self) -> Result<Vec<Node>, PatternError> {
        let mut nodes = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' {
                break;
            }
            let node = self.atom()?;
            nodes.push(self.quantifier(node)?);
        }
        Ok(nodes)
    }

    fn quantifier(&mut self, node: Node) -> Result<Node, PatternError> {
        let (min, max) = match self.peek() {
            Some('?') => (0, Some(1)),
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('{') => return Err(self.error("counted repetition")),
            _ => return Ok(node),
        };
        self.pos += 1;
        if self.peek() == Some('?') {
            return Err(self.error("lazy quantifier"));
        }
        if matches!(
            node,
            Node::Start | Node::End | Node::NotAhead(_) | Node::NotBehind(_)
        ) {
            return Err(self.error("quantified assertion"));
        }
        Ok(Node::Repeat {
            node: Box::new(node),
            min,
            max,
        })
    }

    fn atom(&mut self) -> Result<Node, PatternError> {
        let Some(ch) = self.peek() else {
            return Err(self.error("unexpected end"));
        };
        self.pos += 1;
        match ch {
            '^' => Ok(Node::Start),
            '$' => Ok(Node::End),
            '.' => Ok(Node::Atom(Atom::Any)),
            '(' => self.group(),
            '[' => self.class(),
            '\\' => self.escape().map(|item| match item {
                ClassItem::Char(c) => Node::Atom(Atom::Char(c)),
                other => Node::Atom(Atom::Class {
                    negated: false,
                    items: vec![other],
                }),
            }),
            '*' | '+' | '?' | '{' | '}' | ']' => Err(self.error("unexpected metacharacter")),
            other => Ok(Node::Atom(Atom::Char(other))),
        }
    }

    fn group(&mut self) -> Result<Node, PatternError> {
        let wrap: fn(Vec<Vec<Node>>) -> Node = if self.eat("?:") {
            Node::Group
        } else if self.eat("?!") {
            Node::NotAhead
        } else if self.eat("?<!") {
            Node::NotBehind
        } else if self.peek() == Some('?') {
            return Err(self.error("unsupported group kind"));
        } else {
            Node::Group
        };
        let inner = self.alternation()?;
        if !self.eat(")") {
            return Err(self.error("unclosed group"));
        }
        Ok(wrap(inner))
    }

    fn escape(&mut self) -> Result<ClassItem, PatternError> {
        let Some(ch) = self.peek() else {
            return Err(self.error("trailing backslash"));
        };
        self.pos += 1;
        Ok(match ch {
            'd' => ClassItem::Digit(false),
            'D' => ClassItem::Digit(true),
            'w' => ClassItem::Word(false),
            'W' => ClassItem::Word(true),
            's' => ClassItem::Space(false),
            'S' => ClassItem::Space(true),
            'n' => ClassItem::Char('\n'),
            't' => ClassItem::Char('\t'),
            'r' => ClassItem::Char('\r'),
            c if c.is_ascii_alphanumeric() => return Err(self.error("unsupported escape")),
            c => ClassItem::Char(c),
        })
    }

    fn class(&mut self) -> Result<Node, PatternError> {
        let negated = self.eat("^");
        let mut items = Vec::new();
        loop {
            let Some(ch) = self.peek() else {
                return Err(self.error("unclosed class"));
            };
            self.pos += 1;
            let item = match ch {
                ']' => break,
                '\\' => self.escape()?,
                c => ClassItem::Char(c),
            };
            let is_range = matches!(item, ClassItem::Char(_))
                && self.peek() == Some('-')
                && self.chars.get(self.pos + 1).is_some_and(|c| *c != ']');
            if let (true, ClassItem::Char(low)) = (is_range, &item) {
                self.pos += 1;
                let high = match self.peek() {
                    Some('\\') => {
                        self.pos += 1;
                        match self.escape()? {
                            ClassItem::Char(c) => c,
                            _ => return Err(self.error("class escape as range end")),
                        }
                    }
                    Some(c) => {
                        self.pos += 1;
                        c
                    }
                    None => return Err(self.error("unclosed class")),
                };
                items.push(ClassItem::Range(*low, high));
            } else {
                items.push(item);
            }
        }
        Ok(Node::Atom(Atom::Class { negated, items }))
    }
}

/// Width in characters when every alternative matches a fixed number of characters.
fn fixed_width(alternatives: &[Vec<Node>]) -> Option<usize> {
    let mut widths = alternatives.iter().map(|sequence| {
        sequence.iter().try_fold(0, |total, node| {
            let width = match node {
                Node::Atom(_) => Some(1),
                Node::Start | Node::End | Node::NotAhead(_) | Node::NotBehind(_) => Some(0),
                Node::Group(inner) => fixed_width(inner),
                Node::Repeat { node, min, max } if Some(*min) == *max => {
                    fixed_width(&[vec![node.as_ref().clone()]]).map(|width| width * min)
                }
                Node::Repeat { .. } => None,
            };
            width.map(|width| total + width)
        })
    });
    let first = widths.next()??;
    widths.all(|width| width == Some(first)).then_some(first)
}

struct Matcher<'t> {
    text: &'t [char],
}

impl Matcher<'_> {
    fn alternatives(
        &self,
        alternatives: &[Vec<Node>],
        pos: usize,
        next: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        alternatives
            .iter()
            .any(|sequence| self.sequence(sequence, pos, &mut *next))
    }

    fn sequence(&self, nodes: &[Node], pos: usize, next: &mut dyn FnMut(usize) -> bool) -> bool {
        let Some((first, rest)) = nodes.split_first() else {
            return next(pos);
        };
        match first {
            Node::Atom(atom) => {
                self.text.get(pos).is_some_and(|ch| atom.matches(*ch))
                    && self.sequence(rest, pos + 1, next)
            }
            Node::Start => pos == 0 && self.sequence(rest, pos, next),
            Node::End => pos == self.text.len() && self.sequence(rest, pos, next),
            Node::Group(alternatives) => self.alternatives(alternatives, pos, &mut |end| {
                self.sequence(rest, end, &mut *next)
            }),
            Node::NotAhead(alternatives) => {
                !self.alternatives(alternatives, pos, &mut |_| true)
                    && self.sequence(rest, pos, next)
            }
            Node::NotBehind(alternatives) => {
                let ends_here =
                    |start: usize| self.alternatives(alternatives, start, &mut |end| end == pos);
                let behind = match fixed_width(alternatives) {
                    Some(width) => pos.checked_sub(width).is_some_and(ends_here),
                    None => (0..=pos).rev().any(ends_here),
                };
                !behind && self.sequence(rest, pos, next)
            }
            Node::Repeat { node, min, max } => match node.as_ref() {
                Node::Atom(atom) => self.repeat_atom(atom, *min, *max, pos, rest, next),
                other => self.repeat_node(other, *min, *max, 0, pos, rest, next),
            },
        }
    }

    fn repeat_atom(
        &self,
        atom: &Atom,
        min: usize,
        max: Option<usize>,
        pos: usize,
        rest: &[Node],
        next: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        let limit = max.unwrap_or(usize::MAX);
        let count = self.text[pos.min(self.text.len())..]
            .iter()
            .take(limit)
            .take_while(|ch| atom.matches(**ch))
            .count();
        count >= min
            && (min..=count)
                .rev()
                .any(|taken| self.sequence(rest, pos + taken, &mut *next))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "recursive helper threading the repetition state"
    )]
    fn repeat_node(
        &self,
        node: &Node,
        min: usize,
        max: Option<usize>,
        count: usize,
        pos: usize,
        rest: &[Node],
        next: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        let can_repeat = max.is_none_or(|max| count < max);
        if can_repeat {
            let once = std::slice::from_ref(node);
            let matched = self.sequence(once, pos, &mut |end| {
                // A zero-width iteration cannot make progress; stop repeating.
                end != pos && self.repeat_node(node, min, max, count + 1, end, rest, &mut *next)
            });
            if matched {
                return true;
            }
        }
        count >= min && self.sequence(rest, pos, next)
    }
}

impl Pattern {
    /// Compiles `source` (JS regex syntax, `i` flag implied).
    pub(crate) fn new(source: &str) -> Result<Self, PatternError> {
        let mut parser = Parser {
            source,
            chars: source.chars().collect(),
            pos: 0,
        };
        let alternatives = parser.alternation()?;
        if parser.pos != parser.chars.len() {
            return Err(parser.error("unbalanced parenthesis"));
        }
        Ok(Self { alternatives })
    }

    /// JS `RegExp#test`: whether the pattern matches anywhere in `text`.
    pub(crate) fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        let matcher = Matcher { text: &chars };
        (0..=chars.len())
            .any(|start| matcher.alternatives(&self.alternatives, start, &mut |_| true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pattern: &str, text: &str) -> bool {
        Pattern::new(pattern).unwrap().is_match(text)
    }

    #[test]
    fn subset_semantics() {
        assert!(m("prompt (?:is )?too long", "PROMPT too long"));
        assert!(m(r"[\d,]+ tokens?", "1,000 token"));
        assert!(m(r"(?<![a-z0-9])(?:429|500)(?![a-z0-9])", "HTTP 429!"));
        assert!(!m(r"(?<![a-z0-9])(?:429|500)(?![a-z0-9])", "req_4290"));
        assert!(!m(r"(?<![a-z0-9])(?:429|500)(?![a-z0-9])", "x500"));
        assert!(m(r"^(?:[\w -]+ API error \(413\):|413(?:\s|:|$))", "413"));
        assert!(m(
            r"^(?:[\w -]+ API error \(413\):|413(?:\s|:|$))",
            "github-copilot API error (413): x"
        ));
        assert!(!m(r"^413(?:\s|:|$)", "4130"));
        assert!(m(
            r"input token count.*exceeds the maximum",
            "Input token count (5) exceeds the maximum"
        ));
        assert!(!m(r"a.b", "a\nb"));
        assert!(m(
            r"request to \S+ failed, reason:",
            "request to https://x failed, reason:"
        ));
        assert!(m("(?:a.?)+b", "aaxab"));
        assert!(m("^$", ""));
    }

    #[test]
    fn rejects_unsupported_syntax() {
        for bad in ["a{2}", "a+?", "(?=x)", "[a", "(a", "a)", r"\b", "*"] {
            assert!(Pattern::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn long_inputs_do_not_recurse_per_character() {
        let text = "x".repeat(200_000);
        assert!(!m("input token count.*exceeds", &text));
        assert!(!m(r"(?<![a-z0-9])(?:429|500)(?![a-z0-9])", &text));
        assert!(m("x+$", &text));
    }
}
