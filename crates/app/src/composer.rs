// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Composer window.
//!
//! Collects a new message: the sender identity, recipients, subject and a
//! plain-text body. Identities come from the accounts discovered through
//! GOA/EDS, so the picker follows whatever the desktop knows about.

use std::sync::Arc;

use mail_core::compose::{self, Address, OutgoingMessage};
use mail_core::store::{AccountRecord, Store};
use relm4::adw;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use tracing::debug;

const SEND_UNAVAILABLE: &str = "Sending isn't available yet";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub account_id: i64,
    pub name: String,
    pub email: String,
}

impl Identity {
    pub fn label(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() || name.eq_ignore_ascii_case(&self.email) {
            self.email.clone()
        } else {
            format!("{name} <{}>", self.email)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDraft {
    pub identity: Option<Identity>,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerOutput {
    Send(MessageDraft),
}

pub fn identities(accounts: &[AccountRecord]) -> Vec<Identity> {
    accounts
        .iter()
        .filter(|account| !account.email.trim().is_empty())
        .map(|account| Identity {
            account_id: account.id.unwrap_or_default(),
            name: account.display_name.clone().unwrap_or_default(),
            email: account.email.trim().to_string(),
        })
        .collect()
}

pub fn can_send(identity: Option<&Identity>, to: &str) -> bool {
    identity.is_some() && !to.trim().is_empty()
}

pub fn window_title(subject: &str) -> String {
    let subject = subject.trim();
    if subject.is_empty() {
        "New Message".to_string()
    } else {
        subject.to_string()
    }
}

pub fn draft(
    identity: Option<Identity>,
    to: &str,
    cc: &str,
    bcc: &str,
    subject: &str,
    body: &str,
) -> MessageDraft {
    MessageDraft {
        identity,
        to: to.trim().to_string(),
        cc: cc.trim().to_string(),
        bcc: bcc.trim().to_string(),
        subject: subject.trim().to_string(),
        body: body.to_string(),
    }
}

pub fn outgoing(draft: &MessageDraft) -> OutgoingMessage {
    OutgoingMessage {
        from: draft.identity.as_ref().map(sender_address),
        to: compose::parse_addresses(&draft.to),
        cc: compose::parse_addresses(&draft.cc),
        bcc: compose::parse_addresses(&draft.bcc),
        subject: draft.subject.clone(),
        text: Some(draft.body.clone()),
        ..OutgoingMessage::default()
    }
}

fn sender_address(identity: &Identity) -> Address {
    let name = identity.name.trim();
    let name = if name.is_empty() || name.eq_ignore_ascii_case(&identity.email) {
        None
    } else {
        Some(name.to_string())
    };
    Address::new(name, identity.email.clone())
}

pub struct Composer {
    store: Arc<Store>,
    identity_labels: gtk::StringList,
    identities: Vec<Identity>,
    selected_identity: usize,
    to: String,
    cc: String,
    bcc: String,
    subject: String,
    cc_visible: bool,
    body_view: Option<gtk::TextView>,
    toast_overlay: Option<adw::ToastOverlay>,
}

#[derive(Debug)]
pub enum ComposerMsg {
    IdentitiesLoaded(Vec<Identity>),
    IdentitySelected(u32),
    ToChanged(String),
    CcChanged(String),
    BccChanged(String),
    SubjectChanged(String),
    ToggleCc,
    Send,
}

#[relm4::component(pub)]
impl SimpleComponent for Composer {
    type Init = Arc<Store>;
    type Input = ComposerMsg;
    type Output = ComposerOutput;

    view! {
        #[root]
        adw::ApplicationWindow {
            set_default_size: (760, 640),
            set_width_request: 400,
            set_height_request: 320,
            #[watch]
            set_title: Some(window_title(&model.subject).as_str()),
            set_visible: true,

            #[name(toast_overlay)]
            adw::ToastOverlay {
                adw::ToolbarView {
                    add_top_bar = &adw::HeaderBar {
                        #[name(send_button)]
                        pack_end = &gtk::Button {
                            set_label: "Send",
                            add_css_class: "suggested-action",
                            set_tooltip_text: Some("Send this message"),
                            #[watch]
                            set_sensitive: can_send(model.identity().as_ref(), &model.to),
                            connect_clicked[sender] => move |_| sender.input(ComposerMsg::Send),
                        },
                    },

                    #[wrap(Some)]
                    set_content = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 6,
                            set_margin_all: 12,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,

                                gtk::Label {
                                    set_label: "From",
                                    set_width_chars: 8,
                                    set_xalign: 0.0,
                                    add_css_class: "dim-label",
                                },

                                #[name(identity_dropdown)]
                                gtk::DropDown {
                                    set_hexpand: true,
                                    set_model: Some(&model.identity_labels),
                                    connect_selected_notify[sender] => move |dropdown| {
                                        sender.input(ComposerMsg::IdentitySelected(dropdown.selected()))
                                    },
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,

                                gtk::Label {
                                    set_label: "To",
                                    set_width_chars: 8,
                                    set_xalign: 0.0,
                                    add_css_class: "dim-label",
                                },

                                gtk::Entry {
                                    set_hexpand: true,
                                    set_placeholder_text: Some("name@example.org"),
                                    connect_changed[sender] => move |entry| {
                                        sender.input(ComposerMsg::ToChanged(entry.text().to_string()))
                                    },
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                #[watch]
                                set_visible: model.cc_visible,

                                gtk::Label {
                                    set_label: "Cc",
                                    set_width_chars: 8,
                                    set_xalign: 0.0,
                                    add_css_class: "dim-label",
                                },

                                gtk::Entry {
                                    set_hexpand: true,
                                    connect_changed[sender] => move |entry| {
                                        sender.input(ComposerMsg::CcChanged(entry.text().to_string()))
                                    },
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                                #[watch]
                                set_visible: model.cc_visible,

                                gtk::Label {
                                    set_label: "Bcc",
                                    set_width_chars: 8,
                                    set_xalign: 0.0,
                                    add_css_class: "dim-label",
                                },

                                gtk::Entry {
                                    set_hexpand: true,
                                    connect_changed[sender] => move |entry| {
                                        sender.input(ComposerMsg::BccChanged(entry.text().to_string()))
                                    },
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,

                                gtk::Label {
                                    set_label: "Subject",
                                    set_width_chars: 8,
                                    set_xalign: 0.0,
                                    add_css_class: "dim-label",
                                },

                                gtk::Entry {
                                    set_hexpand: true,
                                    connect_changed[sender] => move |entry| {
                                        sender.input(ComposerMsg::SubjectChanged(entry.text().to_string()))
                                    },
                                },
                            },

                            gtk::Button {
                                set_label: "Cc/Bcc",
                                set_halign: gtk::Align::Start,
                                add_css_class: "flat",
                                add_css_class: "composer-cc-toggle",
                                connect_clicked[sender] => move |_| sender.input(ComposerMsg::ToggleCc),
                            },
                        },

                        gtk::Separator {},

                        gtk::ScrolledWindow {
                            set_vexpand: true,
                            set_hexpand: true,
                            set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),

                            #[name(body_view)]
                            gtk::TextView {
                                set_wrap_mode: gtk::WrapMode::WordChar,
                                set_top_margin: 12,
                                set_bottom_margin: 12,
                                set_left_margin: 12,
                                set_right_margin: 12,
                            },
                        },
                    },
                },
            },
        }
    }

    fn init(
        store: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut model = Composer {
            store,
            identity_labels: gtk::StringList::new(&[]),
            identities: Vec::new(),
            selected_identity: 0,
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            subject: String::new(),
            cc_visible: false,
            body_view: None,
            toast_overlay: None,
        };
        let widgets = view_output!();
        model.body_view = Some(widgets.body_view.clone());
        model.toast_overlay = Some(widgets.toast_overlay.clone());

        let store = model.store.clone();
        let load_sender = sender.clone();
        sender.oneshot_command(async move {
            let identities = match store.accounts().await {
                Ok(accounts) => identities(&accounts),
                Err(err) => {
                    debug!(detail = %err, "failed to load composer identities");
                    Vec::new()
                }
            };
            load_sender.input(ComposerMsg::IdentitiesLoaded(identities));
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            ComposerMsg::IdentitiesLoaded(identities) => {
                let labels: Vec<String> = identities.iter().map(Identity::label).collect();
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                self.identity_labels
                    .splice(0, self.identity_labels.n_items(), &labels);
                self.identities = identities;
                self.selected_identity = 0;
            }
            ComposerMsg::IdentitySelected(index) => self.selected_identity = index as usize,
            ComposerMsg::ToChanged(value) => self.to = value,
            ComposerMsg::CcChanged(value) => self.cc = value,
            ComposerMsg::BccChanged(value) => self.bcc = value,
            ComposerMsg::SubjectChanged(value) => self.subject = value,
            ComposerMsg::ToggleCc => self.cc_visible = !self.cc_visible,
            ComposerMsg::Send => {
                let identity = self.identity();
                if !can_send(identity.as_ref(), &self.to) {
                    return;
                }
                let draft = draft(
                    identity,
                    &self.to,
                    &self.cc,
                    &self.bcc,
                    &self.subject,
                    &self.body(),
                );
                if let Some(overlay) = &self.toast_overlay {
                    overlay.add_toast(adw::Toast::new(SEND_UNAVAILABLE));
                }
                let _ = sender.output(ComposerOutput::Send(draft));
            }
        }
    }
}

impl Composer {
    fn identity(&self) -> Option<Identity> {
        self.identities.get(self.selected_identity).cloned()
    }

    fn body(&self) -> String {
        let Some(view) = &self.body_view else {
            return String::new();
        };
        let buffer = view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::store::{AccountSource, AuthKind};

    fn account(id: i64, name: Option<&str>, email: &str) -> AccountRecord {
        AccountRecord {
            id: Some(id),
            source: AccountSource::Eds,
            external_id: format!("account_{id}"),
            email: email.to_string(),
            display_name: name.map(str::to_string),
            imap_host: None,
            imap_port: None,
            imap_security: None,
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            auth_kind: AuthKind::Password,
            username: None,
        }
    }

    #[test]
    fn identities_follow_the_discovered_accounts() {
        let identities = identities(&[
            account(1, Some("Ada Lovelace"), "ada@lovelace.dev"),
            account(2, None, "grace@navy.dev"),
            account(3, Some("Nobody"), "   "),
        ]);
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].account_id, 1);
        assert_eq!(identities[0].label(), "Ada Lovelace <ada@lovelace.dev>");
        assert_eq!(identities[1].label(), "grace@navy.dev");
    }

    #[test]
    fn identity_label_skips_redundant_names() {
        let identity = Identity {
            account_id: 1,
            name: "ADA@lovelace.dev".to_string(),
            email: "ada@lovelace.dev".to_string(),
        };
        assert_eq!(identity.label(), "ada@lovelace.dev");
    }

    #[test]
    fn drafts_are_trimmed() {
        let identity = identities(&[account(1, Some("Ada"), "ada@lovelace.dev")])
            .into_iter()
            .next();
        let draft = draft(
            identity.clone(),
            "  me@example.org , two@example.org ",
            " cc@example.org ",
            "",
            "  Hello  ",
            "Body\n",
        );
        assert_eq!(draft.identity, identity);
        assert_eq!(draft.to, "me@example.org , two@example.org");
        assert_eq!(draft.cc, "cc@example.org");
        assert_eq!(draft.bcc, "");
        assert_eq!(draft.subject, "Hello");
        assert_eq!(draft.body, "Body\n");
    }

    #[test]
    fn sending_needs_an_identity_and_a_recipient() {
        let identity = identities(&[account(1, Some("Ada"), "ada@lovelace.dev")])
            .into_iter()
            .next();
        assert!(!can_send(None, "me@example.org"));
        assert!(!can_send(identity.as_ref(), "   "));
        assert!(!can_send(identity.as_ref(), ""));
        assert!(can_send(identity.as_ref(), " me@example.org "));
    }

    #[test]
    fn window_title_follows_the_subject() {
        assert_eq!(window_title("   "), "New Message");
        assert_eq!(window_title(" Lunch? "), "Lunch?");
    }

    #[test]
    fn outgoing_messages_carry_identity_and_recipients() {
        let identity = identities(&[account(7, Some("Ada Lovelace"), "ada@lovelace.dev")])
            .into_iter()
            .next();
        let draft = draft(
            identity,
            "grace@navy.dev, \"Hopper, G\" <g@navy.dev>",
            "cc@example.org",
            "",
            "Greetings",
            "Hello there",
        );
        let outgoing = outgoing(&draft);

        let from = outgoing.from.as_ref().unwrap();
        assert_eq!(from.name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(from.email, "ada@lovelace.dev");
        assert_eq!(outgoing.to.len(), 2);
        assert_eq!(outgoing.to[1].name.as_deref(), Some("Hopper, G"));
        assert_eq!(outgoing.cc.len(), 1);
        assert_eq!(outgoing.cc[0].email, "cc@example.org");
        assert!(outgoing.bcc.is_empty());
        assert_eq!(outgoing.subject, "Greetings");
        assert_eq!(outgoing.text.as_deref(), Some("Hello there"));
        assert!(outgoing.html.is_none());
        assert!(outgoing.attachments.is_empty());
    }

    #[test]
    fn outgoing_skips_redundant_sender_names() {
        let identity = identities(&[account(7, None, "ada@lovelace.dev")])
            .into_iter()
            .next();
        let outgoing = outgoing(&draft(identity, "me@example.org", "", "", "", ""));
        assert_eq!(outgoing.from.unwrap().name, None);
    }
}
