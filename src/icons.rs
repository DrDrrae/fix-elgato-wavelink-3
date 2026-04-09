use image::{Rgb, RgbImage};

fn draw_thick_line(
    img: &mut RgbImage,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: Rgb<u8>,
    width: i32,
) {
    let w = img.width() as i32;
    let h = img.height() as i32;
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = if x0 < x1 { 1i32 } else { -1i32 };
    let sy = if y0 < y1 { 1i32 } else { -1i32 };
    let mut err = dx - dy;
    let mut x = x0;
    let mut y = y0;
    let r = width / 2;

    loop {
        for oy in -r..=r {
            for ox in -r..=r {
                if ox * ox + oy * oy <= r * r + r {
                    let px = x + ox;
                    let py = y + oy;
                    if px >= 0 && px < w && py >= 0 && py < h {
                        img.put_pixel(px as u32, py as u32, color);
                    }
                }
            }
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }
}

fn draw_z(img: &mut RgbImage, left: i32, top: i32, width: i32, height: i32, line_width: i32) {
    let black = Rgb([0u8, 0, 0]);
    draw_thick_line(img, left, top, left + width, top, black, line_width);
    draw_thick_line(img, left + width, top, left, top + height, black, line_width);
    draw_thick_line(
        img,
        left,
        top + height,
        left + width,
        top + height,
        black,
        line_width,
    );
}

/// Returns (enabled_rgba, disabled_rgba) as 64×64 RGBA byte vectors.
pub fn create_icons() -> (Vec<u8>, Vec<u8>) {
    let mut enabled_img = RgbImage::from_pixel(64, 64, Rgb([255u8, 255, 255]));

    // Large Z: top-left (15,11), size 40×45, line width 3
    draw_z(&mut enabled_img, 15, 11, 40, 45, 3);
    // Medium Z: top-left (5,20), size 16×20, line width 2
    draw_z(&mut enabled_img, 5, 20, 16, 20, 2);
    // Small Z: top-left (45,30), size 12×14, line width 2
    draw_z(&mut enabled_img, 45, 30, 12, 14, 2);

    let mut disabled_img = enabled_img.clone();
    let red = Rgb([255u8, 0, 0]);
    draw_thick_line(&mut disabled_img, 5, 5, 59, 59, red, 3);
    draw_thick_line(&mut disabled_img, 5, 59, 59, 5, red, 3);

    let to_rgba = |img: RgbImage| -> Vec<u8> {
        let mut rgba = Vec::with_capacity(64 * 64 * 4);
        for pixel in img.pixels() {
            rgba.push(pixel[0]);
            rgba.push(pixel[1]);
            rgba.push(pixel[2]);
            rgba.push(255u8);
        }
        rgba
    };

    (to_rgba(enabled_img), to_rgba(disabled_img))
}
