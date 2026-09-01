//! Being told to remember something, in the words people actually use.
//!
//! The first version of this anchored on a marker at the start of the turn, which was tidy and
//! wrong. Real instructions arrive as
//!
//! ```text
//!   DUUUUDE FUKING REMEBER THIS SHIT COS YOU ARE ASKIG ME 100 TIMES
//!   please remember we use make
//!   for fuck's sake stop asking, it's `make test`
//! ```
//!
//! and an anchored, exactly-spelled marker catches none of them. Someone shouting at an agent
//! is not writing a well-formed directive; they are annoyed, and being annoyed is *itself* the
//! signal — a person saying "you ask me every time" is telling you the agent has already failed
//! at this repeatedly, which is a stronger reason to keep something than a calm mention.
//!
//! Two things are worked out here: whether the turn is an instruction at all, and whether the
//! claim is *in* the turn or somewhere behind it. "REMEMBER THIS" carries no claim; what it
//! means is the last thing that happened.

/// What a turn was asking for.
#[derive(Debug, Clone, PartialEq)]
pub struct Instruction {
    /// The claim, when the turn carries one.
    pub claim: Option<String>,
    /// Whether the person was insisting.
    ///
    /// Which means they have asked before. An insistent instruction is worth more than a calm
    /// one, not less: the agent's failure is part of the evidence.
    pub insistent: bool,
    /// Whether the claim points backwards rather than carrying itself.
    ///
    /// "Remember *this*" names nothing. What it means is the conversation, and the caller has
    /// to go and get it.
    pub referential: bool,
}

/// Words that ask for something to be kept.
const KEEP: &[&str] = &[
    "remember",
    "dont forget",
    "do not forget",
    "keep in mind",
    "make a note",
    "note that",
    "always",
    "never",
    "from now on",
];

/// Words that mean the person has asked before.
///
/// Instructions in their own right. "Stop asking, it's `make test`" contains no marker from
/// `KEEP` and is unmistakably one.
const INSIST: &[&str] = &[
    "stop asking",
    "every time",
    "how many times",
    "for the last time",
    "i keep telling you",
    "i already told you",
    "asking me",
    "asked you",
    "times now",
];

/// Uses of a marker that are not instructions.
///
/// "I can never remember which flag it is" is somebody's aside, not a directive, and a rule
/// that took it would fill a store with the things a person finds hard.
const REPORTED: &[&str] = &[
    "can never",
    "cannot",
    "can not",
    // Apostrophes become spaces on the way in, so the contracted forms are spelled as they
    // arrive rather than as they were typed.
    "can t",
    "cant",
    "could never",
    "couldn t",
    "do you",
    "did you",
    "don t",
    "dont",
    "do not",
    "doesn t",
    "didn t",
    "hard to",
    "difficult to",
    "trying to",
    "unable to",
    "i never",
    "if you",
];

/// Words that point at the conversation rather than naming anything.
const POINTING: &[&str] = &["this", "that", "it", "these", "those", "them"];

/// What a turn says when it is complaining rather than stating.
///
/// The distinction that settles "remember THIS SHIT COS YOU ARE ASKING ME 100 TIMES": a turn
/// that is *about the agent* is pointing backwards at what the agent got wrong, and a turn that
/// is about the world carries its own claim. Second person and a causal connective are what
/// tell them apart.
const COMPLAINT: &[&str] = &[
    "cos",
    "coz",
    "because",
    "since",
    "you",
    "u ",
    "your",
    "i have",
    "i already",
    "i told",
    "i keep",
    "im tired",
    "i am tired",
    "again",
    "ffs",
];

/// Read one user turn.
///
/// `extra` is whatever the configuration added to the shipped list.
#[must_use]
pub fn read(text: &str, extra: &[String]) -> Option<Instruction> {
    let (normalised, offsets) = normalise(text);
    let insistent = is_insistent(text, &normalised);

    let mut markers: Vec<String> = KEEP.iter().map(|m| (*m).to_owned()).collect();
    markers.extend(extra.iter().map(|m| squash(&m.to_lowercase())));
    markers.extend(INSIST.iter().map(|m| (*m).to_owned()));

    // Anywhere in the turn, not only at the start. "Please remember" and "DUDE REMEMBER" are
    // the ordinary shapes and both put words in front of the marker.
    let (at, found) = find(&normalised, &markers)?;
    if is_reported(&normalised, at, found.len()) {
        return None;
    }

    // A question asks; it does not tell. "Does it always fail like that?" contains a marker
    // and instructs nobody. The exception is somebody who is out of patience — "how many times
    // do I have to tell you?" is a question in punctuation only.
    if text.trim_end().ends_with('?') && !INSIST.iter().any(|m| normalised.contains(m)) {
        return None;
    }

    // Back into the text as it was written. Matching wants it lowercased and de-punctuated;
    // nobody wants `it s make test` in their store, and a claim should read the way somebody
    // said it — backticks, apostrophes and all.
    let after = offsets.get(at + found.len()).copied().unwrap_or(text.len());
    let claim = sentence(text[after..].trim_start_matches([',', ':', '.', '!', ' ', '-', '\t']));

    // A claim that names nothing is not a claim. What "remember this" means is the
    // conversation, and only the caller can go and get it.
    let (claim, referential) = substantive(&claim);

    Some(Instruction {
        claim: (!referential).then_some(claim),
        insistent,
        referential,
    })
}

/// What is left of a claim once the pointing and the complaining are taken off, and whether
/// anything is.
///
/// "remember this: we use make" carries a claim behind a demonstrative and keeps it. "REMEMBER
/// THIS SHIT COS YOU ARE ASKING ME 100 TIMES" does not: strip the demonstrative and the
/// swearing and what remains is a complaint about the agent, which points backwards at whatever
/// the agent got wrong rather than saying anything about the world.
///
/// A heuristic, and named as one. What it gets wrong it gets wrong in the safe direction: a
/// claim mistaken for a reference makes aeon look at the conversation, which is where the
/// answer was anyway.
fn substantive(claim: &str) -> (String, bool) {
    let mut rest = claim.trim();

    loop {
        let flat = squash(&rest.to_lowercase());
        let head = flat
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap_or("");
        if head.is_empty() {
            break;
        }
        if POINTING.contains(&head) || SWEARING.contains(&head) {
            // Unless it is half of a contraction. "it's make test" is a statement whose
            // subject happens to be a pronoun; stripping the "it" out of it left "s make
            // test", which is not anything.
            let after = rest[head.len().min(rest.len())..].chars().next();
            if matches!(after, Some('\'' | '\u{2019}')) {
                break;
            }
            rest = rest[head.len().min(rest.len())..]
                .trim_start_matches(|c: char| !c.is_alphanumeric())
                .trim();
            continue;
        }
        break;
    }

    if rest.len() < 3 {
        return (claim.to_owned(), true);
    }
    let flat = squash(&rest.to_lowercase());
    if COMPLAINT.iter().any(|word| flat.starts_with(word)) {
        return (claim.to_owned(), true);
    }
    (rest.to_owned(), false)
}

/// The turn, lowercased and de-punctuated, with a byte offset into the original for every
/// character.
///
/// Length-preserving in *characters*, so a match found here can be turned back into a position
/// in what the person actually typed. Collapsing repeated letters happens later, and only for
/// the fuzzy word match, because it is the one thing that cannot preserve alignment.
fn normalise(text: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut offsets = Vec::with_capacity(text.len() + 1);
    for (at, c) in text.char_indices() {
        offsets.push(at);
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else {
            out.push(' ');
        }
    }
    offsets.push(text.len());
    // A multi-character lowercase mapping would break the alignment. Vanishingly rare in a
    // command line and worth being right about rather than assuming away.
    if out.chars().count() != offsets.len() - 1 {
        let plain: String = text
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect();
        return (plain.to_lowercase(), offsets);
    }
    (out, offsets)
}

/// A word with its stretched letters collapsed: `duuuude` becomes `duude`.
///
/// Two of a letter is a real word — `all`, `less`, `keep`. Three in a row is somebody leaning
/// on it, and what is left after collapsing is close enough for a one-edit match to reach.
fn squash(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = ('\0', 0_usize);
    for c in text.chars() {
        if c == run.0 {
            run.1 += 1;
            if run.1 >= 2 {
                continue;
            }
        } else {
            run = (c, 0);
        }
        out.push(c);
    }
    out
}

/// Whether the person was insisting.
///
/// Shouting, stretching a word out, exclaiming, swearing, or saying outright that they have
/// asked before. Any of them means this is not the first time.
fn is_insistent(raw: &str, normalised: &str) -> bool {
    if INSIST.iter().any(|m| normalised.contains(m)) {
        return true;
    }
    if raw.contains('!') {
        return true;
    }
    // Somebody counting. "you ask me 100 times" is a person keeping score.
    if normalised.contains("times") && normalised.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    if SWEARING.iter().any(|word| normalised.contains(word)) {
        return true;
    }
    // Shouting: a run of capitals long enough not to be an acronym.
    if raw
        .split_whitespace()
        .any(|word| word.chars().filter(|c| c.is_uppercase()).count() >= 4)
    {
        return true;
    }
    // A stretched word: `duuuude`, `pleeease`.
    raw.to_lowercase()
        .as_bytes()
        .windows(3)
        .any(|three| three[0] == three[1] && three[1] == three[2] && three[0].is_ascii_alphabetic())
}

/// Enough to notice somebody is cross. Not a filter, and never used to refuse anything.
const SWEARING: &[&str] = &["fuck", "fuk", "shit", "damn", "bloody", "christ", "goddamn"];

/// Where a marker is, allowing for one typo.
///
/// Answers a position in the normalised text and the run of it that matched, so the caller can
/// map straight back into what was typed.
fn find(text: &str, markers: &[String]) -> Option<(usize, String)> {
    let mut best: Option<(usize, String)> = None;
    for marker in markers {
        if let Some(at) = text.find(marker.as_str())
            && best.as_ref().is_none_or(|(held, _)| at < *held)
        {
            best = Some((at, marker.clone()));
        }
    }
    if best.is_some() {
        return best;
    }

    // Nothing matched exactly. People misspell the word they are shouting, and `remeber` is
    // one keystroke from `remember` — near enough to act on, and long enough that a
    // single-edit match will not catch an unrelated word.
    let words: Vec<(usize, &str)> = word_positions(text);
    for marker in markers {
        if marker.contains(' ') || marker.len() < 6 {
            continue;
        }
        for (at, word) in &words {
            if within_one_edit(&squash(word), marker) {
                return Some((*at, (*word).to_owned()));
            }
        }
    }
    None
}

/// Every word and where it starts.
fn word_positions(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut at = 0;
    for word in text.split(' ') {
        if !word.is_empty() {
            out.push((at, word));
        }
        at += word.len() + 1;
    }
    out
}

/// Whether two words differ by at most one insertion, deletion or substitution.
fn within_one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };

    let mut i = 0;
    let mut j = 0;
    let mut slack = 1;
    while i < short.len() && j < long.len() {
        if short[i] == long[j] {
            i += 1;
            j += 1;
            continue;
        }
        if slack == 0 {
            return false;
        }
        slack -= 1;
        if short.len() == long.len() {
            i += 1;
        }
        j += 1;
    }
    slack >= long.len() - j
}

/// Whether the marker is being reported rather than issued.
///
/// The window covers the marker itself, because the giveaway usually straddles it: in "I can
/// never remember", the phrase that settles it is "can never" and the marker is "never".
fn is_reported(text: &str, at: usize, len: usize) -> bool {
    let from = at.saturating_sub(24);
    let to = (at + len).min(text.len());
    let window = &text[from..to];
    REPORTED.iter().any(|phrase| window.contains(phrase))
}

/// The first sentence of what follows a marker.
///
/// A full stop only ends a sentence before whitespace or at the end. `10.0.0.7` and `fly.io`
/// are one word each, and splitting on every dot truncated them at the first digit.
fn sentence(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        let ends = match c {
            '\n' | '!' | '?' => true,
            '.' => chars.get(i + 1).is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if ends {
            return chars[..i].iter().collect::<String>().trim().to_owned();
        }
    }
    text.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_it(text: &str) -> Option<Instruction> {
        read(text, &[])
    }

    #[test]
    fn somebody_shouting_a_misspelled_instruction_is_caught() {
        // The turn this whole module was rewritten for. An anchored, exactly-spelled marker
        // caught none of it.
        let said = read_it("DUUUUDE FUKING REMEBER THIS SHIT COS YOU ARE ASKIG ME 100 TIMES")
            .expect("an instruction");
        assert!(said.insistent, "they are plainly not asking politely");
        assert!(said.referential, "'this shit' names nothing on its own");
    }

    #[test]
    fn a_marker_in_the_middle_of_a_turn_is_found() {
        let said = read_it("please remember we use make").expect("an instruction");
        assert_eq!(said.claim.as_deref(), Some("we use make"));
    }

    #[test]
    fn a_calm_instruction_is_still_an_instruction() {
        let said = read_it("remember: we deploy with fly").expect("an instruction");
        assert_eq!(said.claim.as_deref(), Some("we deploy with fly"));
        assert!(!said.insistent);
    }

    #[test]
    fn insisting_is_an_instruction_with_no_marker_at_all() {
        // "Stop asking, it's make test" contains nothing from the keep list and is
        // unmistakably a directive.
        let said = read_it("for fuck's sake stop asking, it's make test").expect("an instruction");
        assert!(said.insistent);
        assert_eq!(said.claim.as_deref(), Some("it's make test"));
    }

    #[test]
    fn an_address_survives_being_read() {
        let said = read_it("remember: the staging box is at 10.0.0.7").expect("an instruction");
        assert_eq!(
            said.claim.as_deref(),
            Some("the staging box is at 10.0.0.7")
        );
    }

    #[test]
    fn a_claim_is_kept_the_way_it_was_written() {
        // Matching needs it lowercased and de-punctuated. Nobody wants `it s make test` in
        // their store, and a claim should read the way somebody said it.
        let said = read_it("Please REMEMBER we use `make test` here").expect("an instruction");
        assert_eq!(said.claim.as_deref(), Some("we use `make test` here"));
    }

    #[test]
    fn an_aside_about_forgetting_is_not_a_directive() {
        // A rule that took this would fill a store with the things a person finds hard.
        for aside in [
            "I can never remember which flag it is",
            "do you remember what we did last time",
            "it's hard to remember all of this",
            "I don't remember the port",
        ] {
            assert!(read_it(aside).is_none(), "{aside}");
        }
    }

    #[test]
    fn an_ordinary_question_is_not_an_instruction() {
        // A question asks; it does not tell. Several of these contain markers and instruct
        // nobody.
        for question in [
            "what does this function do?",
            "does it always fail like that?",
            "should I remember the port?",
            "can you run the tests",
            "the build is slow today",
        ] {
            assert!(read_it(question).is_none(), "{question}");
        }
    }

    #[test]
    fn a_question_from_somebody_out_of_patience_is_an_instruction() {
        // "How many times do I have to tell you?" is a question in punctuation only.
        let said = read_it("how many times do I have to tell you it's make test?")
            .expect("an instruction");
        assert!(said.insistent);
    }

    #[test]
    fn shouting_alone_counts_as_insisting() {
        let said = read_it("REMEMBER WE USE MAKE").expect("an instruction");
        assert!(said.insistent);
    }

    #[test]
    fn an_exclamation_counts() {
        let said = read_it("remember we use make!").expect("an instruction");
        assert!(said.insistent);
    }

    #[test]
    fn counting_the_times_counts() {
        let said = read_it("remember this, I have told you 4 times").expect("an instruction");
        assert!(said.insistent);
    }

    #[test]
    fn a_referential_instruction_carries_no_claim() {
        // What "remember this" means is the conversation, and only the caller can go and get
        // it. Storing the word "this" would be worse than storing nothing.
        for pointing in [
            "REMEMBER THIS",
            "remember that",
            "remember it",
            "remember this shit cos you keep asking",
            "REMEMBER THAT!! you asked me again",
        ] {
            let said = read_it(pointing).expect("an instruction");
            assert!(said.referential, "{pointing}");
            assert_eq!(said.claim, None, "{pointing}");
        }
    }

    #[test]
    fn a_pronoun_in_a_contraction_is_not_a_pointing_word() {
        // "it's make test" is a statement whose subject happens to be a pronoun. Stripping the
        // "it" out of it leaves "s make test", which is not anything.
        let said = read_it("stop asking, it's make test").expect("an instruction");
        assert_eq!(said.claim.as_deref(), Some("it's make test"));
    }

    #[test]
    fn a_claim_behind_a_demonstrative_is_still_a_claim() {
        // "remember this: we use make" points and then says. Treating it as referential would
        // throw away a perfectly good fact.
        let said = read_it("remember this: we use make").expect("an instruction");
        assert!(!said.referential);
        assert_eq!(said.claim.as_deref(), Some("we use make"));
    }

    #[test]
    fn a_typo_of_one_keystroke_is_forgiven() {
        assert!(read_it("remeber we use make").is_some());
        assert!(read_it("rememberr we use make").is_some());
    }

    #[test]
    fn an_unrelated_word_is_not_forgiven_into_a_marker() {
        // A single-edit match on short words would take half the language.
        assert!(read_it("the number is 5").is_none());
        assert!(
            read_it("we never got round to it").is_some(),
            "'never' is a real marker"
        );
    }

    #[test]
    fn a_configuration_may_add_its_own_words() {
        let said = read("merk das: we use make", &["merk das".to_owned()]).expect("an instruction");
        assert_eq!(said.claim.as_deref(), Some("we use make"));
    }

    #[test]
    fn a_stretched_word_squashes_to_something_matchable() {
        assert_eq!(squash("duuuude"), "duude");
        assert_eq!(squash("remeber"), "remeber");
    }

    #[test]
    fn squashing_keeps_ordinary_doubled_letters() {
        // `all`, `less`, `keep` are words. Only three in a row is somebody leaning on it.
        assert_eq!(squash("keep all of it"), "keep all of it");
    }

    #[test]
    fn a_match_maps_back_to_where_it_was_typed() {
        // The whole reason normalisation preserves length: a claim has to come back in the
        // words somebody used.
        let (normalised, offsets) = normalise("Please REMEMBER it's `make`");
        assert_eq!(normalised, "please remember it s  make ");
        assert_eq!(
            offsets.len(),
            "Please REMEMBER it's `make`".chars().count() + 1
        );
    }
}
