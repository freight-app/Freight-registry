---
id: intro
title: Freight ecosystem guide
slug: /
---

Freight is the build tool, package format, registry, documentation browser, and editor integration surface for native projects.

## Project setup

A Freight project is described by `freight.toml`. Keep source layout conventional and let Freight generate the build metadata used by compilers, IDEs, and package consumers.

```bash
freight init hello
cd hello
freight build
freight run
```

## Dependencies

Dependencies can come from the registry, local paths, Git repositories, or URL archives. Registry dependencies are cached under `.pkgs/`; compiled artifacts stay under `target/`.

```toml
[dependencies]
fmt = "10"

[dependencies.local-lib]
path = "../local-lib"

[os.windows.dependencies]
winapi = "0.3"
```

## Publish and registry workflow

The registry stores package metadata, sources, uploaded README content, generated documentation indexes, and prebuilts. Use channels for release lanes and ownership for shared package maintenance.

```bash
freight login --registry https://registry.example.com
freight publish --registry https://registry.example.com
freight fetch
freight build --release
```

## Documentation

`freight doc` extracts source comments into a structured symbol index. Published docs appear in the package page through the source documentation viewer, while this guide covers the ecosystem workflow.

```bash
freight doc
freight doc --json
freight publish --registry https://registry.example.com
```

## Editor support

The editor extensions start `freight lsp` for `freight.toml` diagnostics, completion, hover, and source-language passthroughs. VS Code and Neovim wrappers live with the workspace; JetBrains support is planned.
