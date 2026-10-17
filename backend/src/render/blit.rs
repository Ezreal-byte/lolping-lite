//! Axis-aligned, premultiplied RGBA sprite composition. Overlay sprites never
//! rotate, so a fixed-point bilinear blit avoids a general float shader per pixel.
use tiny_skia::Pixmap;
const RB: u32 = 0x00ff00ff;
#[inline(always)]
fn lerp(a: u32, b: u32, t: u32) -> u32 {
    let rb = (((a & RB) * (256 - t) + (b & RB) * t) >> 8) & RB;
    let ga = ((((a >> 8) & RB) * (256 - t) + ((b >> 8) & RB) * t) >> 8) & RB;
    rb | (ga << 8)
}
#[inline(always)]
fn read(p: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(p[i..i + 4].try_into().unwrap())
}
pub fn draw(dst: &mut Pixmap, src: &Pixmap, x: f32, y: f32, w: f32, h: f32, opacity: f32) {
    if w <= 0. || h <= 0. || opacity <= 0. {
        return;
    }
    let (dw, dh) = (dst.width() as usize, dst.height() as usize);
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (left, top) = (x.ceil().max(0.) as usize, y.ceil().max(0.) as usize);
    let (right, bottom) = (
        (x + w).ceil().clamp(0., dw as f32) as usize,
        (y + h).ceil().clamp(0., dh as f32) as usize,
    );
    if right <= left || bottom <= top {
        return;
    }
    let a = (opacity.clamp(0., 1.) * 256.).round() as u32;
    let xs: Vec<_> = (left..right)
        .map(|dx| {
            let sx = ((dx as f32 + 0.5 - x) * sw as f32 / w - 0.5).clamp(0., (sw - 1) as f32);
            let first = sx as usize;
            (
                first * 4,
                (first + 1).min(sw - 1) * 4,
                ((sx - first as f32) * 256.) as u32,
            )
        })
        .collect();
    let pixels = src.data();
    let output = dst.data_mut();
    for dy in top..bottom {
        let sy = ((dy as f32 + 0.5 - y) * sh as f32 / h - 0.5).clamp(0., (sh - 1) as f32);
        let first = sy as usize;
        let r0 = first * sw * 4;
        let r1 = (first + 1).min(sh - 1) * sw * 4;
        let fy = ((sy - first as f32) * 256.) as u32;
        for (out, &(x0, x1, fx)) in output[(dy * dw + left) * 4..(dy * dw + right) * 4]
            .chunks_exact_mut(4)
            .zip(&xs)
        {
            let c = lerp(
                lerp(read(pixels, r0 + x0), read(pixels, r0 + x1), fx),
                lerp(read(pixels, r1 + x0), read(pixels, r1 + x1), fx),
                fy,
            );
            let c = if a == 256 { c } else { lerp(0, c, a) };
            let alpha = c >> 24;
            if alpha == 0 {
                continue;
            }
            let blended = if alpha == 255 {
                c
            } else {
                let old = u32::from_le_bytes(out.try_into().unwrap());
                let inv = 256 - alpha;
                let rb = (c & RB) + (((old & RB) * inv) >> 8 & RB);
                let ga = ((c >> 8) & RB) + ((((old >> 8) & RB) * inv) >> 8 & RB);
                rb | (ga << 8)
            };
            out.copy_from_slice(&blended.to_le_bytes());
        }
    }
}
pub fn copy(dst: &mut Pixmap, src: &Pixmap, x: i32, y: i32, opacity: f32) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    let (left, top) = (x.max(0), y.max(0));
    let (right, bottom) = ((x + sw).min(dw), (y + sh).min(dh));
    if right <= left || bottom <= top {
        return;
    }
    let a = (opacity.clamp(0., 1.) * 256.).round() as u32;
    let source = src.data();
    let out = dst.data_mut();
    for dy in top..bottom {
        let si = ((dy - y) * sw + left - x) as usize * 4;
        let di = (dy * dw + left) as usize * 4;
        let len = (right - left) as usize * 4;
        for (input, output) in source[si..si + len]
            .chunks_exact(4)
            .zip(out[di..di + len].chunks_exact_mut(4))
        {
            let c = u32::from_le_bytes(input.try_into().unwrap());
            let c = if a == 256 { c } else { lerp(0, c, a) };
            let alpha = c >> 24;
            if alpha == 0 {
                continue;
            }
            let c = if alpha == 255 {
                c
            } else {
                let old = u32::from_le_bytes(output.try_into().unwrap());
                let inv = 256 - alpha;
                let rb = (c & RB) + (((old & RB) * inv) >> 8 & RB);
                let ga = ((c >> 8) & RB) + ((((old >> 8) & RB) * inv) >> 8 & RB);
                rb | (ga << 8)
            };
            output.copy_from_slice(&c.to_le_bytes());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alpha_blending_and_clipping_keep_channels_premultiplied() {
        let mut dst = Pixmap::new(4, 4).unwrap();
        dst.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
        let mut src = Pixmap::new(2, 2).unwrap();
        src.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 128));
        draw(&mut dst, &src, -2., -2., 4., 4., 1.);
        let p = dst.pixel(0, 0).unwrap();
        assert_eq!((p.red(), p.blue(), p.alpha()), (128, 127, 255));
        assert_eq!(dst.pixel(3, 3).unwrap().blue(), 255);
        draw(&mut dst, &src, 100., 0., 2., 2., 1.);
        for p in dst.pixels() {
            assert!(p.red() <= p.alpha() && p.green() <= p.alpha() && p.blue() <= p.alpha());
        }
    }
}
