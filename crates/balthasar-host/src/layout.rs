//! `layout`, `applied` and `overflowed`: what one request holds, what the harness confirmed it
//! sent, and a tighter answer when the provider refused one as too long.

use crate::supply::{self, Supplied};
use crate::window::stub_for;
use crate::{Answering, Hooks};
use balthasar_buffer::{Ask, Compacting, Covered, Held, Layout, Row, Rules, Slot, corrected};
use balthasar_ipc::{Reply, Request};
use balthasar_model::SessionId;
use balthasar_store::{Prompt, State, StoreError, Turn};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// How many search hits the memory slot is packed from.
const CANDIDATES: usize = 20;

/// The model a factor is kept under when the run never said which it talks to.
const UNNAMED: &str = "default";

/// What a reply is assumed to need when the harness does not say.
const REPLY: u32 = 8_192;

/// `layout(session, request)`: what the next request should hold.
pub fn layout(at: &mut Answering<'_>, request: &Request, hooks: &mut dyn Hooks) -> Reply {
    let Some(session) = session_of(request) else {
        return Reply::refused("layout needs a session");
    };
    let asked = request.args.get(1).cloned().unwrap_or_else(|| json!({}));
    match lay_out(at, &session, &asked, hooks) {
        Ok(laid) => Reply::one(laid),
        Err(why) => Reply::refused(why),
    }
}

/// `applied(session, {id, usage})`: the provider accepted what layout `id` described.
pub fn applied(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = session_of(request) else {
        return Reply::refused("applied needs a session");
    };
    let said = request.args.get(1);
    let Some(id) = said.and_then(|s| s.as_str().or_else(|| s.get("id")?.as_str())) else {
        return Reply::refused("applied needs the layout's id");
    };
    match confirm(at, &session, id, said.and_then(|s| s.get("usage"))) {
        Ok(()) => Reply::none(),
        Err(why) => Reply::refused(why),
    }
}

/// `overflowed(session, {id, said})`: the provider refused layout `id` as too long.
pub fn overflowed(at: &mut Answering<'_>, request: &Request, hooks: &mut dyn Hooks) -> Reply {
    let Some(session) = session_of(request) else {
        return Reply::refused("overflowed needs a session");
    };
    let said = request.args.get(1);
    let Some(id) = said.and_then(|s| s.as_str().or_else(|| s.get("id")?.as_str())) else {
        return Reply::refused("overflowed needs the layout's id");
    };
    let message = said
        .and_then(|s| s.get("said"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let asked = match tighten(at, &session, id, message) {
        Ok(asked) => asked,
        Err(why) => return Reply::refused(why),
    };
    match lay_out(at, &session, &asked, hooks) {
        Ok(laid) => Reply::one(laid),
        Err(why) => Reply::refused(why),
    }
}

/// Lay out one request, keep it as proposed, and answer it.
fn lay_out(
    at: &mut Answering<'_>,
    session: &SessionId,
    asked: &Value,
    hooks: &mut dyn Hooks,
) -> Result<Value, String> {
    let rules = hooks.window();
    let query = asked["query"].as_str().unwrap_or_default().to_owned();
    let (model, factor, turns, summary, mut prompt) = gather(at, session, asked, &query)?;
    let ask = Ask {
        round: number(asked, "round").unwrap_or(0),
        window: number(asked, "window")
            .or(model.1)
            .unwrap_or(balthasar_buffer::Window::default().size),
        reply: number(asked, "reply").unwrap_or(REPLY),
        fixed: fixed(asked),
        idle_s: asked["idle_s"].as_u64(),
        tight: prompt.overflows,
    };
    let caps = balthasar_buffer::budget(&rules, &ask, factor);
    let (pinned, pinned_tokens) = crate::notes::pinned_slot(
        at,
        &mut prompt,
        (f64::from(caps.pinned) / factor) as u32,
        rules.estimate_chars_per_token,
    )
    .map_err(|e| e.to_string())?;

    // Settled once per prompt, so every round of it sends the same memory.
    let memory = if let Some(kept) = Supplied::kept(prompt.memory.as_ref(), &query) {
        kept
    } else {
        let found = supply::candidates(at, &query, CANDIDATES);
        let room = (f64::from(caps.memory) / factor) as u32;
        let packed = supply::pack(at, &found, room, rules.estimate_chars_per_token, hooks)
            .unwrap_or_default();
        prompt.memory = Some(packed.as_json(&query));
        packed
    };

    let known = at
        .scrollback
        .as_ref()
        .map_or(Ok(Vec::new()), |s| s.jobs_of(session))
        .map_err(|e| e.to_string())?;
    let mut stubs: HashMap<u64, String> = HashMap::new();
    let rows = rows_of(&turns, &rules, hooks, &mut stubs);
    let held = Held {
        summary: summary.as_ref().map(|s| Covered {
            from: s.from,
            to: s.to,
            tokens: rules.estimate(&s.text),
        }),
        pinned: pinned_tokens,
        memory: if memory.ids.is_empty() {
            0
        } else {
            memory.tokens
        },
        note: rules.estimate(&rules.warning),
        compacting: if crate::queue::pending(&known, "summarise", at.now) {
            Compacting::Pending
        } else if prompt.compactions >= rules.max_compactions_per_prompt {
            Compacting::Spent
        } else {
            Compacting::Allowed
        },
    };
    let mut laid = balthasar_buffer::lay(&rules, &ask, &rows, &held, factor);
    let mut note = rules.warning.clone();
    by_policy(
        hooks, &mut laid, &rows, &held, &mut stubs, &mut note, factor,
    );

    // A summary when one is due, and a curated memory once per prompt when a helper can do it.
    let e = |e: StoreError| e.to_string();
    if let Some(span) = laid.compact {
        crate::jobs::summarise(
            at,
            session,
            (span.from, span.to),
            &rules,
            laid.budget.summary,
        )
        .map_err(e)?;
        prompt.compactions += 1;
        let keeping = hooks.memory();
        crate::notes::before_cut(at, session, span.to, &prompt.helpers, &keeping).map_err(e)?;
    }
    // This prompt and what came before it, read in the background while it is answered: a rule
    // the person just stated reaches an agent started in this same prompt.
    let asked_now = turns.iter().rev().find(|t| t.role == "user").map(|t| t.cursor);
    if ask.round == 0
        && let Some(upto) = asked_now
    {
        let keeping = hooks.memory();
        crate::notes::background(at, session, &prompt.helpers, Some(upto), &keeping).map_err(e)?;
    }
    let curating = known
        .iter()
        .any(|job| job.kind == "curate" && job.context["mark"].as_str() == Some(&prompt.mark));
    if ask.round == 0
        && !curating
        && !query.trim().is_empty()
        && crate::queue::can_run(&prompt.helpers, "memory")
    {
        let room = (f64::from(caps.memory) / factor) as u32;
        crate::jobs::curate(at, session, &query, &prompt.mark, room, hooks).map_err(e)?;
    }
    let jobs = crate::jobs::hand_out(
        at,
        session,
        (ask.round == 0).then_some(prompt.mark.as_str()),
    )
    .map_err(e)?;

    let fix = |tokens: u32| corrected(tokens, factor);
    let slots: Vec<Value> = laid
        .slots
        .iter()
        .map(|slot| match slot {
            Slot::Pinned => {
                json!({ "kind": "pinned", "text": pinned, "tokens": fix(pinned_tokens) })
            }
            Slot::Summary => {
                let s = summary.as_ref().expect("a summary slot has a summary");
                json!({ "kind": "summary", "text": s.text, "covers": [s.from, s.to],
                        "tokens": fix(rules.estimate(&s.text)) })
            }
            Slot::Item(cursor) => json!({ "kind": "item", "cursor": cursor }),
            Slot::Stub(cursor) => json!({ "kind": "stub", "cursor": cursor,
                                          "text": stubs.get(cursor).cloned().unwrap_or_default() }),
            Slot::Note => json!({ "kind": "note", "text": note }),
            Slot::Memory => json!({ "kind": "memory", "text": memory.text,
                                    "tokens": fix(memory.tokens), "ids": memory.ids }),
        })
        .collect();
    let b = laid.budget;
    let body = json!({
        "budget": {
            "window": b.window, "fixed": b.fixed, "reply": b.reply, "room": b.room,
            "conversation": b.conversation, "memory": b.memory, "pinned": b.pinned,
            "summary": b.summary, "used": b.used, "estimated_input": b.estimated_input,
            "factor": (b.factor * 1000.0).round() / 1000.0,
        },
        "slots": slots,
        "jobs": jobs,
        "fits": laid.fits,
        "why": laid.why,
    });

    let raw = (f64::from(b.estimated_input) / factor).round() as u64;
    let scrollback = at
        .scrollback
        .as_ref()
        .ok_or("laying out needs a scrollback")?;
    let id = scrollback
        .propose(session, &model.0, raw, &body, asked, at.now)
        .map_err(|e| e.to_string())?;
    scrollback
        .keep_prompt(session, &prompt)
        .map_err(|e| e.to_string())?;
    let mut body = body;
    body["id"] = json!(id);
    Ok(body)
}

/// Everything a layout reads from the scrollback, read once.
type Gathered = (
    (String, Option<u32>),
    f64,
    Vec<Turn>,
    Option<balthasar_store::Summary>,
    Prompt,
);

fn gather(
    at: &Answering<'_>,
    session: &SessionId,
    asked: &Value,
    query: &str,
) -> Result<Gathered, String> {
    let scrollback = at
        .scrollback
        .as_ref()
        .ok_or("laying out needs a scrollback")?;
    let e = |e: StoreError| e.to_string();
    let noted = scrollback.model_of(session).map_err(e)?;
    let model = noted.as_ref().map_or(UNNAMED, |(name, _)| name).to_owned();
    let factor = scrollback.measured(&model).map_err(e)?.map_or(1.0, |m| m.0);
    let mut turns = scrollback.outline(session).map_err(e)?;
    if let Some(live) = asked["live"].as_array() {
        let live: HashSet<u64> = live.iter().filter_map(Value::as_u64).collect();
        let known: HashSet<u64> = turns.iter().map(|t| t.cursor).collect();
        turns.retain(|t| live.contains(&t.cursor));
        // A cursor the harness sends but never observed is still sent; it just costs nothing here.
        turns.extend(live.difference(&known).map(|cursor| Turn {
            cursor: *cursor,
            role: "other".into(),
            kind: "prose".into(),
            tokens: Some(0),
            ..Turn::default()
        }));
        turns.sort_by_key(|t| t.cursor);
    }
    if turns.is_empty() {
        return Err(format!(
            "nothing has been observed for '{session}' — stream turns before asking for a layout"
        ));
    }
    let summary = scrollback.summary(session).map_err(e)?;
    let mark = turns
        .iter()
        .rev()
        .find(|t| t.role == "user")
        .map_or_else(|| format!("q:{query}"), |t| t.cursor.to_string());
    let mut prompt = scrollback
        .prompt(session)
        .map_err(e)?
        .filter(|p| p.mark == mark)
        .unwrap_or(Prompt {
            mark,
            ..Prompt::default()
        });
    if let Some(helpers) = asked["helpers"].as_array() {
        prompt.helpers = helpers
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    Ok((
        (model, noted.map(|(_, size)| size)),
        factor,
        turns,
        summary,
        prompt,
    ))
}

/// The rows as the rules see them, with each tool row's stub worked out.
fn rows_of(
    turns: &[Turn],
    rules: &Rules,
    hooks: &mut dyn Hooks,
    stubs: &mut HashMap<u64, String>,
) -> Vec<Row> {
    let mut describe = |turn: &Turn| hooks.stub(turn);
    turns
        .iter()
        .map(|turn| {
            let stub = stub_for(turn, &mut describe);
            let stub_tokens = stub.as_ref().map_or(turn.weight(), |said| {
                rules.estimate(said) + turn.args.as_deref().map_or(0, |a| rules.estimate(a))
            });
            if let Some(said) = stub {
                stubs.insert(turn.cursor, said);
            }
            Row {
                cursor: turn.cursor,
                unit: turn.unit(),
                tokens: turn.weight(),
                stub_tokens,
                tool: turn.is_tool(),
                user: turn.role == "user",
                keep: turn.keep || turn.pinned,
                error: turn.error,
                stubbed: turn.state == State::Masked,
            }
        })
        .collect()
}

/// Let a configured policy lay the request out instead, when it answers with something usable.
fn by_policy(
    hooks: &mut dyn Hooks,
    laid: &mut Layout,
    rows: &[Row],
    held: &Held,
    stubs: &mut HashMap<u64, String>,
    note: &mut String,
    factor: f64,
) {
    let b = laid.budget;
    let budget = json!({ "window": b.window, "fixed": b.fixed, "reply": b.reply, "room": b.room,
        "conversation": b.conversation, "memory": b.memory, "pinned": b.pinned,
        "summary": b.summary, "factor": b.factor });
    let shown: Vec<&Row> = rows
        .iter()
        .filter(|r| {
            !held
                .summary
                .is_some_and(|s| (s.from..=s.to).contains(&r.cursor))
        })
        .collect();
    let items: Vec<Value> = shown
        .iter()
        .map(|r| {
            json!({ "cursor": r.cursor, "group": r.unit, "tokens": r.tokens, "tool": r.tool,
            "user": r.user, "keep": r.keep, "error": r.error, "stub": stubs.get(&r.cursor) })
        })
        .collect();
    let Some(said) = hooks.policy(&budget, &Value::Array(items)) else {
        return;
    };
    let Some(slots) = policy_slots(&said, &shown, held, stubs, note) else {
        laid.why
            .push_str("; the policy's answer was unusable, so the rules decided");
        return;
    };
    let named: HashSet<u64> = slots
        .iter()
        .filter_map(|s| match s {
            Slot::Item(c) | Slot::Stub(c) => Some(*c),
            _ => None,
        })
        .collect();
    laid.stubbed = slots
        .iter()
        .filter_map(|s| match s {
            Slot::Stub(c) => Some(*c),
            _ => None,
        })
        .collect();
    laid.dropped = shown
        .iter()
        .map(|r| r.cursor)
        .filter(|c| !named.contains(c))
        .collect();
    laid.warned = slots.contains(&Slot::Note);
    laid.slots = slots;
    balthasar_buffer::account(laid, rows, held, factor);
    laid.why = said["why"]
        .as_str()
        .unwrap_or("laid out by the configured policy")
        .to_owned();
}

/// A policy's slots, if they keep every rule the harness relies on.
fn policy_slots(
    said: &Value,
    shown: &[&Row],
    held: &Held,
    stubs: &mut HashMap<u64, String>,
    note: &mut String,
) -> Option<Vec<Slot>> {
    let by: HashMap<u64, &Row> = shown.iter().map(|r| (r.cursor, *r)).collect();
    let mut out = Vec::new();
    let mut last = None;
    for slot in said.get("slots")?.as_array()? {
        let text = slot.get("text").and_then(Value::as_str);
        match (
            slot.get("kind")?.as_str()?,
            slot.get("cursor").and_then(Value::as_u64),
        ) {
            (kind @ ("item" | "stub"), Some(cursor)) => {
                let row = by.get(&cursor)?;
                if last.is_some_and(|l| l >= cursor) {
                    return None;
                }
                last = Some(cursor);
                if kind == "item" {
                    out.push(Slot::Item(cursor));
                    continue;
                }
                if !row.tool {
                    return None;
                }
                if let Some(text) = text {
                    stubs.insert(cursor, text.to_owned());
                }
                stubs.contains_key(&cursor).then_some(())?;
                out.push(Slot::Stub(cursor));
            }
            ("pinned", _) if held.pinned > 0 => out.push(Slot::Pinned),
            ("summary", _) if held.summary.is_some() => out.push(Slot::Summary),
            ("memory", _) if held.memory > 0 => out.push(Slot::Memory),
            ("note", _) => {
                if let Some(text) = text {
                    text.clone_into(note);
                }
                out.push(Slot::Note);
            }
            _ => return None,
        }
    }
    // Groups go whole, and nothing sits inside one.
    let unit = |slot: &Slot| match slot {
        Slot::Item(c) | Slot::Stub(c) => by.get(c).map(|r| r.unit),
        _ => None,
    };
    let named: Vec<u64> = out.iter().filter_map(unit).collect();
    for row in shown {
        let sent = named.contains(&row.unit);
        let all = shown
            .iter()
            .filter(|r| r.unit == row.unit)
            .all(|r| out.contains(&Slot::Item(r.cursor)) || out.contains(&Slot::Stub(r.cursor)));
        if sent && !all {
            return None;
        }
    }
    for (index, slot) in out.iter().enumerate() {
        if unit(slot).is_some() {
            continue;
        }
        let before = out[..index].iter().rev().find_map(unit);
        let after = out[index + 1..].iter().find_map(unit);
        if before.is_some() && before == after {
            return None;
        }
    }
    Some(out)
}

/// Record what a confirmed layout sent: its stubs stay stubs and its summary stands in for the
/// span it covers. The provider's count corrects the model's factor.
fn confirm(
    at: &mut Answering<'_>,
    session: &SessionId,
    id: &str,
    usage: Option<&Value>,
) -> Result<(), String> {
    let scrollback = at.scrollback.as_ref().ok_or("applied needs a scrollback")?;
    let e = |e: StoreError| e.to_string();
    let Some(proposal) = scrollback.proposal(id).map_err(e)? else {
        return Err(format!("no layout called '{id}'"));
    };
    if &proposal.session != session {
        return Err(format!("layout '{id}' was not laid out for '{session}'"));
    }
    let real = usage
        .map(|u| {
            ["input", "cache_read", "cache_write"]
                .iter()
                .filter_map(|k| u.get(*k).and_then(Value::as_u64))
                .sum::<u64>()
        })
        .filter(|n| *n > 0);
    if !scrollback.confirm(id, real, at.now).map_err(e)? {
        return Ok(());
    }

    let slots = proposal.body["slots"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut summarised = Vec::new();
    let outline = scrollback.outline(session).map_err(e)?;
    for slot in &slots {
        match slot["kind"].as_str() {
            Some("stub") => {
                if let Some(cursor) = slot["cursor"].as_u64() {
                    scrollback.mark(session, cursor, State::Masked).map_err(e)?;
                }
            }
            Some("summary") => {
                let (Some(from), Some(to)) =
                    (slot["covers"][0].as_u64(), slot["covers"][1].as_u64())
                else {
                    continue;
                };
                for turn in &outline {
                    if (from..=to).contains(&turn.cursor) && turn.state != State::Summarised {
                        scrollback
                            .mark(session, turn.cursor, State::Summarised)
                            .map_err(e)?;
                        summarised.push(turn.cursor);
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(real) = real
        && proposal.estimated > 0
    {
        let ratio = real as f64 / proposal.estimated as f64;
        let next = match scrollback.measured(&proposal.model).map_err(e)? {
            Some((old, _)) => old * 0.7 + ratio * 0.3,
            None => ratio,
        };
        scrollback
            .set_factor(&proposal.model, next.clamp(0.5, 4.0))
            .map_err(e)?;
    }
    // TIDE. What a summary now stands in for becomes a candidate, never a fact.
    distil(at, session, &summarised).map_err(e)?;
    Ok(())
}

/// Raise the factor by what the provider said, count the overflow, and hand back what the
/// refused layout was asked with.
fn tighten(
    at: &mut Answering<'_>,
    session: &SessionId,
    id: &str,
    message: &str,
) -> Result<Value, String> {
    let scrollback = at
        .scrollback
        .as_ref()
        .ok_or("overflowed needs a scrollback")?;
    let e = |e: StoreError| e.to_string();
    let Some(proposal) = scrollback.proposal(id).map_err(e)? else {
        return Err(format!("no layout called '{id}'"));
    };
    if &proposal.session != session {
        return Err(format!("layout '{id}' was not laid out for '{session}'"));
    }
    let old = scrollback
        .measured(&proposal.model)
        .map_err(e)?
        .map_or(1.0, |m| m.0);
    let window = proposal.body["budget"]["window"].as_u64().unwrap_or(0);
    let reply = proposal.body["budget"]["reply"].as_u64().unwrap_or(0);
    let next = counted_in(message, &[window, reply])
        .map(|n| n as f64 / proposal.estimated.max(1) as f64)
        .filter(|f| *f > old)
        .unwrap_or(old * 1.25);
    scrollback
        .set_factor(&proposal.model, next.clamp(0.5, 8.0))
        .map_err(e)?;
    if let Some(mut prompt) = scrollback.prompt(session).map_err(e)? {
        prompt.overflows += 1;
        scrollback.keep_prompt(session, &prompt).map_err(e)?;
    }
    Ok(proposal.asked)
}

/// The provider's own count in a refusal: the largest number in it that is not the window or the
/// reply, such as `215034` in "prompt is too long: 215034 tokens > 200000 maximum".
fn counted_in(message: &str, known: &[u64]) -> Option<u64> {
    let plain: String = message.chars().filter(|c| *c != ',' && *c != '_').collect();
    plain
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse::<u64>().ok())
        .filter(|n| *n >= 1_000 && !known.contains(n))
        .max()
}

/// Attach a distillation witness to each turn a summary now stands in for, below the promotion
/// floor.
fn distil(
    at: &mut Answering<'_>,
    session: &SessionId,
    cursors: &[u64],
) -> Result<usize, StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(0);
    };
    let turns: Vec<Turn> = cursors
        .iter()
        .filter_map(|c| scrollback.at(session, *c).ok().flatten())
        .collect();
    let mut carried = 0;
    for entry in &turns {
        let scope = at.scope.clone();
        let Some(id) = at.run(session)?.scratch_for(&scope, session, &entry.text)? else {
            continue;
        };
        let witness = balthasar_model::Witness::new(
            balthasar_model::WitnessId::new(format!("tide-{}-{}", session, entry.cursor)),
            balthasar_model::WitnessKind::Distillation,
            session.clone(),
            at.scope.clone(),
            entry.at,
        )
        .at_cursor(entry.cursor)
        .noted("left the context window (rules, not a model)");
        let now = at.now;
        at.run(session)?.attach(&id, witness, now)?;
        carried += 1;
    }
    Ok(carried)
}

fn session_of(request: &Request) -> Option<SessionId> {
    request
        .args
        .first()
        .and_then(Value::as_str)
        .map(SessionId::new)
}

fn number(asked: &Value, name: &str) -> Option<u32> {
    asked
        .get(name)?
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
}

/// The fixed items: every count under `fixed`, or one number.
fn fixed(asked: &Value) -> u32 {
    let total: u64 = match asked.get("fixed") {
        Some(Value::Object(parts)) => parts.values().filter_map(Value::as_u64).sum(),
        Some(one) => one.as_u64().unwrap_or(0),
        None => 0,
    };
    u32::try_from(total).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_providers_count_is_found_in_its_refusal() {
        let said = "prompt is too long: 215034 tokens > 200000 maximum";
        assert_eq!(counted_in(said, &[200_000, 32_000]), Some(215_034));
        assert_eq!(
            counted_in("input of 1,203,400 tokens", &[]),
            Some(1_203_400)
        );
        assert_eq!(counted_in("context length exceeded", &[]), None);
        assert_eq!(counted_in("max 200000", &[200_000]), None);
    }
}
