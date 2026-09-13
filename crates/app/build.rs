// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if let Some(bundle) = env::var_os("TEGAMI_RESOURCE_BUNDLE") {
        let path = PathBuf::from(&bundle);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
            println!("cargo:rustc-env=TEGAMI_RESOURCE_BUNDLE={}", path.display());
            return;
        }
    }

    let data_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../data");
    let xml = data_dir.join("tegami.gresource.xml");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let bundle = out_dir.join("tegami.gresource");

    let status = Command::new("blueprint-compiler")
        .arg("batch-compile")
        .arg(&out_dir)
        .arg(&data_dir)
        .arg(data_dir.join("ui").join("window.blp"))
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir")
        .arg(&out_dir)
        .arg("--sourcedir")
        .arg(&data_dir)
        .arg("--target")
        .arg(&bundle)
        .arg(&xml)
        .status()
        .unwrap();
    assert!(status.success());

    println!("cargo:rerun-if-changed={}", xml.display());
    println!(
        "cargo:rerun-if-changed={}",
        data_dir.join("resources").display()
    );
    println!("cargo:rerun-if-changed={}", data_dir.join("ui").display());
    println!(
        "cargo:rustc-env=TEGAMI_RESOURCE_BUNDLE={}",
        bundle.display()
    );
}
