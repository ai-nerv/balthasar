//! Atomic transcript application and replay of cross-store layout effects.

use crate::Answering;
use balthasar_model::{SessionId, Witness, WitnessId, WitnessKind};
use balthasar_store::{LayoutEffect, LayoutTarget, State, StoreError};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PLAUSIBLE: std::ops::RangeInclusive<f64> = 0.25..=3.0;

pub(crate) fn confirm(
    at: &mut Answering<'_>,
    session: &SessionId,
    id: &str,
    usage: Option<&Value>,
) -> Result<(), StoreError> {
    let scrollback = at
        .scrollback
        .as_ref()
        .ok_or_else(|| StoreError::Foreign("applied needs a scrollback".into()))?;
    scrollback.atomic(|scrollback| {
        let proposal = scrollback
            .proposal(id)?
            .ok_or_else(|| StoreError::Unknown(format!("no layout called '{id}'")))?;
        if &proposal.session != session {
            return Err(StoreError::Foreign(format!(
                "layout '{id}' was not laid out for '{session}'"
            )));
        }
        let real = usage
            .map(|u| {
                ["input", "cache_read", "cache_write"]
                    .iter()
                    .filter_map(|k| u.get(*k).and_then(Value::as_u64))
                    .fold(0_u64, u64::saturating_add)
            })
            .filter(|n| *n > 0);
        if !scrollback.confirm(id, real, at.now)? {
            return Ok(());
        }
        let run = scrollback.run_of(session)?;
        let outline = scrollback.outline(session)?;
        for slot in proposal.body["slots"].as_array().into_iter().flatten() {
            match slot["kind"].as_str() {
                Some("stub") => {
                    if let Some(cursor) = slot["cursor"].as_u64() {
                        scrollback.mark(session, cursor, State::Masked)?;
                    }
                }
                Some("summary") => {
                    let (Some(from), Some(to)) =
                        (slot["covers"][0].as_u64(), slot["covers"][1].as_u64())
                    else {
                        continue;
                    };
                    for row in &outline {
                        if !(from..=to).contains(&row.cursor) || row.state == State::Summarised {
                            continue;
                        }
                        let turn = scrollback.at(session, row.cursor)?.ok_or_else(|| {
                            StoreError::Unknown("summary source is missing".into())
                        })?;
                        scrollback.mark(session, row.cursor, State::Summarised)?;
                        scrollback.queue_layout_effect(&LayoutEffect {
                            layout: id.into(),
                            session: session.clone(),
                            run: run.clone(),
                            agent: at.agent.clone(),
                            scope: at.scope.clone(),
                            cursor: turn.cursor,
                            text: turn.text,
                            at: turn.at,
                        })?;
                    }
                }
                _ => {}
            }
        }
        if let Some(real) = real
            && proposal.estimated > 0
        {
            let ratio = real as f64 / proposal.estimated as f64;
            if PLAUSIBLE.contains(&ratio) {
                let next = match scrollback.measured(&proposal.model)? {
                    Some((old, _)) => old * 0.7 + ratio * 0.3,
                    None => ratio,
                };
                scrollback.set_factor(&proposal.model, next.clamp(0.5, 4.0))?;
            }
        }
        Ok(())
    })?;
    replay(at, session)
}

pub(crate) fn replay(at: &mut Answering<'_>, session: &SessionId) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    let effects = scrollback.layout_effects(session, &at.agent, &at.scope)?;
    for effect in effects {
        if at.run_id(session)? != effect.run {
            return Err(StoreError::Foreign(
                "layout effect has a conflicting scratch run".into(),
            ));
        }
        let key = json!([
            effect.scope.as_str(),
            effect.run.as_str(),
            effect.agent.as_str(),
            effect.session.as_str(),
            effect.layout,
            effect.cursor
        ]);
        let witness = Witness::new(
            WitnessId::new(format!(
                "tide-{:x}",
                Sha256::digest(key.to_string().as_bytes())
            )),
            WitnessKind::Distillation,
            effect.run.clone(),
            effect.scope.clone(),
            effect.at,
        )
        .at_cursor(effect.cursor)
        .noted("left the context window (rules, not a model)");
        let scrollback = at
            .scrollback
            .as_ref()
            .ok_or_else(|| StoreError::Foreign("layout recovery needs a scrollback".into()))?;
        let mut target = scrollback.layout_target(&effect)?;
        if target == LayoutTarget::Unresolved {
            let store = at.run(session)?;
            let candidate = match store.witnessed_memory(&witness.id)? {
                Some(id) => Some(id),
                None => store.scratch_for(&effect.scope, &effect.run, &effect.text)?,
            };
            target = at
                .scrollback
                .as_ref()
                .ok_or_else(|| StoreError::Foreign("layout recovery needs a scrollback".into()))?
                .bind_layout_target(&effect, candidate.as_ref())?;
        }
        if let LayoutTarget::Bound(id) = target {
            let now = at.now;
            let store = at.run(session)?;
            if let Some(memory) = store.get(&id)? {
                if memory.scope != effect.scope
                    || memory.session.as_ref() != Some(&effect.run)
                    || memory.text() != effect.text
                {
                    return Err(StoreError::Foreign(
                        "layout effect target no longer matches its source".into(),
                    ));
                }
                store.attach(&id, witness, now)?;
            }
        }
        if let Some(scrollback) = at.scrollback.as_ref() {
            scrollback.finish_layout_effect(&effect)?;
        }
    }
    Ok(())
}
