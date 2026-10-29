use super::super::{Encoded, Id, Loss, MAX_EVENT_BYTES, Revision};
use super::{Sent, TransportLoss, transport};
use crate::events::Program as Owner;
use serde::{Deserialize, Serialize};
use std::{
    num::NonZeroU64,
    time::{Duration, Instant},
};

pub const REPORT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerStatus {
    pub revision: NonZeroU64,
    pub events_sent: u64,
    pub queue_loss: Loss,
    pub transport_loss: TransportLoss,
    pub finished: bool,
}

#[cfg(test)]
mod tests;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    schema: Revision,
    program: Owner,
    producer: Id,
    status: ProducerStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Sent(Sent),
    RateLimited,
    Exhausted,
    Closed,
    Encoding,
}

pub struct Sender {
    inner: transport::Sender,
    lineage: crate::events::Lineage,
    parent: Option<rustix::process::Pid>,
    owner: Owner,
    producer: Id,
    events_sent: u64,
    next_revision: u64,
    last_report: Option<Instant>,
    finished: bool,
}

impl Sender {
    pub(super) fn parent(&self) -> Option<rustix::process::Pid> {
        self.parent
    }

    pub(super) fn with_parent(mut self, parent: rustix::process::Pid) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn lineage(&self) -> crate::events::Lineage {
        self.lineage
    }

    pub(super) fn with_lineage(mut self, lineage: crate::events::Lineage) -> Self {
        self.lineage = lineage;
        self
    }

    pub(super) fn into_fresh(self) -> Option<(transport::Sender, Owner, Id)> {
        (self.events_sent == 0
            && self.next_revision == 1
            && self.last_report.is_none()
            && !self.finished
            && self.inner.loss() == TransportLoss::default())
        .then_some((self.inner, self.owner, self.producer))
    }

    pub fn identity(&self) -> Id {
        self.producer
    }

    pub(super) fn new(inner: transport::Sender, owner: Owner, producer: Id) -> Self {
        Self {
            inner,
            lineage: crate::events::Lineage::default(),
            parent: None,
            owner,
            producer,
            events_sent: 0,
            next_revision: 1,
            last_report: None,
            finished: false,
        }
    }

    pub fn send(&mut self, bytes: &[u8]) -> Sent {
        if self.finished || self.events_sent == u64::MAX {
            return self.inner.send(&[]);
        }
        let result = self.inner.send(bytes);
        if result == Sent::Queued {
            self.events_sent += 1;
        }
        result
    }

    pub fn loss(&self) -> TransportLoss {
        self.inner.loss()
    }

    pub fn report(&mut self, queue_loss: Loss, finished: bool) -> Report {
        self.report_at(queue_loss, finished, Instant::now())
    }

    fn report_at(&mut self, queue_loss: Loss, finished: bool, now: Instant) -> Report {
        if self.finished {
            return Report::Closed;
        }
        if self
            .last_report
            .is_some_and(|last| now.saturating_duration_since(last) < REPORT_INTERVAL)
        {
            return Report::RateLimited;
        }
        let Some(next_revision) = self.next_revision.checked_add(1) else {
            return Report::Exhausted;
        };
        let Some(revision) = NonZeroU64::new(self.next_revision) else {
            return Report::Exhausted;
        };
        self.last_report = Some(now);
        self.next_revision = next_revision;
        let frame = Frame {
            schema: Revision,
            program: self.owner,
            producer: self.producer,
            status: ProducerStatus {
                revision,
                events_sent: self.events_sent,
                queue_loss,
                transport_loss: self.inner.loss(),
                finished,
            },
        };
        let mut encoded = Encoded {
            bytes: [0; MAX_EVENT_BYTES],
            len: 0,
        };
        if serde_json::to_writer(&mut encoded, &frame).is_err() {
            return Report::Encoding;
        }
        let Some(bytes) = encoded.bytes.get(..encoded.len) else {
            return Report::Encoding;
        };
        let result = self.inner.send(bytes);
        if result == Sent::Queued && finished {
            self.finished = true;
        }
        Report::Sent(result)
    }
}
