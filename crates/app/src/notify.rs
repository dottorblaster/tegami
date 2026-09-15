// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! New-mail notifications, sent through `GNotification` so the notification
//! portal delivers them.

use relm4::gtk::gio;
use relm4::gtk::gio::prelude::*;

use crate::config;

const MAX_INDIVIDUAL: usize = 3;

const OPEN_ACTION: &str = "app.open-message";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MailNotice {
    pub folder_id: i64,
    pub uid: u32,
    pub sender: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MailNotification {
    pub id: String,
    pub title: String,
    pub body: String,
    pub target: Option<String>,
}

pub(crate) fn notifications(folder_title: &str, notices: &[MailNotice]) -> Vec<MailNotification> {
    match notices {
        [] => Vec::new(),
        [notice] => vec![individual(notice)],
        notices if notices.len() <= MAX_INDIVIDUAL => notices.iter().map(individual).collect(),
        notices => {
            let folder_id = notices[0].folder_id;
            let target = notices
                .iter()
                .max_by_key(|notice| notice.uid)
                .map(|notice| format!("{}:{}", notice.folder_id, notice.uid));
            vec![MailNotification {
                id: format!("mail-summary-{folder_id}"),
                title: format!("{} new messages", notices.len()),
                body: folder_title.to_string(),
                target,
            }]
        }
    }
}

fn individual(notice: &MailNotice) -> MailNotification {
    MailNotification {
        id: format!("mail-{}-{}", notice.folder_id, notice.uid),
        title: notice.sender.clone(),
        body: notice.subject.clone(),
        target: Some(format!("{}:{}", notice.folder_id, notice.uid)),
    }
}

pub(crate) fn send(notifications: &[MailNotification]) {
    let application = relm4::main_application();
    for item in notifications {
        let notification = gio::Notification::new(&item.title);
        notification.set_body(Some(&item.body));
        notification.set_icon(&gio::ThemedIcon::new(config::APP_ID));
        notification.set_priority(gio::NotificationPriority::Normal);
        if let Some(target) = &item.target {
            notification
                .set_default_action_and_target_value(OPEN_ACTION, Some(&target.to_variant()));
        }
        application.send_notification(Some(&item.id), &notification);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(uid: u32) -> MailNotice {
        MailNotice {
            folder_id: 7,
            uid,
            sender: format!("Sender {uid}"),
            subject: format!("Subject {uid}"),
        }
    }

    #[test]
    fn nothing_to_notify_about() {
        assert!(notifications("Inbox", &[]).is_empty());
    }

    #[test]
    fn single_message_uses_sender_and_subject() {
        let items = notifications("Inbox", &[notice(4)]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "mail-7-4");
        assert_eq!(items[0].title, "Sender 4");
        assert_eq!(items[0].body, "Subject 4");
        assert_eq!(items[0].target.as_deref(), Some("7:4"));
    }

    #[test]
    fn small_batches_stay_individual() {
        let notices: Vec<MailNotice> = (1..=3).map(notice).collect();
        let items = notifications("Inbox", &notices);
        assert_eq!(items.len(), 3);
        assert_eq!(items[2].id, "mail-7-3");
    }

    #[test]
    fn larger_batches_collapse_into_a_summary_pointing_at_the_newest() {
        let notices: Vec<MailNotice> = (1..=9).map(notice).collect();
        let items = notifications("Inbox", &notices);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "mail-summary-7");
        assert_eq!(items[0].title, "9 new messages");
        assert_eq!(items[0].body, "Inbox");
        assert_eq!(items[0].target.as_deref(), Some("7:9"));
    }
}
