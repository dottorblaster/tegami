// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A private D-Bus session bus for the account integration tests.
//!
//! These tests impersonate the GOA and EDS services by claiming their
//! well-known bus names. Doing that on the user's real session bus hijacks
//! Evolution's account backend — it suddenly sees the mock fixtures and loses
//! the real accounts — so every test talks to a freshly spawned, throwaway
//! `dbus-daemon` instead.

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

use zbus::connection::{Builder, Connection};

pub struct TestBus {
    address: String,
    daemon: Child,
    _stdout: ChildStdout,
}

impl TestBus {
    pub fn start() -> Option<Self> {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = daemon.stdout.take()?;
        let mut address = String::new();
        BufReader::new(&mut stdout).read_line(&mut address).ok()?;
        let address = address.trim().to_string();
        if address.is_empty() {
            return None;
        }
        Some(Self {
            address,
            daemon,
            _stdout: stdout,
        })
    }

    pub async fn connection(&self, names: &[&str]) -> Option<Connection> {
        let mut builder = Builder::address(self.address.as_str()).ok()?;
        for name in names {
            builder = builder.name(*name).ok()?;
        }
        builder.build().await.ok()
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}
