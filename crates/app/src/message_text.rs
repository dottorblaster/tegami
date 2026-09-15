// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Formatting of message headers for the list and conversation views.

use mail_core::store::MessageRecord;
use relm4::gtk::glib;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub(crate) fn display_subject(subject: &str) -> String {
    let subject = subject.trim();
    if subject.is_empty() {
        "(no subject)".to_string()
    } else {
        subject.to_string()
    }
}

pub(crate) fn display_sender(record: &MessageRecord) -> String {
    if let Some(name) = non_empty(record.from_name.as_deref()) {
        return name.to_string();
    }
    if let Some(address) = non_empty(record.from_addr.as_deref()) {
        return address.to_string();
    }
    "(unknown sender)".to_string()
}

pub(crate) fn format_timestamp(timestamp: i64, now: &glib::DateTime) -> String {
    match glib::DateTime::from_unix_local(timestamp) {
        Ok(date) => format_relative(&date, now),
        Err(_) => String::new(),
    }
}

fn format_relative(date: &glib::DateTime, now: &glib::DateTime) -> String {
    if date.year() == now.year()
        && date.month() == now.month()
        && date.day_of_month() == now.day_of_month()
    {
        format!("{:02}:{:02}", date.hour(), date.minute())
    } else if date.year() == now.year() {
        format!("{} {}", date.day_of_month(), month_name(date.month()))
    } else {
        format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            date.month(),
            date.day_of_month()
        )
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn month_name(month: i32) -> &'static str {
    let index = (month - 1).clamp(0, 11) as usize;
    MONTHS[index]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::store::{BodyState, FLAG_SEEN};

    fn record(uid: u32) -> MessageRecord {
        MessageRecord {
            id: Some(i64::from(uid)),
            folder_id: 1,
            uid,
            modseq: None,
            message_id: None,
            thread_id: None,
            subject: "Hello".to_string(),
            from_addr: Some("ada@lovelace.dev".to_string()),
            from_name: Some("Ada Lovelace".to_string()),
            to_addrs: None,
            cc_addrs: None,
            date_sent: Some(1_700_000_000),
            date_recv: None,
            in_reply_to: None,
            refs: None,
            flags: FLAG_SEEN,
            has_attach: false,
            size: None,
            structure: None,
            raw_path: None,
            body_state: BodyState::None,
        }
    }

    fn utc(timestamp: i64) -> glib::DateTime {
        glib::DateTime::from_unix_utc(timestamp).expect("valid timestamp")
    }

    #[test]
    fn display_subject_falls_back_when_blank() {
        assert_eq!(display_subject("  "), "(no subject)");
        assert_eq!(display_subject(" Meeting "), "Meeting");
    }

    #[test]
    fn display_sender_prefers_name_then_address() {
        let mut record = record(1);
        assert_eq!(display_sender(&record), "Ada Lovelace");

        record.from_name = Some("   ".to_string());
        assert_eq!(display_sender(&record), "ada@lovelace.dev");

        record.from_addr = None;
        assert_eq!(display_sender(&record), "(unknown sender)");
    }

    #[test]
    fn format_relative_picks_day_year_and_clock() {
        let now = utc(1_700_000_000);

        assert_eq!(format_relative(&utc(1_699_990_000), &now), "19:26");
        assert_eq!(format_relative(&utc(1_699_000_000), &now), "3 Nov");
        assert_eq!(format_relative(&utc(1_600_000_000), &now), "2020-09-13");
    }
}
