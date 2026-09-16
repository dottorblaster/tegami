// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Composer window.
//!
//! Collects a new message: the sender identity, recipients, subject and a
//! plain-text body. Identities come from the accounts discovered through
//! GOA/EDS, so the picker follows whatever the desktop knows about.

use std::sync::Arc;

use mail_core::compose::{self, Address, OutgoingAttachment, OutgoingMessage};
use mail_core::store::{AccountRecord, Store};
use relm4::adw;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use tracing::debug;

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

/// Identifies a draft already stored in a folder, so saving it again can
/// replace the previous version and sending it can remove it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftRef {
    pub account_id: i64,
    pub folder: String,
    pub uid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDraft {
    pub identity: Option<Identity>,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<OutgoingAttachment>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub draft: Option<DraftRef>,
}

/// Everything the composer needs to start: the store for identities and an
/// optional prefilled draft (reply, reply-all or forward).
pub struct ComposerInit {
    pub store: Arc<Store>,
    pub initial: Option<MessageDraft>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerOutput {
    Send(MessageDraft),
    SaveDraft(MessageDraft),
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
        attachments: Vec::new(),
        in_reply_to: None,
        references: Vec::new(),
        draft: None,
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
        attachments: draft.attachments.clone(),
        in_reply_to: draft.in_reply_to.clone(),
        references: draft.references.clone(),
        ..OutgoingMessage::default()
    }
}

pub(crate) fn render_addresses(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(render_address)
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_address(address: &Address) -> String {
    match &address.name {
        Some(name) if !name.trim().is_empty() => {
            let name = name.trim();
            if name.contains(',') || name.contains(';') || name.contains('"') {
                format!("\"{name}\" <{}>", address.email)
            } else {
                format!("{name} <{}>", address.email)
            }
        }
        _ => address.email.clone(),
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
    attachments: Vec<OutgoingAttachment>,
    in_reply_to: Option<String>,
    references: Vec<String>,
    draft: Option<DraftRef>,
    pending: Option<MessageDraft>,
    to_entry: Option<gtk::Entry>,
    cc_entry: Option<gtk::Entry>,
    bcc_entry: Option<gtk::Entry>,
    subject_entry: Option<gtk::Entry>,
    identity_dropdown: Option<gtk::DropDown>,
    body_view: Option<gtk::TextView>,
    attachments_slot: Option<gtk::Box>,
    save_draft_button: Option<gtk::Button>,
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
    SaveDraft,
    DraftSaved { folder: String, uid: u32 },
}

#[relm4::component(pub)]
impl SimpleComponent for Composer {
    type Init = ComposerInit;
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
                        #[name(save_draft_button)]
                        pack_start = &gtk::Button {
                            set_icon_name: "document-save-symbolic",
                            set_tooltip_text: Some("Save draft"),
                            set_sensitive: false,
                            connect_clicked[sender] => move |_| sender.input(ComposerMsg::SaveDraft),
                        },

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

                                #[name(to_entry)]
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

                                #[name(cc_entry)]
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

                                #[name(bcc_entry)]
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

                                #[name(subject_entry)]
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

                            #[name(attachments_slot)]
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 2,
                                add_css_class: "composer-attachments",
                                #[watch]
                                set_visible: !model.attachments.is_empty(),
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
        init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut model = Composer {
            store: init.store,
            identity_labels: gtk::StringList::new(&[]),
            identities: Vec::new(),
            selected_identity: 0,
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            subject: String::new(),
            cc_visible: false,
            attachments: Vec::new(),
            in_reply_to: None,
            references: Vec::new(),
            pending: init.initial,
            to_entry: None,
            cc_entry: None,
            bcc_entry: None,
            subject_entry: None,
            identity_dropdown: None,
            body_view: None,
            attachments_slot: None,
            save_draft_button: None,
            toast_overlay: None,
            draft: None,
        };
        let widgets = view_output!();
        model.to_entry = Some(widgets.to_entry.clone());
        model.cc_entry = Some(widgets.cc_entry.clone());
        model.bcc_entry = Some(widgets.bcc_entry.clone());
        model.subject_entry = Some(widgets.subject_entry.clone());
        model.identity_dropdown = Some(widgets.identity_dropdown.clone());
        model.body_view = Some(widgets.body_view.clone());
        model.attachments_slot = Some(widgets.attachments_slot.clone());
        model.save_draft_button = Some(widgets.save_draft_button.clone());
        model.toast_overlay = Some(widgets.toast_overlay.clone());
        widgets.save_draft_button.set_sensitive(false);

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
                if let Some(draft) = self.pending.take() {
                    self.apply_draft(draft);
                }
                self.post_identities();
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
                let mut draft = draft(
                    identity,
                    &self.to,
                    &self.cc,
                    &self.bcc,
                    &self.subject,
                    &self.body(),
                );
                draft.attachments = self.attachments.clone();
                draft.in_reply_to = self.in_reply_to.clone();
                draft.references = self.references.clone();
                draft.draft = self.draft.clone();
                let _ = sender.output(ComposerOutput::Send(draft));
            }
            ComposerMsg::SaveDraft => {
                let Some(identity) = self.identity() else {
                    return;
                };
                let mut draft = draft(
                    Some(identity),
                    &self.to,
                    &self.cc,
                    &self.bcc,
                    &self.subject,
                    &self.body(),
                );
                draft.attachments = self.attachments.clone();
                draft.in_reply_to = self.in_reply_to.clone();
                draft.references = self.references.clone();
                draft.draft = self.draft.clone();
                let _ = sender.output(ComposerOutput::SaveDraft(draft));
            }
            ComposerMsg::DraftSaved { folder, uid } => {
                let account_id = self.identity().map(|identity| identity.account_id);
                if let Some(account_id) = account_id {
                    self.draft = Some(DraftRef {
                        account_id,
                        folder,
                        uid,
                    });
                }
                if let Some(overlay) = &self.toast_overlay {
                    overlay.add_toast(adw::Toast::new("Draft saved"));
                }
            }
        }
    }
}

impl Composer {
    fn identity(&self) -> Option<Identity> {
        self.identities.get(self.selected_identity).cloned()
    }

    fn apply_draft(&mut self, draft: MessageDraft) {
        self.to = draft.to.clone();
        self.cc = draft.cc.clone();
        self.bcc = draft.bcc.clone();
        self.subject = draft.subject.clone();
        self.cc_visible = !draft.cc.is_empty() || !draft.bcc.is_empty();
        self.attachments = draft.attachments.clone();
        self.in_reply_to = draft.in_reply_to.clone();
        self.references = draft.references.clone();
        self.draft = draft.draft.clone();

        if let Some(entry) = &self.to_entry {
            entry.set_text(&draft.to);
        }
        if let Some(entry) = &self.cc_entry {
            entry.set_text(&draft.cc);
        }
        if let Some(entry) = &self.bcc_entry {
            entry.set_text(&draft.bcc);
        }
        if let Some(entry) = &self.subject_entry {
            entry.set_text(&draft.subject);
        }
        if let Some(view) = &self.body_view {
            view.buffer().set_text(&draft.body);
        }

        if let Some(index) = draft.identity.as_ref().and_then(|identity| {
            self.identities
                .iter()
                .position(|known| known.account_id == identity.account_id)
        }) {
            self.selected_identity = index;
            if let Some(dropdown) = &self.identity_dropdown {
                dropdown.set_selected(index as u32);
            }
        }

        self.sync_attachments();
    }

    fn post_identities(&self) {
        let identity = self.identities.get(self.selected_identity);
        if let Some(button) = &self.save_draft_button {
            button.set_sensitive(identity.is_some());
        }
    }

    fn sync_attachments(&mut self) {
        let Some(slot) = &self.attachments_slot else {
            return;
        };
        while let Some(child) = slot.first_child() {
            slot.remove(&child);
        }
        for attachment in &self.attachments {
            let label = gtk::Label::new(Some(&attachment.filename));
            label.set_xalign(0.0);
            label.add_css_class("dim-label");
            slot.append(&label);
        }
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
