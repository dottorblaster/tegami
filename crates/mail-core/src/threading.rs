// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Conversation threading.
//!
//! Messages are linked into conversations through the `References` and
//! `In-Reply-To` headers, falling back to a normalized subject whenever the
//! reference chain cannot be resolved against the stored messages. Every
//! conversation is identified by the row id of its root message, so replies
//! that arrive before their parent still converge on a single thread.

use std::collections::HashMap;

/// The headers and identity of one message participating in threading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessage {
    pub id: i64,
    pub account_id: i64,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
}

/// Computes the thread root id for every message, preserving input order.
///
/// Messages sharing a `Message-ID` reference chain are joined, then
/// reference-less (or unresolved) messages are grouped by normalized subject
/// within their account. The resulting id is the row id of the conversation's
/// root message, or the earliest message when several roots are merged.
pub fn thread_roots(messages: &[ThreadMessage]) -> Vec<i64> {
    let mut forest = Forest::new(messages.len());

    let mut by_message_id: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        if let Some(message_id) = normalized_message_id(message.message_id.as_deref()) {
            by_message_id.entry(message_id).or_default().push(index);
        }
    }
    for indices in by_message_id.values() {
        for &index in &indices[1..] {
            forest.union(indices[0], index);
        }
    }

    let mut linked = vec![false; messages.len()];
    for (index, message) in messages.iter().enumerate() {
        for ancestor in ancestors(message) {
            if let Some(parents) = by_message_id.get(ancestor) {
                for &parent in parents {
                    forest.union(index, parent);
                }
                linked[index] = true;
            }
        }
    }

    let subjects: Vec<String> = messages
        .iter()
        .map(|message| normalized_subject(&message.subject))
        .collect();
    let mut by_subject: HashMap<(i64, &str), Vec<usize>> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        if !subjects[index].is_empty() {
            by_subject
                .entry((message.account_id, subjects[index].as_str()))
                .or_default()
                .push(index);
        }
    }
    for (index, message) in messages.iter().enumerate() {
        if linked[index] || subjects[index].is_empty() {
            continue;
        }
        if let Some(peers) = by_subject.get(&(message.account_id, subjects[index].as_str())) {
            for &peer in peers {
                forest.union(index, peer);
            }
        }
    }

    let mut has_parent = vec![false; messages.len()];
    for (index, message) in messages.iter().enumerate() {
        has_parent[index] = ancestors(message).any(|ancestor| by_message_id.contains_key(ancestor));
    }

    let mut roots: HashMap<usize, i64> = HashMap::new();
    let mut earliest: HashMap<usize, i64> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        let component = forest.find(index);
        keep_earliest(earliest.entry(component).or_insert(message.id), message.id);
        if !has_parent[index] {
            keep_earliest(roots.entry(component).or_insert(message.id), message.id);
        }
    }

    (0..messages.len())
        .map(|index| {
            let component = forest.find(index);
            roots
                .get(&component)
                .or_else(|| earliest.get(&component))
                .copied()
                .unwrap_or(messages[index].id)
        })
        .collect()
}

/// Parses the JSON string array stored in the `refs` column.
pub fn parse_references(value: Option<&str>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let mut references = Vec::new();
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '"' {
            continue;
        }
        let mut current = String::new();
        while let Some(character) = characters.next() {
            match character {
                '\\' => {
                    if let Some(escaped) = characters.next() {
                        current.push(escaped);
                    }
                }
                '"' => break,
                other => current.push(other),
            }
        }
        references.push(current);
    }
    references
}

/// Strips common reply/forward prefixes and normalizes case and whitespace.
pub fn normalized_subject(subject: &str) -> String {
    let mut value = subject.trim();
    while let Some(stripped) = strip_reply_prefix(value) {
        value = stripped.trim_start();
    }
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalized_message_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn ancestors(message: &ThreadMessage) -> impl Iterator<Item = &str> {
    message
        .references
        .iter()
        .map(String::as_str)
        .chain(message.in_reply_to.as_deref())
        .filter(|value| !value.trim().is_empty())
}

fn keep_earliest(slot: &mut i64, candidate: i64) {
    if candidate < *slot {
        *slot = candidate;
    }
}

fn strip_reply_prefix(value: &str) -> Option<&str> {
    for tag in ["re", "fwd", "fw", "aw", "sv"] {
        let Some(head) = value.get(..tag.len()) else {
            continue;
        };
        if !head.eq_ignore_ascii_case(tag) {
            continue;
        }
        let mut rest = &value[tag.len()..];
        if let Some(inner) = rest.strip_prefix('[')
            && let Some(end) = inner.find(']')
        {
            let count = &inner[..end];
            if !count.is_empty() && count.chars().all(|digit| digit.is_ascii_digit()) {
                rest = &inner[end + 1..];
            }
        }
        let rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix(':') {
            return Some(after);
        }
    }
    None
}

struct Forest {
    parent: Vec<usize>,
}

impl Forest {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
        }
    }

    fn find(&mut self, index: usize) -> usize {
        let mut root = index;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut current = index;
        while self.parent[current] != root {
            let next = self.parent[current];
            self.parent[current] = root;
            current = next;
        }
        root
    }

    fn union(&mut self, left: usize, right: usize) {
        let (left, right) = (self.find(left), self.find(right));
        if left != right {
            self.parent[right] = left;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: i64, subject: &str) -> ThreadMessage {
        ThreadMessage {
            id,
            account_id: 1,
            message_id: Some(format!("<{id}@example.org>")),
            in_reply_to: None,
            references: Vec::new(),
            subject: subject.to_string(),
        }
    }

    #[test]
    fn normalized_subject_strips_reply_and_forward_prefixes() {
        assert_eq!(normalized_subject("Re: Hello"), "hello");
        assert_eq!(normalized_subject("RE:  Hello World "), "hello world");
        assert_eq!(normalized_subject("Re[2]: Hello"), "hello");
        assert_eq!(normalized_subject("Fwd: Fw: Re: Hello"), "hello");
        assert_eq!(normalized_subject("Aw: Hello"), "hello");
        assert_eq!(normalized_subject("Re:"), "");
        assert_eq!(normalized_subject("Reply"), "reply");
    }

    #[test]
    fn parse_references_reads_json_string_arrays() {
        assert!(parse_references(None).is_empty());
        assert!(parse_references(Some("[]")).is_empty());
        assert_eq!(
            parse_references(Some(r#"["<a@x>","<b@x>"]"#)),
            vec!["<a@x>".to_string(), "<b@x>".to_string()]
        );
        assert_eq!(
            parse_references(Some(r#"["<a\"b@x>"]"#)),
            vec!["<a\"b@x>".to_string()]
        );
    }

    #[test]
    fn replies_share_the_root_thread_id() {
        let mut reply = message(2, "Re: Hello");
        reply.in_reply_to = Some("<1@example.org>".to_string());
        let roots = thread_roots(&[message(1, "Hello"), reply]);
        assert_eq!(roots, vec![1, 1]);
    }

    #[test]
    fn root_wins_even_when_the_reply_is_stored_first() {
        let mut reply = message(1, "Re: Hello");
        reply.message_id = Some("<reply@example.org>".to_string());
        reply.in_reply_to = Some("<root@example.org>".to_string());
        let mut root = message(2, "Hello");
        root.message_id = Some("<root@example.org>".to_string());
        let roots = thread_roots(&[reply, root]);
        assert_eq!(roots, vec![2, 2]);
    }

    #[test]
    fn references_link_deep_replies() {
        let mut first = message(1, "Hello");
        first.message_id = Some("<a@x>".to_string());
        let mut second = message(2, "Re: Hello");
        second.message_id = Some("<b@x>".to_string());
        second.references = vec!["<a@x>".to_string()];
        let mut third = message(3, "Re: Hello");
        third.message_id = Some("<c@x>".to_string());
        third.references = vec!["<a@x>".to_string(), "<b@x>".to_string()];
        let roots = thread_roots(&[first, second, third]);
        assert_eq!(roots, vec![1, 1, 1]);
    }

    #[test]
    fn subject_fallback_groups_reference_less_messages() {
        let roots = thread_roots(&[message(1, "Lunch?"), message(2, "Re: Lunch?")]);
        assert_eq!(roots, vec![1, 1]);
    }

    #[test]
    fn subject_fallback_does_not_cross_accounts() {
        let mut other = message(2, "Re: Lunch?");
        other.account_id = 2;
        let roots = thread_roots(&[message(1, "Lunch?"), other]);
        assert_eq!(roots, vec![1, 2]);
    }

    #[test]
    fn subject_fallback_joins_an_unresolved_reply_to_a_thread() {
        let mut root = message(1, "Hello");
        root.message_id = Some("<root@x>".to_string());
        let mut linked = message(2, "Re: Hello");
        linked.message_id = Some("<linked@x>".to_string());
        linked.in_reply_to = Some("<root@x>".to_string());
        let mut orphan = message(3, "Re: Hello");
        orphan.message_id = Some("<orphan@x>".to_string());
        orphan.in_reply_to = Some("<missing@x>".to_string());
        let roots = thread_roots(&[root, linked, orphan]);
        assert_eq!(roots, vec![1, 1, 1]);
    }

    #[test]
    fn unrelated_messages_keep_distinct_threads() {
        let roots = thread_roots(&[message(1, "Hello"), message(2, "Goodbye")]);
        assert_eq!(roots, vec![1, 2]);
    }

    #[test]
    fn empty_input_yields_no_roots() {
        assert!(thread_roots(&[]).is_empty());
    }
}
