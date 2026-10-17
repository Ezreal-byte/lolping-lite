"""Rebuild the OFL font subset: python subset.py /path/to/NotoSansCJKsc-Regular.otf."""
import pathlib
import sys
from fontTools import subset
from fontTools.ttLib import TTFont

root = pathlib.Path(__file__).resolve().parents[2]
text = "".join(
    path.read_text(encoding="utf-8")
    for path in [
        root / "backend/src/ui/strings.json",
        root / "backend/src/ui.rs",
        root / "backend/src/ui/keys.rs",
        root / "resources/catalog.json",
    ]
)
text += "".join(chr(cp) for cp in range(32, 127))
options = subset.Options()
options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
options.name_legacy = True
options.name_languages = [0x409]
font = TTFont(sys.argv[1])
subsetter = subset.Subsetter(options=options)
subsetter.populate(text=text)
subsetter.subset(font)
names = {1: "LOLPing UI", 2: "Regular", 3: "LOLPingUI-Regular-0.1", 4: "LOLPing UI Regular", 6: "LOLPingUI-Regular"}
for record in font["name"].names:
    if record.nameID in names:
        record.string = names[record.nameID].encode(record.getEncoding())
if "CFF " in font:
    cff = font["CFF "].cff
    cff.fontNames = ["LOLPingUI-Regular"]
    cff.topDictIndex[0].FamilyName = "LOLPing UI"
    cff.topDictIndex[0].FullName = "LOLPing UI Regular"
font.save(pathlib.Path(__file__).with_name("LOLPingUI-Regular.otf"))
