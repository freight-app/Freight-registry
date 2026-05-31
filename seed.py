#!/usr/bin/env python3
"""
Seed the freight registry with example packages, users, and tokens.

Usage:
    python3 seed.py [--data DIR] [--url URL]

Defaults:
    --data /tmp/freight-seed
    --url  http://localhost:7878

The script:
  1. Creates the data dir and initialises the DB via `freight-registry user add`
  2. Creates sample users (alice, bob, carol) with tokens
  3. Publishes example packages via the HTTP API
  4. Uploads a sample README for each package
"""

import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


# ── Config ────────────────────────────────────────────────────────────────────

REGISTRY   = str(Path(__file__).parent.parent.parent / "target" / "debug" / "freight-registry")
DATA_DIR   = "/tmp/freight-seed"
BASE_URL   = "http://localhost:7878"
SERVER_PID = None

USERS = [
    {"username": "alice",   "email": "alice@example.com",  "password": "hunter2!"},
    {"username": "bob",     "email": "bob@example.com",    "password": "password1"},
    {"username": "carol",   "email": "carol@example.com",  "password": "letmein99"},
]

PACKAGES = [
    {
        "name": "libvec",
        "vers": "1.0.0",
        "description": "A fast, header-only SIMD vector math library for C and C++",
        "license": "MIT",
        "keywords": ["math", "linear-algebra", "simd", "geometry"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/example/libvec/archive/v1.0.0.tar.gz",
        "deps": {"gtest": "1.14"},
        "owner": "alice",
        "readme": """\
# libvec

A fast, header-only SIMD vector math library for C and C++.

Provides `vec2`, `vec3`, `vec4`, `mat4`, and quaternion types with SSE2/AVX2 acceleration.

## Install

```toml
[dependencies]
libvec = "1.0"
```

## Usage

```c
#include <vec/vec3.h>

vec3 a = {1.0f, 2.0f, 3.0f};
vec3 b = {4.0f, 5.0f, 6.0f};
vec3 c = vec3_add(a, b);
```
""",
    },
    {
        "name": "libvec",
        "vers": "1.1.0",
        "description": "A fast, header-only SIMD vector math library for C and C++",
        "license": "MIT",
        "keywords": ["math", "linear-algebra", "simd", "geometry"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/example/libvec/archive/v1.1.0.tar.gz",
        "deps": {"gtest": "1.14"},
        "owner": "alice",
        "readme": None,   # inherit from 1.0.0
    },
    {
        "name": "zlib-ng",
        "vers": "2.1.6",
        "description": "zlib replacement with optimizations for next generation systems",
        "license": "zlib",
        "keywords": ["compression", "zlib", "deflate"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/zlib-ng/zlib-ng/archive/2.1.6.tar.gz",
        "deps": {},
        "owner": "bob",
        "readme": """\
# zlib-ng

A zlib replacement with optimizations for next generation systems.

Drop-in replacement for zlib with improved performance on modern hardware.

## Features
- Fully compatible with zlib 1.2.x API/ABI
- ARM, Power, S390 SIMD optimizations
- 64-bit support improvements
""",
    },
    {
        "name": "openssl",
        "vers": "3.3.0",
        "description": "TLS/SSL and cryptography library",
        "license": "Apache-2.0",
        "keywords": ["tls", "ssl", "crypto", "security"],
        "build_system": "make",
        "upstream_url": "https://github.com/openssl/openssl/archive/openssl-3.3.0.tar.gz",
        "deps": {},
        "owner": "bob",
        "readme": """\
# OpenSSL

A robust, full-featured open-source toolkit implementing the Secure Sockets Layer (SSL v2/v3)
and Transport Layer Security (TLS v1) protocols as well as a general-purpose cryptography library.

## Install

```toml
[dependencies]
openssl = "3.3"
```
""",
    },
    {
        "name": "fmt",
        "vers": "10.2.1",
        "description": "A modern C++ formatting library — fast, safe alternative to printf and iostreams",
        "license": "MIT",
        "keywords": ["formatting", "string", "printf", "cpp"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/fmtlib/fmt/archive/10.2.1.tar.gz",
        "deps": {},
        "owner": "alice",
        "readme": """\
# {fmt}

An open-source formatting library providing a fast and safe alternative to C stdio and C++ iostreams.

```cpp
#include <fmt/core.h>

int main() {
    fmt::print("Hello, {}!\n", "world");
}
```
""",
    },
    {
        "name": "spdlog",
        "vers": "1.13.0",
        "description": "Fast C++ logging library",
        "license": "MIT",
        "keywords": ["logging", "cpp", "async"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/gabime/spdlog/archive/v1.13.0.tar.gz",
        "deps": {"fmt": "10.2"},
        "owner": "alice",
        "readme": """\
# spdlog

Very fast, header-only/compiled, C++ logging library.

## Features
- Very fast — much faster than most logging libraries
- Headers only, just copy and use. Or compile if needed
- Feature-rich: call stack info, logger filtering, custom formatting, etc.
""",
    },
    {
        "name": "nlohmann-json",
        "vers": "3.11.3",
        "description": "JSON for modern C++ — single-header library",
        "license": "MIT",
        "keywords": ["json", "cpp", "serialization", "parsing"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/nlohmann/json/archive/v3.11.3.tar.gz",
        "deps": {},
        "owner": "carol",
        "readme": """\
# nlohmann/json

JSON for Modern C++. Intuitive syntax, type-safe, zero dependencies.

```cpp
#include <nlohmann/json.hpp>
using json = nlohmann::json;

json j = {{"name","freight"},{"version","1.0"}};
std::cout << j.dump(2) << std::endl;
```
""",
    },
    {
        "name": "abseil-cpp",
        "vers": "20240116.0",
        "description": "Abseil Common Libraries for C++ — Google's open-source collection",
        "license": "Apache-2.0",
        "keywords": ["cpp", "containers", "strings", "utilities"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/abseil/abseil-cpp/archive/20240116.0.tar.gz",
        "deps": {},
        "owner": "carol",
        "readme": """\
# Abseil C++

Abseil is an open-source collection of C++ library code designed to augment the C++ standard library.

Includes: strings, containers, algorithm, memory, numeric, time, and more.
""",
    },
    {
        "name": "libcurl",
        "vers": "8.7.1",
        "description": "URL transfer library supporting HTTP, FTP, SMTP, and more",
        "license": "curl",
        "keywords": ["http", "network", "transfer", "curl"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/curl/curl/archive/curl-8_7_1.tar.gz",
        "deps": {"openssl": "3.3", "zlib-ng": "2.1"},
        "owner": "bob",
        "readme": """\
# libcurl

libcurl is a free and easy-to-use client-side URL transfer library.
Supports DICT, FILE, FTP, FTPS, GOPHER, GOPHERS, HTTP, HTTPS, IMAP, IMAPS,
LDAP, LDAPS, MQTT, POP3, POP3S, RTMP, RTMPS, RTSP, SMB, SMBS, SMTP, SMTPS,
TELNET, TFTP, WS, and WSS.
""",
    },
    {
        "name": "eigen",
        "vers": "3.4.0",
        "description": "C++ template library for linear algebra: matrices, vectors, solvers",
        "license": "MPL-2.0",
        "keywords": ["math", "linear-algebra", "matrix", "cpp"],
        "build_system": "cmake",
        "upstream_url": "https://gitlab.com/libeigen/eigen/-/archive/3.4.0/eigen-3.4.0.tar.gz",
        "deps": {},
        "owner": "alice",
        "readme": """\
# Eigen

Eigen is a C++ template library for linear algebra: matrices, vectors, numerical solvers.

```cpp
#include <Eigen/Dense>
using namespace Eigen;

Matrix3f A;
Vector3f b;
Vector3f x = A.colPivHouseholderQr().solve(b);
```
""",
    },
    {
        "name": "protobuf",
        "vers": "26.1.0",
        "description": "Google's Protocol Buffers — language-neutral serialization",
        "license": "BSD-3-Clause",
        "keywords": ["serialization", "protobuf", "rpc", "proto"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/protocolbuffers/protobuf/archive/v26.1.tar.gz",
        "deps": {"abseil-cpp": "20240116.0"},
        "owner": "carol",
        "readme": """\
# Protocol Buffers

Protocol Buffers are Google's language-neutral, platform-neutral, extensible mechanism
for serializing structured data.

Define your schema in `.proto` files:
```proto
message Person {
  string name = 1;
  int32  id   = 2;
  string email = 3;
}
```
""",
    },
    {
        "name": "googletest",
        "vers": "1.14.0",
        "description": "Google's C++ test and mock framework",
        "license": "BSD-3-Clause",
        "keywords": ["testing", "cpp", "mock", "gtest"],
        "build_system": "cmake",
        "upstream_url": "https://github.com/google/googletest/archive/v1.14.0.tar.gz",
        "deps": {},
        "owner": "carol",
        "readme": """\
# GoogleTest

Google's C++ testing and mocking framework. Runs on Linux, macOS, Windows, Cygwin, and more.

```cpp
TEST(FactorialTest, HandlesPositiveInput) {
  EXPECT_EQ(Factorial(1), 1);
  EXPECT_EQ(Factorial(8), 40320);
}
```
""",
    },
]


# ── Helpers ───────────────────────────────────────────────────────────────────

def run(*args, check=True, capture=False):
    kwargs = {"check": check}
    if capture:
        kwargs |= {"capture_output": True, "text": True}
    return subprocess.run(list(args), **kwargs)


def registry(*args, check=True, capture=False):
    return run(REGISTRY, "--data", DATA_DIR, *args, check=check, capture=capture)


def sha256hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def build_publish_body(meta: dict) -> bytes:
    """Build the freight publish wire format: [u32le JSON len][JSON][u32le tarball len][tarball]"""
    meta_bytes = json.dumps(meta).encode()
    tarball    = b""    # zero-byte tarball — metadata-only package
    return (
        struct.pack("<I", len(meta_bytes)) + meta_bytes +
        struct.pack("<I", len(tarball))   + tarball
    )


def api_request(method: str, path: str, body=None, token: str = None,
                content_type="application/json") -> tuple[int, dict | None]:
    """Returns (status_code, response_body_or_None)."""
    url = BASE_URL + path
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if body is not None:
        headers["Content-Type"] = content_type
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
    else:
        data = None
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req) as resp:
            return resp.status, json.loads(resp.read())
    except urllib.error.HTTPError as e:
        body_text = e.read().decode()
        print(f"  HTTP {e.code} {method} {path}: {body_text}", file=sys.stderr)
        return e.code, None


def wait_for_server(retries=20):
    for i in range(retries):
        try:
            urllib.request.urlopen(BASE_URL + "/health", timeout=1)
            return True
        except Exception:
            time.sleep(0.3)
    return False


# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    global SERVER_PID, DATA_DIR, BASE_URL

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", default=DATA_DIR)
    parser.add_argument("--url",  default=BASE_URL)
    args = parser.parse_args()
    DATA_DIR = args.data
    BASE_URL = args.url.rstrip("/")

    os.makedirs(DATA_DIR, exist_ok=True)

    # ── 1. Create users ───────────────────────────────────────────────────────
    print("Creating users…")
    tokens = {}
    for u in USERS:
        result = registry("user", "add", u["username"],
                          "--email", u["email"],
                          "--password", u["password"],
                          check=False, capture=True)
        if result.returncode != 0 and "already" not in result.stderr.lower():
            print(f"  Warning: {result.stderr.strip()}")

        # Revoke any previous seed token before re-creating
        registry("token", "revoke", "seed-token", "--user", u["username"],
                 check=False, capture=True)
        result = registry("token", "add", "seed-token",
                          "--user", u["username"],
                          check=False, capture=True)
        token = None
        if result.returncode == 0:
            for line in result.stdout.splitlines():
                stripped = line.strip()
                # Token is a 64-char hex string on its own line
                if len(stripped) == 64 and all(c in "0123456789abcdefABCDEF" for c in stripped):
                    token = stripped
                    break
        if token:
            tokens[u["username"]] = token
            print(f"  {u['username']} → token={token[:20]}…")
        else:
            print(f"  Warning: could not get token for {u['username']}: {result.stderr.strip()}")

    if not tokens:
        print("No tokens created — aborting.", file=sys.stderr)
        sys.exit(1)

    # ── 2. Start the server ───────────────────────────────────────────────────
    print(f"\nStarting registry at {BASE_URL}…")
    srv = subprocess.Popen(
        [REGISTRY, "--data", DATA_DIR, "serve",
         "--base-url", BASE_URL, "--bind", "0.0.0.0:7878"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    SERVER_PID = srv.pid

    if not wait_for_server():
        srv.terminate()
        print("Server did not start in time.", file=sys.stderr)
        sys.exit(1)
    print("  Server ready.")

    # ── 3. Publish packages ───────────────────────────────────────────────────
    print("\nPublishing packages…")
    published_readmes = {}

    for pkg in PACKAGES:
        owner  = pkg["owner"]
        token  = tokens.get(owner)
        if not token:
            print(f"  Skipping {pkg['name']} {pkg['vers']} — no token for {owner}")
            continue

        meta = {
            "name":         pkg["name"],
            "vers":         pkg["vers"],
            "description":  pkg.get("description"),
            "license":      pkg.get("license"),
            "upstream_url": pkg.get("upstream_url"),
            "build_system": pkg.get("build_system"),
            "deps":         pkg.get("deps", {}),
        }
        body = build_publish_body(meta)

        status, result = api_request("PUT", "/api/v1/packages", body=body, token=token,
                                     content_type="application/octet-stream")
        if status == 409:
            print(f"  ~ {pkg['name']} {pkg['vers']} (already exists)")
        elif status == 429:
            # Rate limited — wait and retry once
            time.sleep(7)
            status, result = api_request("PUT", "/api/v1/packages", body=body, token=token,
                                         content_type="application/octet-stream")
            if status == 200:
                print(f"  ✓ {pkg['name']} {pkg['vers']} (owner: {owner}, retry)")
            elif status == 409:
                print(f"  ~ {pkg['name']} {pkg['vers']} (already exists)")
            else:
                print(f"  ✗ {pkg['name']} {pkg['vers']}")
                continue
        elif status == 200:
            print(f"  ✓ {pkg['name']} {pkg['vers']} (owner: {owner})")
        else:
            print(f"  ✗ {pkg['name']} {pkg['vers']}")
            continue

        time.sleep(6)  # stay well under 10 writes/min

        # Upload README if provided
        readme = pkg.get("readme")
        if readme is None:
            # Inherit first version's README
            readme = published_readmes.get(pkg["name"])
        if readme:
            rs, _ = api_request("PUT", f"/api/v1/packages/{pkg['name']}/{pkg['vers']}/readme",
                                body=readme.encode(), token=token,
                                content_type="text/plain")
            if rs == 200:
                print(f"    ↳ README uploaded")
            published_readmes.setdefault(pkg["name"], readme)
            time.sleep(6)

    # ── 4. Promote alice to admin ─────────────────────────────────────────────
    print("\nPromoting alice to admin…")
    registry("user", "promote", "alice", check=False)

    # ── 5. Print summary ──────────────────────────────────────────────────────
    try:
        _, stats = api_request("GET", "/api/v1/stats")
        if stats:
            print(f"\nRegistry stats: {stats}")
    except Exception:
        pass

    print("\nDone! Seed data written to:", DATA_DIR)
    print("To browse: cargo run -p freight-registry -- --data", DATA_DIR,
          "serve --base-url", BASE_URL)
    print("\nUser credentials:")
    for u in USERS:
        print(f"  {u['username']} / {u['password']}")

    srv.terminate()


if __name__ == "__main__":
    main()
