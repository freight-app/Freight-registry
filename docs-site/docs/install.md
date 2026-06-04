---
id: install
title: Install
---

Install the CLI first, then point it at the registry used by your project or organization.

## Linux

```bash
curl https://sh.rustup.rs -sSf | sh
cargo install --git https://github.com/freight-app/freight freight
```

Install a C/C++ compiler such as GCC or Clang. Install CMake, pkg-config, and Ninja when packages need foreign builds.

## macOS

```bash
xcode-select --install
curl https://sh.rustup.rs -sSf | sh
cargo install --git https://github.com/freight-app/freight freight
```

Use Apple Clang from Command Line Tools for native builds. Install CMake and Ninja through Homebrew when needed.

## Windows

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools
cargo install --git https://github.com/freight-app/freight freight
```

Enable the MSVC C++ toolchain in Visual Studio Build Tools. Use PowerShell, Windows Terminal, or a developer command prompt.

## From source

```bash
git clone https://github.com/freight-app/freight
cd freight
cargo install --path crates/freight
```

Run `freight --version` after install.

## Registry server

```bash
cargo install --git https://github.com/freight-app/freight-registry freight-registry
freight-registry --data /var/lib/freight-registry serve --base-url https://registry.example.com
```

Authenticate once per registry. The token is stored by the CLI and is not displayed again in the account settings page.

```bash
freight login --registry https://registry.example.com
freight fetch
freight build
```
