// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Caching of OAuth2 access tokens retrieved through
//! [`crate::goa::OAuth2BasedProxy::get_access_token`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use zbus::connection::Connection;

use crate::goa::OAuth2BasedProxy;

const DEFAULT_REFRESH_MARGIN: Duration = Duration::from_secs(30);

pub struct OAuth2TokenCache {
    tokens: Mutex<HashMap<String, AccessToken>>,
    refresh_margin: Duration,
}

struct AccessToken {
    value: String,
    expires_at: Instant,
}

impl OAuth2TokenCache {
    pub fn new() -> Self {
        Self::with_refresh_margin(DEFAULT_REFRESH_MARGIN)
    }

    pub fn with_refresh_margin(refresh_margin: Duration) -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
            refresh_margin,
        }
    }

    pub async fn access_token(
        &self,
        connection: &Connection,
        account_path: &str,
    ) -> zbus::Result<String> {
        {
            let tokens = self.tokens.lock().unwrap();
            if let Some(token) = tokens.get(account_path)
                && token.expires_at > Instant::now()
            {
                return Ok(token.value.clone());
            }
        }
        let (value, expires_in) = self.fetch(connection, account_path).await?;
        let lifetime = Duration::from_secs(expires_in.max(0) as u64);
        let token = AccessToken {
            value,
            expires_at: Instant::now() + lifetime.saturating_sub(self.refresh_margin),
        };
        let value = token.value.clone();
        self.tokens
            .lock()
            .unwrap()
            .insert(account_path.to_string(), token);
        Ok(value)
    }

    async fn fetch(
        &self,
        connection: &Connection,
        account_path: &str,
    ) -> zbus::Result<(String, i32)> {
        let proxy = OAuth2BasedProxy::builder(connection)
            .path(account_path)?
            .build()
            .await?;
        proxy.get_access_token().await
    }
}

impl Default for OAuth2TokenCache {
    fn default() -> Self {
        Self::new()
    }
}
