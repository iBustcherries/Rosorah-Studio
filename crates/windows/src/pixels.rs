//! Converts padded DXGI BGRA textures to the engine's bounded RGBA frames.
use anyhow::{Result, ensure};
use rosorah_capture::Frame;

#[derive(Clone, Copy, Debug)]
pub enum Rotation {
    Identity,
    Clockwise90,
    Clockwise180,
    Clockwise270,
}

pub fn convert(
    bytes: &[u8],
    width: u32,
    height: u32,
    pitch: usize,
    rotation: Rotation,
    resolution: u32,
) -> Result<Frame> {
    ensure!(
        width > 0 && height > 0 && width <= 16384 && height <= 16384,
        "Invalid desktop dimensions"
    );
    ensure!(pitch >= width as usize * 4, "Invalid desktop row pitch");
    let required = (height as usize - 1)
        .checked_mul(pitch)
        .and_then(|v| v.checked_add(width as usize * 4))
        .ok_or_else(|| anyhow::anyhow!("Desktop buffer overflow"))?;
    ensure!(bytes.len() >= required, "Truncated desktop buffer");
    ensure!(resolution <= 4, "Invalid capture resolution");
    let (rw, rh) = match rotation {
        Rotation::Clockwise90 | Rotation::Clockwise270 => (height, width),
        _ => (width, height),
    };
    let max_height = [720, 1080, 1440, 2160, 4096][resolution as usize];
    let scale = (max_height as f64 / rh as f64)
        .min(4096.0 / rw as f64)
        .min(1.0);
    let ow = ((rw as f64 * scale).round() as u32).max(1);
    let oh = ((rh as f64 * scale).round() as u32).max(1);
    let mut rgba = vec![0; ow as usize * oh as usize * 4];
    if matches!(rotation, Rotation::Identity) && ow == width && oh == height {
        // The common unscaled path is a contiguous swizzle. Keep division,
        // rotation branches and coordinate calculations outside the pixel loop.
        for (y, row) in rgba.chunks_exact_mut(ow as usize * 4).enumerate() {
            let input = &bytes[y * pitch..y * pitch + row.len()];
            for (dst, src) in row.chunks_exact_mut(4).zip(input.chunks_exact(4)) {
                dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
            }
        }
    } else {
        // Scaling/rotation still uses the exact original nearest-neighbour
        // mapping, but each horizontal coordinate is divided only once.
        let xs: Vec<usize> = (0..ow)
            .map(|x| (x as u64 * rw as u64 / ow as u64) as usize)
            .collect();
        for (y, row) in rgba.chunks_exact_mut(ow as usize * 4).enumerate() {
            let ry = (y as u64 * rh as u64 / oh as u64) as usize;
            let (start, step, reverse) = match rotation {
                Rotation::Identity => (ry * pitch, 4, false),
                Rotation::Clockwise90 => ((height as usize - 1) * pitch + ry * 4, pitch, true),
                Rotation::Clockwise180 => (
                    (height as usize - 1 - ry) * pitch + (width as usize - 1) * 4,
                    4,
                    true,
                ),
                Rotation::Clockwise270 => ((width as usize - 1 - ry) * 4, pitch, false),
            };
            for (dst, &x) in row.chunks_exact_mut(4).zip(&xs) {
                let offset = x * step;
                let from = if reverse {
                    start - offset
                } else {
                    start + offset
                };
                dst.copy_from_slice(&[bytes[from + 2], bytes[from + 1], bytes[from], 255]);
            }
        }
    }
    Ok(Frame {
        width: ow,
        height: oh,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> Vec<u8> {
        vec![
            1, 0, 10, 0, 2, 0, 20, 0, 99, 99, 99, 99, 3, 0, 30, 0, 4, 0, 40, 0, 99, 99, 99, 99, 5,
            0, 50, 0, 6, 0, 60, 0,
        ]
    }
    fn red(r: Rotation) -> Vec<u8> {
        convert(&image(), 2, 3, 12, r, 4)
            .unwrap()
            .rgba
            .chunks_exact(4)
            .map(|p| p[0])
            .collect()
    }
    #[test]
    fn padded_rows_and_opaque_alpha() {
        let f = convert(&image(), 2, 3, 12, Rotation::Identity, 4).unwrap();
        assert_eq!(red(Rotation::Identity), vec![10, 20, 30, 40, 50, 60]);
        assert!(f.rgba.chunks_exact(4).all(|p| p[3] == 255));
        assert_eq!(&f.rgba[..4], &[10, 0, 1, 255]);
    }
    #[test]
    fn portrait_and_flipped_monitors() {
        assert_eq!(red(Rotation::Clockwise90), vec![50, 30, 10, 60, 40, 20]);
        assert_eq!(red(Rotation::Clockwise180), vec![60, 50, 40, 30, 20, 10]);
        assert_eq!(red(Rotation::Clockwise270), vec![20, 40, 60, 10, 30, 50]);
    }
    #[test]
    fn optimized_conversion_matches_reference() {
        for (w, h) in [(7, 5), (5, 7), (1921, 1081), (1081, 1921)] {
            let pitch = w as usize * 4 + 12;
            let bytes: Vec<u8> = (0..pitch * h as usize).map(|n| (n * 37) as u8).collect();
            for rotation in [
                Rotation::Identity,
                Rotation::Clockwise90,
                Rotation::Clockwise180,
                Rotation::Clockwise270,
            ] {
                for resolution in [0, 4] {
                    let f = convert(&bytes, w, h, pitch, rotation, resolution).unwrap();
                    let (rw, rh) = match rotation {
                        Rotation::Clockwise90 | Rotation::Clockwise270 => (h, w),
                        _ => (w, h),
                    };
                    for y in 0..f.height {
                        for x in 0..f.width {
                            let rx = (x as u64 * rw as u64 / f.width as u64) as u32;
                            let ry = (y as u64 * rh as u64 / f.height as u64) as u32;
                            let (sx, sy) = match rotation {
                                Rotation::Identity => (rx, ry),
                                Rotation::Clockwise90 => (ry, h - 1 - rx),
                                Rotation::Clockwise180 => (w - 1 - rx, h - 1 - ry),
                                Rotation::Clockwise270 => (w - 1 - ry, rx),
                            };
                            let from = sy as usize * pitch + sx as usize * 4;
                            let to = (y as usize * f.width as usize + x as usize) * 4;
                            assert_eq!(
                                &f.rgba[to..to + 4],
                                &[bytes[from + 2], bytes[from + 1], bytes[from], 255]
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn malformed_buffers_rejected() {
        assert!(convert(&[], 1, 1, 4, Rotation::Identity, 4).is_err());
        assert!(convert(&image(), 2, 3, 4, Rotation::Identity, 4).is_err());
        assert!(convert(&image(), 0, 3, 12, Rotation::Identity, 4).is_err());
        assert!(convert(&image(), 2, 3, 12, Rotation::Identity, 5).is_err());
    }
    #[test]
    fn original_resolution_stays_within_engine_limits() {
        let f = convert(&vec![0; 5000 * 4], 5000, 1, 20000, Rotation::Identity, 4).unwrap();
        assert_eq!((f.width, f.height), (4096, 1));
    }
    #[test]
    fn selected_resolution_preserves_aspect_ratio() {
        let f = convert(
            &vec![0; 1920 * 1080 * 4],
            1920,
            1080,
            7680,
            Rotation::Identity,
            0,
        )
        .unwrap();
        assert_eq!((f.width, f.height), (1280, 720));
    }
}
