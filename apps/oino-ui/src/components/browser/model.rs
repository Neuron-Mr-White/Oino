//! View model of the screencast Browser tab (browser-control.md §9.2): the session's tabs,
//! the shown tab, page / agent / dialog / handoff state, frame geometry. Pure (no GPUI), so it
//! is unit-tested; the view only renders it and forwards input.

use oino_extension_api::browser::protocol::{
    BrowserViewEvent, ControlledBy, TabInfo, TabOwner, redact_url,
};

/// Page state of one tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageState {
    /// URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Loading.
    pub loading: bool,
    /// Back possible.
    pub can_back: bool,
    /// Forward possible.
    pub can_forward: bool,
}

/// An open JS dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogCard {
    /// `alert|confirm|prompt|beforeunload`.
    pub kind: String,
    /// Message.
    pub message: String,
    /// Default prompt text.
    pub default_prompt: Option<String>,
}

/// What the view must do after an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Nothing beyond a repaint.
    Repaint,
    /// Attach a viewer to this tab (detaching the previous one).
    Show(String),
    /// Ask the pane to reveal the Browser tab (agent opened / handoff), with the tab to show.
    Reveal(String),
    /// The shown tab closed; nothing to show.
    Blank,
}

/// Take-over strip text (§9.2, unipi wording).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strip {
    /// No agent activity on the shown tab.
    None,
    /// "● Agent is using this tab · Take over".
    AgentDriving,
    /// "You have control · Give back".
    UserControl,
}

/// Screencast tab state.
#[derive(Clone, Debug, Default)]
pub struct BrowserModel {
    /// Session.
    pub session: String,
    /// Session tabs (agent + user).
    pub tabs: Vec<TabInfo>,
    /// Shown tab.
    pub current: Option<String>,
    /// Page of the shown tab.
    pub page: PageState,
    /// An agent command runs on the shown tab.
    pub driving: bool,
    /// The user has control of the shown tab.
    pub taken_over: bool,
    /// Open dialog of the shown tab.
    pub dialog: Option<DialogCard>,
    /// Pending handoff request of the shown tab.
    pub handoff: Option<String>,
    /// Last frame's page CSS size.
    pub frame_css: (f64, f64),
    /// Last frame sequence (acks).
    pub last_seq: u64,
    /// Download notice.
    pub download: Option<String>,
    /// The tab closed (reason) — empty state.
    pub closed: Option<String>,
}

impl BrowserModel {
    /// A model for `session`.
    #[must_use]
    pub fn new(session: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            ..Self::default()
        }
    }

    fn is_current(&self, tab: &str) -> bool {
        self.current.as_deref() == Some(tab)
    }

    /// Selects `tab` (user click on the strip).
    pub fn select(&mut self, tab: &str) -> Effect {
        if self.is_current(tab) {
            return Effect::Repaint;
        }
        self.current = Some(tab.to_owned());
        self.reset_tab_state();
        if let Some(t) = self.tabs.iter().find(|t| t.id == tab) {
            self.page.url.clone_from(&t.url);
            self.page.title.clone_from(&t.title);
            self.driving = t.driving;
            self.taken_over = t.controlled_by == ControlledBy::User;
        }
        Effect::Show(tab.to_owned())
    }

    fn reset_tab_state(&mut self) {
        self.page = PageState::default();
        self.dialog = None;
        self.handoff = None;
        self.driving = false;
        self.taken_over = false;
        self.closed = None;
        self.last_seq = 0;
    }

    /// Applies a viewer / session event.
    pub fn apply(&mut self, e: &BrowserViewEvent) -> Effect {
        match e {
            BrowserViewEvent::Frame {
                tab,
                seq,
                css_width,
                css_height,
                ..
            } if self.is_current(tab) => {
                self.frame_css = (*css_width, *css_height);
                self.last_seq = *seq;
                self.closed = None;
                Effect::Repaint
            }
            BrowserViewEvent::Page {
                tab,
                url,
                title,
                loading,
                can_back,
                can_forward,
            } => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == *tab) {
                    t.url.clone_from(url);
                    t.title.clone_from(title);
                }
                if self.is_current(tab) {
                    self.page = PageState {
                        url: url.clone(),
                        title: title.clone(),
                        loading: *loading,
                        can_back: *can_back,
                        can_forward: *can_forward,
                    };
                }
                Effect::Repaint
            }
            BrowserViewEvent::Tabs { tabs } => {
                self.tabs.clone_from(tabs);
                match &self.current {
                    Some(c) if !tabs.iter().any(|t| t.id == *c) => {
                        self.current = None;
                        if let Some(first) = tabs.first() {
                            let id = first.id.clone();
                            self.select(&id)
                        } else {
                            self.reset_tab_state();
                            Effect::Blank
                        }
                    }
                    None => match tabs.first() {
                        Some(first) => {
                            let id = first.id.clone();
                            self.select(&id)
                        }
                        None => Effect::Repaint,
                    },
                    Some(_) => Effect::Repaint,
                }
            }
            BrowserViewEvent::Agent {
                tab,
                driving,
                taken_over,
            } => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.id == *tab) {
                    t.driving = *driving;
                    t.controlled_by = if *taken_over {
                        ControlledBy::User
                    } else {
                        ControlledBy::Agent
                    };
                }
                if self.is_current(tab) {
                    self.driving = *driving;
                    self.taken_over = *taken_over;
                }
                Effect::Repaint
            }
            BrowserViewEvent::Reveal { tab, reason } => {
                if reason == "agent_opened" || reason == "handoff" || reason == "user" {
                    self.current = None;
                    let _ = self.select(tab);
                    Effect::Reveal(tab.clone())
                } else if self.current.is_none() {
                    self.select(tab)
                } else {
                    Effect::Repaint
                }
            }
            BrowserViewEvent::Dialog {
                tab,
                dialog,
                message,
                default_prompt,
            } if self.is_current(tab) => {
                self.dialog = Some(DialogCard {
                    kind: dialog.clone(),
                    message: message.clone(),
                    default_prompt: default_prompt.clone(),
                });
                Effect::Repaint
            }
            BrowserViewEvent::DialogClosed { tab } if self.is_current(tab) => {
                self.dialog = None;
                Effect::Repaint
            }
            BrowserViewEvent::Handoff {
                tab,
                message,
                pending,
            } => {
                if self.is_current(tab) {
                    self.handoff = pending.then(|| message.clone());
                    if *pending {
                        self.taken_over = true;
                    }
                }
                Effect::Repaint
            }
            BrowserViewEvent::Download { name, state, .. } => {
                self.download = Some(format!("{name} · {state}"));
                Effect::Repaint
            }
            BrowserViewEvent::Closed { tab, reason } => {
                self.tabs.retain(|t| t.id != *tab);
                if self.is_current(tab) {
                    self.current = None;
                    self.reset_tab_state();
                    self.closed = Some(reason.clone());
                    if let Some(first) = self.tabs.first() {
                        let id = first.id.clone();
                        return self.select(&id);
                    }
                    return Effect::Blank;
                }
                Effect::Repaint
            }
            _ => Effect::Repaint,
        }
    }

    /// Take-over strip state of the shown tab.
    #[must_use]
    pub fn strip(&self) -> Strip {
        if self.taken_over {
            Strip::UserControl
        } else if self.driving || self.current_is_agent() && self.recently_agent() {
            Strip::AgentDriving
        } else {
            Strip::None
        }
    }

    fn recently_agent(&self) -> bool {
        self.tabs
            .iter()
            .find(|t| self.is_current(&t.id))
            .is_some_and(|t| t.lifecycle.as_str() == "active" && t.driving)
    }

    /// The shown tab is an agent tab.
    #[must_use]
    pub fn current_is_agent(&self) -> bool {
        self.tabs
            .iter()
            .find(|t| self.is_current(&t.id))
            .is_some_and(|t| matches!(t.owner, TabOwner::Agent { .. }))
    }

    /// Tab strip label of `tab` (title, else host).
    #[must_use]
    pub fn label(tab: &TabInfo) -> String {
        if !tab.title.is_empty() {
            return tab.title.clone();
        }
        let r = redact_url(&tab.url);
        r.split("://")
            .nth(1)
            .and_then(|x| x.split('/').next())
            .filter(|h| !h.is_empty())
            .map_or_else(|| "New tab".to_owned(), str::to_owned)
    }

    /// Side tab title.
    #[must_use]
    pub fn title(&self) -> String {
        if !self.page.title.is_empty() {
            return self.page.title.clone();
        }
        self.tabs
            .iter()
            .find(|t| self.is_current(&t.id))
            .map_or_else(|| "Browser".to_owned(), Self::label)
    }
}

/// Letterbox geometry: where a `css` sized frame is drawn inside `area` (x, y, w, h), and the
/// page CSS point under a window point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// Draw origin x.
    pub x: f64,
    /// Draw origin y.
    pub y: f64,
    /// Draw scale (window px per CSS px).
    pub scale: f64,
}

impl Fit {
    /// Fits `css` into `area` (contain, centred).
    #[must_use]
    pub fn new(area: (f64, f64, f64, f64), css: (f64, f64)) -> Self {
        let (ax, ay, aw, ah) = area;
        let (cw, ch) = (css.0.max(1.0), css.1.max(1.0));
        let scale = (aw / cw).min(ah / ch).max(0.01);
        Self {
            x: ax + (aw - cw * scale) / 2.0,
            y: ay + (ah - ch * scale) / 2.0,
            scale,
        }
    }

    /// Window point → page CSS point (`None` outside the frame).
    #[must_use]
    pub fn to_page(&self, p: (f64, f64), css: (f64, f64)) -> Option<(f64, f64)> {
        let x = (p.0 - self.x) / self.scale;
        let y = (p.1 - self.y) / self.scale;
        ((0.0..=css.0).contains(&x) && (0.0..=css.1).contains(&y)).then_some((x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_extension_api::browser::protocol::TabLifecycle;

    fn tab(id: &str, agent: bool) -> TabInfo {
        TabInfo {
            id: id.into(),
            url: format!("https://ex.com/{id}"),
            title: String::new(),
            owner: if agent {
                TabOwner::Agent {
                    session: "s".into(),
                }
            } else {
                TabOwner::User {
                    session: Some("s".into()),
                }
            },
            lifecycle: if agent {
                TabLifecycle::Active
            } else {
                TabLifecycle::User
            },
            controlled_by: ControlledBy::Agent,
            loading: false,
            can_back: false,
            can_forward: false,
            profile: "default".into(),
            driving: false,
        }
    }

    #[test]
    fn tabs_reveal_takeover_and_close() {
        let mut m = BrowserModel::new("s");
        assert_eq!(
            m.apply(&BrowserViewEvent::Tabs {
                tabs: vec![tab("t1", true)]
            }),
            Effect::Show("t1".into())
        );
        assert_eq!(m.title(), "ex.com");
        assert_eq!(
            m.apply(&BrowserViewEvent::Tabs {
                tabs: vec![tab("t1", true), tab("t2", true)]
            }),
            Effect::Repaint
        );
        assert_eq!(
            m.apply(&BrowserViewEvent::Reveal {
                tab: "t2".into(),
                reason: "agent_opened".into()
            }),
            Effect::Reveal("t2".into())
        );
        assert_eq!(m.current.as_deref(), Some("t2"));
        m.apply(&BrowserViewEvent::Agent {
            tab: "t2".into(),
            driving: true,
            taken_over: false,
        });
        assert_eq!(m.strip(), Strip::AgentDriving);
        m.apply(&BrowserViewEvent::Agent {
            tab: "t2".into(),
            driving: false,
            taken_over: true,
        });
        assert_eq!(m.strip(), Strip::UserControl);
        m.apply(&BrowserViewEvent::Handoff {
            tab: "t2".into(),
            message: "Solve the CAPTCHA".into(),
            pending: true,
        });
        assert_eq!(m.handoff.as_deref(), Some("Solve the CAPTCHA"));
        m.apply(&BrowserViewEvent::Page {
            tab: "t2".into(),
            url: "https://ex.com/a".into(),
            title: "A page".into(),
            loading: false,
            can_back: true,
            can_forward: false,
        });
        assert_eq!(m.title(), "A page");
        assert!(m.page.can_back);
        assert_eq!(
            m.apply(&BrowserViewEvent::Closed {
                tab: "t2".into(),
                reason: "agent".into()
            }),
            Effect::Show("t1".into())
        );
        assert_eq!(
            m.apply(&BrowserViewEvent::Closed {
                tab: "t1".into(),
                reason: "user".into()
            }),
            Effect::Blank
        );
        assert_eq!(m.closed.as_deref(), Some("user"));
    }

    #[test]
    fn dialog_only_for_shown_tab() {
        let mut m = BrowserModel::new("s");
        m.apply(&BrowserViewEvent::Tabs {
            tabs: vec![tab("t1", false), tab("t2", false)],
        });
        m.apply(&BrowserViewEvent::Dialog {
            tab: "t2".into(),
            dialog: "alert".into(),
            message: "hi".into(),
            default_prompt: None,
        });
        assert!(m.dialog.is_none());
        m.apply(&BrowserViewEvent::Dialog {
            tab: "t1".into(),
            dialog: "alert".into(),
            message: "hi".into(),
            default_prompt: None,
        });
        assert_eq!(m.dialog.as_ref().map(|d| d.kind.as_str()), Some("alert"));
        m.apply(&BrowserViewEvent::DialogClosed { tab: "t1".into() });
        assert!(m.dialog.is_none());
        assert!(!m.current_is_agent());
    }

    #[test]
    fn letterbox_mapping() {
        let f = Fit::new((100.0, 50.0, 640.0, 800.0), (1280.0, 800.0));
        assert!((f.scale - 0.5).abs() < 1e-9);
        assert!((f.y - 250.0).abs() < 1e-9);
        assert_eq!(
            f.to_page((420.0, 450.0), (1280.0, 800.0)),
            Some((640.0, 400.0))
        );
        assert_eq!(f.to_page((420.0, 100.0), (1280.0, 800.0)), None);
    }
}
