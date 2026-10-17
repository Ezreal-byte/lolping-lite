# Chinese UI fallback

`LOLPingUI-Regular.otf` is a renamed subset of Noto Sans CJK SC Regular,
distributed under the SIL Open Font License 1.1 in `OFL.txt`.

Original font copyright: © 2014–2021 Adobe (http://www.adobe.com/).

Source: https://github.com/notofonts/noto-cjk/blob/main/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf

The subset contains the characters used in the native settings translations,
built-in emote catalog and common ASCII. System Chinese fonts are preferred
when present so custom emote names can use a larger character set. The font
is embedded only in the settings process and is released when settings close.

Regeneration requires Python and fonttools. `subset.py` accepts the original
Noto font path and writes the renamed subset beside this file. Run it after
adding translated UI strings or catalog names.
