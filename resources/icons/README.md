# Native settings icons

These 15 white transparent PNGs use Microsoft's Fluent System Icons, the same
icon set as the upstream lolPing settings. The first six use the exact upstream
navigation symbols. See `sources.json` for the icon-to-symbol mapping.

Source package: `@fluentui/react-icons` **2.0.343** from the official npm registry.
The source SVG paths are unchanged; `generate.mjs` extracts them without running
package code and rasterizes at 96 × 96 for crisp 20–24 point icons up to 200%
scale. Rust tints the PNGs at paint time. No React, JavaScript or SVG renderer is
needed by the application, and only icons shown on screen acquire textures.

- Repository: https://github.com/microsoft/fluentui-system-icons
- npm source revision: `e17e4b6a88cb4f5daf6331c2a7bb7cc7217f9b11`
- Package tarball: https://registry.npmjs.org/@fluentui/react-icons/-/react-icons-2.0.343.tgz
- Tarball SHA-256: `fc60ab88ce7ce9c9a8a5396a44ea15d636d1c40fb1d7eca142c1f3b58e2b23bc`
- License: MIT, copyright © 2020 Microsoft Corporation. The complete notice is
  in `../../licenses/Fluent-System-Icons-MIT.txt`.

Development-only regeneration requires Node.js and Sharp. Extract the pinned
npm tarball, then run:

```text
node resources/icons/generate.mjs /path/to/unpacked/package /path/to/node_modules/sharp
```
