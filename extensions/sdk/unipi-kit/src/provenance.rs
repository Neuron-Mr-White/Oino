//! Harness provenance (unipi `harness-messages.ts` → host provenance, audit-unipi §1.3.10).
//!
//! unipi tagged harness text with `unipiHarness {version, id, source, title, synopsis,
//! delivery, severity}` through an arm/match workaround. Oino carries provenance natively on
//! `send-message` / `send-user-message`; [`Harness`] builds the record and maps unipi's
//! `delivery` onto the host's delivery and trigger options.

use oino_extension::session::{CustomMessage, Delivery, Provenance, SendOptions};
use serde_json::Value;

/// unipi `HarnessDelivery`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessDelivery {
    /// Sent now (idle: starts a turn).
    Direct,
    /// Steer the running turn.
    Steer,
    /// After the running turn.
    FollowUp,
    /// With the next user turn.
    NextTurn,
    /// At a boundary (compaction, settle): next turn, no trigger.
    Boundary,
    /// Injected at `before_agent_start` (returned from the hook, not sent).
    BeforeAgentStart,
}

impl HarnessDelivery {
    /// Host delivery while streaming.
    #[must_use]
    pub fn deliver_as(self) -> Option<Delivery> {
        match self {
            Self::Direct | Self::BeforeAgentStart => None,
            Self::Steer => Some(Delivery::Steer),
            Self::FollowUp => Some(Delivery::FollowUp),
            Self::NextTurn | Self::Boundary => Some(Delivery::NextTurn),
        }
    }

    /// Start a turn when idle.
    #[must_use]
    pub fn triggers_turn(self) -> bool {
        matches!(self, Self::Direct | Self::Steer | Self::FollowUp)
    }

    /// unipi wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Steer => "steer",
            Self::FollowUp => "followUp",
            Self::NextTurn => "nextTurn",
            Self::Boundary => "boundary",
            Self::BeforeAgentStart => "before_agent_start",
        }
    }

    /// Parses a unipi wire name.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "direct" => Self::Direct,
            "steer" => Self::Steer,
            "followUp" => Self::FollowUp,
            "nextTurn" => Self::NextTurn,
            "boundary" => Self::Boundary,
            "before_agent_start" => Self::BeforeAgentStart,
            _ => return None,
        })
    }
}

/// unipi severity (`warning` is the only one unipi uses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Warning styling.
    Warning,
}

/// A harness message description.
#[derive(Clone, Debug, PartialEq)]
pub struct Harness {
    /// Source label (package display name, e.g. `long-horizon`).
    pub source: String,
    /// Title.
    pub title: String,
    /// One-line synopsis.
    pub synopsis: Option<String>,
    /// Severity.
    pub severity: Option<Severity>,
    /// Delivery.
    pub delivery: HarnessDelivery,
}

impl Harness {
    /// Direct-delivery harness message from `source` titled `title`.
    #[must_use]
    pub fn new(source: &str, title: &str) -> Self {
        Self {
            source: source.to_owned(),
            title: title.to_owned(),
            synopsis: None,
            severity: None,
            delivery: HarnessDelivery::Direct,
        }
    }

    /// Synopsis.
    #[must_use]
    pub fn synopsis(mut self, s: impl Into<String>) -> Self {
        self.synopsis = Some(s.into());
        self
    }

    /// Warning severity.
    #[must_use]
    pub fn warning(mut self) -> Self {
        self.severity = Some(Severity::Warning);
        self
    }

    /// Delivery.
    #[must_use]
    pub fn delivery(mut self, d: HarnessDelivery) -> Self {
        self.delivery = d;
        self
    }

    /// Host provenance record.
    #[must_use]
    pub fn provenance(&self) -> Provenance {
        Provenance {
            source: self.source.clone(),
            title: Some(self.title.clone()),
            synopsis: self.synopsis.clone(),
            severity: self.severity.map(|Severity::Warning| "warning".to_owned()),
        }
    }

    /// Options for `session::send_message`.
    #[must_use]
    pub fn send_options(&self) -> SendOptions {
        SendOptions {
            trigger_turn: self.delivery.triggers_turn(),
            deliver_as: self.delivery.deliver_as(),
            provenance: Some(self.provenance()),
        }
    }

    /// Sends a custom message with this provenance.
    pub async fn send_custom(
        &self,
        custom_type: &str,
        content: Value,
        details: Option<Value>,
    ) -> oino_extension::Result<()> {
        let msg = CustomMessage {
            custom_type: custom_type.to_owned(),
            content,
            display: true,
            details,
        };
        oino_extension::session::send_message(&msg, self.send_options()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_mapping() {
        for name in [
            "direct",
            "steer",
            "followUp",
            "nextTurn",
            "boundary",
            "before_agent_start",
        ] {
            assert_eq!(
                HarnessDelivery::parse(name).map(HarnessDelivery::as_str),
                Some(name)
            );
        }
        assert_eq!(HarnessDelivery::parse("later"), None);
        assert!(HarnessDelivery::Direct.deliver_as().is_none());
        assert!(matches!(
            HarnessDelivery::Boundary.deliver_as(),
            Some(Delivery::NextTurn)
        ));
        assert!(!HarnessDelivery::NextTurn.triggers_turn());
        assert!(HarnessDelivery::FollowUp.triggers_turn());
    }

    #[test]
    fn provenance_record() {
        let h = Harness::new("long-horizon", "Goal check")
            .synopsis("2 of 5 done")
            .warning()
            .delivery(HarnessDelivery::FollowUp);
        let p = h.provenance();
        assert_eq!(p.source, "long-horizon");
        assert_eq!(p.title.as_deref(), Some("Goal check"));
        assert_eq!(p.severity.as_deref(), Some("warning"));
        let o = h.send_options();
        assert!(o.trigger_turn);
        assert!(matches!(o.deliver_as, Some(Delivery::FollowUp)));
    }
}
