// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SASL authenticators used by the backend.

use async_imap::Authenticator;

/// The XOAUTH2 SASL mechanism: `user=<user>\x01auth=Bearer <token>\x01\x01`.
pub struct Xoauth2 {
    user: String,
    token: String,
    sent: bool,
}

impl Xoauth2 {
    pub fn new(user: String, token: String) -> Self {
        Self {
            user,
            token,
            sent: false,
        }
    }
}

impl Authenticator for Xoauth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> String {
        // The first challenge carries the credentials; a second challenge means
        // the server rejected them and expects an empty response to end the
        // exchange. Resending the token here would loop forever.
        if self.sent {
            return String::new();
        }
        self.sent = true;
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_the_token_once_then_an_empty_response() {
        let mut authenticator = Xoauth2::new("me@example.org".to_string(), "token".to_string());

        assert_eq!(
            authenticator.process(b""),
            "user=me@example.org\x01auth=Bearer token\x01\x01"
        );
        assert_eq!(authenticator.process(b"error"), "");
        assert_eq!(authenticator.process(b"error"), "");
    }
}
