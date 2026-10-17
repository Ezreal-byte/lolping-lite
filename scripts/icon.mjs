import { unzlibSync } from 'fflate';

// Convert the checked-in RGBA PNG to a conventional 32-bit ICO. NSIS expects
// bitmap icon entries; no image editor or build-time Python dependency is needed.
export function windowsIcon(png) {
  const w = png.readUInt32BE(16), h = png.readUInt32BE(20);
  if (png[24] !== 8 || png[25] !== 6 || png[28] !== 0) throw Error('Expected non-interlaced RGBA8 icon PNG');
  const parts = [];
  for (let at = 8; at < png.length;) {
    const len = png.readUInt32BE(at), type = png.toString('ascii', at + 4, at + 8);
    if (type === 'IDAT') parts.push(png.subarray(at + 8, at + 8 + len));
    at += len + 12;
  }
  const raw = unzlibSync(Buffer.concat(parts)), pixels = Buffer.alloc(w * h * 4), stride = w * 4;
  const paeth = (a, b, c) => { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); return pa <= pb && pa <= pc ? a : pb <= pc ? b : c; };
  for (let y = 0; y < h; y++) {
    const filter = raw[y * (stride + 1)];
    if (filter > 4) throw Error('Invalid PNG filter');
    for (let x = 0; x < stride; x++) {
      const at = y * stride + x, a = x >= 4 ? pixels[at - 4] : 0, b = y ? pixels[at - stride] : 0, c = y && x >= 4 ? pixels[at - stride - 4] : 0;
      pixels[at] = raw[y * (stride + 1) + 1 + x] + [0, a, b, Math.floor((a + b) / 2), paeth(a, b, c)][filter];
    }
  }
  const sizes = [16, 32, 48, 128], header = Buffer.alloc(6 + sizes.length * 16), images = [];
  header.writeUInt16LE(1, 2); header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((size, i) => {
    const maskStride = Math.ceil(size / 32) * 4;
    const image = Buffer.alloc(40 + size * size * 4 + maskStride * size);
    image.writeUInt32LE(40, 0); image.writeInt32LE(size, 4); image.writeInt32LE(size * 2, 8); image.writeUInt16LE(1, 12); image.writeUInt16LE(32, 14);
    image.writeUInt32LE(size * size * 4, 20);
    for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
      const src = (Math.floor(y * h / size) * w + Math.floor(x * w / size)) * 4, dst = 40 + ((size - 1 - y) * size + x) * 4;
      image.set([pixels[src + 2], pixels[src + 1], pixels[src], pixels[src + 3]], dst);
      if (!pixels[src + 3]) image[40 + size * size * 4 + (size - 1 - y) * maskStride + Math.floor(x / 8)] |= 0x80 >> (x % 8);
    }
    const at = 6 + i * 16;
    header[at] = size; header[at + 1] = size; header.writeUInt16LE(1, at + 4); header.writeUInt16LE(32, at + 6);
    header.writeUInt32LE(image.length, at + 8); header.writeUInt32LE(offset, at + 12);
    offset += image.length; images.push(image);
  });
  return Buffer.concat([header, ...images]);
}
