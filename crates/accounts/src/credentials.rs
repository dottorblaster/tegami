// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Credential retrieval for unified accounts.
//!
//! GOA-backed accounts fetch OAuth2 access tokens through the GOA
//! daemon (cached in-process until expiry) and plain passwords through
//! the `PasswordBased` interface; EDS-backed accounts resolve plain
//! passwords through the Secret Service.

use zbus::connection::Connection;

use oo7::Keyring;

use crate::eds::secret::password_for_source;
use crate::goa::{OAuth2TokenCache, PasswordBasedProxy};
use crate::model::Account;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credentials {
    Password(String),
    OAuth2(String),
}

#[derive(Debug)]
pub enum CredentialError {
    ZBus(zbus::Error),
    Oo7(oo7::Error),
}

impl From<zbus::Error> for CredentialError {
    fn from(err: zbus::Error) -> Self {
        Self::ZBus(err)
    }
}

impl From<oo7::Error> for CredentialError {
    fn from(err: oo7::Error) -> Self {
        Self::Oo7(err)
    }
}

pub struct CredentialWorker {
    keyring: Keyring,
    token_cache: OAuth2TokenCache,
}

impl CredentialWorker {
    #[allow(clippy::result_large_err)]
    pub async fn new() -> oo7::Result<Self> {
        Ok(Self {
            keyring: Keyring::new().await?,
            token_cache: OAuth2TokenCache::new(),
        })
    }

    pub async fn access_token(
        &self,
        connection: &Connection,
        account: &Account,
    ) -> zbus::Result<Option<String>> {
        match &account.goa {
            Some(goa) if goa.oauth2 => Ok(Some(
                self.token_cache
                    .access_token(connection, &goa.account_path)
                    .await?,
            )),
            _ => Ok(None),
        }
    }

    #[allow(clippy::result_large_err)]
    pub async fn password(
        &self,
        connection: &Connection,
        account: &Account,
    ) -> Result<Option<String>, CredentialError> {
        match &account.goa {
            Some(goa) if !goa.oauth2 => {
                let proxy = PasswordBasedProxy::builder(connection)
                    .path(goa.account_path.as_str())?
                    .build()
                    .await?;
                Ok(Some(proxy.get_password("imap-password").await?))
            }
            _ => match &account.eds {
                Some(eds) => Ok(password_for_source(&self.keyring, &eds.uid).await?),
                None => Ok(None),
            },
        }
    }

    #[allow(clippy::result_large_err)]
    pub async fn credentials(
        &self,
        connection: &Connection,
        account: &Account,
    ) -> Result<Option<Credentials>, CredentialError> {
        if let Some(token) = self
            .access_token(connection, account)
            .await
            .map_err(CredentialError::from)?
        {
            return Ok(Some(Credentials::OAuth2(token)));
        }
        Ok(self
            .password(connection, account)
            .await?
            .map(Credentials::Password))
    }
}
