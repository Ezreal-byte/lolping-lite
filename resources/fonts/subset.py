"""Rebuild native settings fonts; see README.md for the pinned source files.

python subset.py NotoSansCJKsc-Regular.otf NotoSansCJKsc-Medium.otf
The repository's unmodified Inter 4.1 variable font provides the Latin faces.
"""
import argparse
import hashlib
import pathlib
from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

root = pathlib.Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("regular", type=pathlib.Path)
parser.add_argument("medium", type=pathlib.Path)
args = parser.parse_args()
paths = {
    root / "backend/src/ui/strings.json",
    root / "resources/catalog.json",
    # Runtime errors (for example GNOME extension installation) are shown by
    # the settings UI too; cover the application's own Rust sources, not deps.
    *list((root / "backend/src").rglob("*.rs")),
}
text = "".join(path.read_text(encoding="utf-8") for path in sorted(paths))
text += "".join(chr(cp) for cp in range(32, 256))
text += "←↑→↓↔×−–—…•✓"


def build(source: pathlib.Path, family: str, style: str, extension: str, weight=None):
    font = TTFont(source, recalcTimestamp=False)
    if weight is not None:
        axes = {axis.axisTag: axis.defaultValue for axis in font["fvar"].axes}
        axes["wght"] = weight
        font = instantiateVariableFont(font, axes, inplace=True)
    options = subset.Options()
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14, 16, 17]
    options.name_legacy = True
    options.name_languages = [0x409]
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text=text)
    subsetter.subset(font)
    postscript = family.replace(" ", "") + "-" + style
    names = {
        1: family, 2: style, 3: postscript + "-0.1", 4: family + " " + style,
        6: postscript, 16: family, 17: style,
    }
    for record in font["name"].names:
        if record.nameID in names:
            record.string = names[record.nameID].encode(record.getEncoding())
    if "CFF " in font:
        cff = font["CFF "].cff
        cff.fontNames = [postscript]
        cff.topDictIndex[0].FamilyName = family
        cff.topDictIndex[0].FullName = family + " " + style
    destination = pathlib.Path(__file__).with_name(postscript + extension)
    font.save(destination)
    print(f"{destination.name}: {destination.stat().st_size:,} bytes, "
          f"{len(font.getBestCmap()):,} characters, "
          f"sha256={hashlib.sha256(destination.read_bytes()).hexdigest()}")


build(args.regular, "LOLPing UI", "Regular", ".otf")
build(args.medium, "LOLPing UI", "Medium", ".otf")
inter = root / "backend/resources/InterVariable.ttf"
build(inter, "LOLPing Latin", "Regular", ".ttf", 400)
build(inter, "LOLPing Latin", "Medium", ".ttf", 500)
