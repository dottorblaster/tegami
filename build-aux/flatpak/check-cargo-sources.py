#!/usr/bin/env python3

import json
import sys
import tomllib

lock = tomllib.load(open("Cargo.lock", "rb"))["package"]
expected = {
    f"cargo/vendor/{p['name']}-{p['version']}" for p in lock if p.get("source")
}
sources = json.load(open("build-aux/flatpak/cargo-sources.json"))
vendored = {e["dest"] for e in sources if e.get("type") == "archive"}

missing = expected - vendored
extra = vendored - expected

if missing or extra:
    print("build-aux/flatpak/cargo-sources.json is out of date with Cargo.lock")
    print("regenerate it with:")
    print("  flatpak-cargo-generator Cargo.lock -o build-aux/flatpak/cargo-sources.json")
    if missing:
        print("missing from vendor:", sorted(missing)[:8])
    if extra:
        print("extra in vendor:", sorted(extra)[:8])
    sys.exit(1)

print("build-aux/flatpak/cargo-sources.json is up to date")