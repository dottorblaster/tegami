// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SASL authenticators used by the backend.

use async_imap::Authenticator;

/// The XOAUTH2 SASL mechanism: `user=<user>\x01auth=Bearer <token>\x01\x01`.
pub struct Xoauth2 {
    user: String,
    token: String,
}

impl Xoauth2 {
    pub fn new(user: String, token: String) -> Self {
        Self { user, token }
    }
}

impl Authenticator for Xoauth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> String {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}
