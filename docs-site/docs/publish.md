---
id: publish
title: Publish packages
---

Publishing sends the package manifest, source archive, README, and optional generated docs or prebuilts to the registry.

```bash
freight login --registry https://registry.example.com
freight publish --registry https://registry.example.com
```

Use channels for release lanes:

```bash
freight publish --registry https://registry.example.com --channel beta
```

Use owner management in the registry UI or API for packages maintained by more than one user or organization.
