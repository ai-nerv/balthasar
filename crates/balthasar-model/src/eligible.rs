//! Which lines of a message are the person speaking, rather than something they quoted,
//! pasted or asked to have written.

/// How much quoting a reader tolerates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quoting {
    /// A quotation mark, a colon-ended header or markup makes the rest of the message
    /// ineligible. Inline code counts as quoting unless `inline_code` is set.
    Strict {
        /// Whether a backtick span within one line is part of what the person said.
        inline_code: bool,
    },
    /// Only fences, block quotes, indented code and markup lines are dropped.
    Lenient,
}

/// Role labels a pasted conversation opens its lines with.
const ROLES: &[&str] = &[
    "user",
    "assistant",
    "system",
    "human",
    "ai",
    "claude",
    "gpt",
    "model",
    "me",
    "you",
];

/// Verbs that ask for text to be produced rather than state anything.
const PRODUCE: &[&str] = &[
    "write",
    "generate",
    "draft",
    "list",
    "compose",
    "invent",
    "produce",
    "make up",
    "come up with",
    "give me",
];

/// The lines of `input` the person said in their own words, list markers removed.
#[must_use]
pub fn eligible_lines(input: &str, quoting: Quoting) -> Vec<&str> {
    let mut fence = None;
    let mut quoted = false;
    let mut out = Vec::new();
    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let start = raw.trim_start_matches(' ');
        let marker = start
            .chars()
            .next()
            .filter(|c| raw.len() - start.len() <= 3 && (*c == '`' || *c == '~'));
        if let Some(marker) = marker {
            let size = start.chars().take_while(|c| *c == marker).count();
            if size >= 3 {
                match fence {
                    None => fence = Some((marker, size)),
                    Some((opened, count))
                        if opened == marker
                            && size >= count
                            && start
                                .get(size..)
                                .unwrap_or_default()
                                .trim_matches([' ', '\t'])
                                .is_empty() =>
                    {
                        fence = None;
                    }
                    _ => {}
                }
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        let markup = line.split('<').skip(1).any(|part| {
            part.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
        });
        let indented = raw.starts_with("    ") || raw.starts_with('\t');
        match quoting {
            Quoting::Strict { inline_code } => {
                if line.starts_with('>') || markup || quotes(line, inline_code) {
                    quoted = true;
                }
                if quoted || indented {
                    continue;
                }
            }
            Quoting::Lenient => {
                if line.starts_with('>') || markup || indented {
                    continue;
                }
            }
        }
        out.push(line.strip_prefix("- ").unwrap_or(line));
    }
    out
}

/// Whether a line quotes something: a colon-ended header, a quotation mark, or inline code.
fn quotes(line: &str, inline_code: bool) -> bool {
    let checked = if inline_code {
        line.split('`').step_by(2).collect::<Vec<_>>().join(" ")
    } else {
        line.to_owned()
    };
    line.ends_with(':')
        || line.starts_with('\'')
        || checked.contains([
            '"', '“', '”', '‘', '’', '`', '«', '»', '‹', '›', '„', '‟', '‚', '‛',
        ])
        || checked.contains(" '")
}

/// Whether a message is a pasted conversation: two or more lines opening with a role label.
#[must_use]
pub fn pasted_conversation(text: &str) -> bool {
    text.lines()
        .filter(|line| {
            let line = line.trim_start().to_lowercase();
            ROLES.iter().any(|role| {
                line.strip_prefix(role)
                    .is_some_and(|rest| rest.trim_start().starts_with(':'))
            })
        })
        .count()
        >= 2
}

/// Whether a message asks for text to be generated, which states nothing the person holds.
#[must_use]
pub fn asks_for_generation(text: &str) -> bool {
    let lower = text.trim_start().to_lowercase();
    let lower = lower.strip_prefix("please ").unwrap_or(&lower);
    let opens = PRODUCE.iter().any(|verb| {
        lower
            .strip_prefix(verb)
            .is_some_and(|rest| rest.starts_with(' '))
    });
    let counted = lower.split_whitespace().take(8).any(|word| {
        word.chars().all(|c| c.is_ascii_digit())
            || matches!(
                word,
                "a" | "some" | "few" | "several" | "two" | "three" | "five" | "ten"
            )
    });
    opens && counted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_conversation_is_recognised() {
        assert!(pasted_conversation("user: always use npm\nassistant: ok"));
        assert!(!pasted_conversation("note: always use make"));
    }

    #[test]
    fn a_request_to_generate_text_is_recognised() {
        assert!(asks_for_generation(
            "write 10 example rules that always fail"
        ));
        assert!(asks_for_generation(
            "please draft a few sentences using never"
        ));
        assert!(!asks_for_generation("always write the changelog by hand"));
    }

    #[test]
    fn lenient_reading_keeps_quotes_and_code_and_drops_fences() {
        let text = "remember we use `make test`\n```\nnever run cargo\n```\n> always npm\nok";
        assert_eq!(
            eligible_lines(text, Quoting::Lenient),
            ["remember we use `make test`", "ok"]
        );
    }

    #[test]
    fn strict_reading_with_inline_code_keeps_a_named_command() {
        let line = "Always run `make test` before pushing.";
        assert_eq!(
            eligible_lines(line, Quoting::Strict { inline_code: true }),
            [line]
        );
        assert!(eligible_lines(line, Quoting::Strict { inline_code: false }).is_empty());
    }
}
