# Native settings fonts

The settings process uses embedded Regular and Medium faces so English and
Chinese labels keep the same proportions and weight across Windows, macOS and
Linux. All four files are compact subsets, renamed to avoid using their
upstream reserved names. They are loaded only by the settings process and are
released when it closes. The app's text atlas remains demand-driven.

`LOLPingLatin-Regular.ttf` and `LOLPingLatin-Medium.ttf` are static 400/500 weight
instances of the repository's unmodified Inter 4.1 variable font. Copyright
© 2016 The Inter Project Authors; SIL OFL 1.1, `../../licenses/Inter-OFL.txt`.
See `../../backend/resources/README.md` for the original URL and SHA-256.

`LOLPingUI-Regular.otf` and `LOLPingUI-Medium.otf` are subsets of Noto Sans CJK SC
2.004. Copyright © 2014–2021 Adobe; SIL OFL 1.1, `OFL.txt`. Pinned source files:

| Face | Source | SHA-256 |
| --- | --- | --- |
| Regular | [Sans2.004 Regular](https://github.com/notofonts/noto-cjk/blob/Sans2.004/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf) | `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b` |
| Medium | [Sans2.004 Medium](https://github.com/notofonts/noto-cjk/blob/Sans2.004/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Medium.otf) | `ca094f6b0001fb048ca39ddd797a0cdb0179e1e55c6561e111c49c3e6a61d7b7` |

The subset includes characters found recursively in `backend/src/ui/**/*.rs`,
`backend/src/ui.rs`, `backend/src/ui/strings.json`, `resources/catalog.json`,
Latin-1 and common UI punctuation. A system CJK font supplies additional
characters for user-imported emote names; it does not override bundled labels.

To regenerate, install `fonttools==4.61.1` for Python 3.12, download the two
pinned Noto source files, then run from the repository root:

```text
python resources/fonts/subset.py /path/to/NotoSansCJKsc-Regular.otf /path/to/NotoSansCJKsc-Medium.otf
```

Run this after all new UI strings have been added. The script reads Inter from
`backend/resources/InterVariable.ttf` and writes the four subsets beside itself.
