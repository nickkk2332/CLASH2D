//! [`PixelBuffer`]: what the renderer draws into, and how a buffer is
//! written to and read from an image file.
//!
//! A pixel is a `u32` holding `0x00RRGGBB`, the form `softbuffer` shows,
//! so a drawn buffer goes to the window without conversion.
//!
//! # Images
//!
//! A buffer is written as a binary PPM (`P6`): a short text header,
//! `P6`, the width and the height, and `255`, then three bytes of red,
//! green and blue for each pixel, row by row from the top. Every image
//! viewer reads it, and writing it needs nothing but the standard
//! library. The golden images of the tests are kept in this form.

use clash_scene::Rgb;

/// The magic number of a binary PPM.
const PPM_MAGIC: &str = "P6";

/// The largest value of a channel in the images this crate writes.
const PPM_MAX: u32 = 255;

/// The bytes of one pixel in a binary PPM.
const PPM_BYTES_PER_PIXEL: usize = 3;

/// A colour as a pixel: `0x00RRGGBB`.
///
/// ```
/// use clash_front_window::pixel;
/// use clash_scene::Rgb;
/// assert_eq!(pixel(Rgb::new(0x12, 0x34, 0x56)), 0x0012_3456);
/// ```
pub const fn pixel(rgb: Rgb) -> u32 {
    ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32
}

/// A pixel as a colour.
pub const fn rgb_of(pixel: u32) -> Rgb {
    Rgb::new((pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8)
}

/// A rectangle of pixels, each `0x00RRGGBB`, row by row from the top.
///
/// Resizing keeps the storage where it can, so a buffer drawn into
/// every frame allocates only when the window grows.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PixelBuffer {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl PixelBuffer {
    /// A black buffer.
    pub fn new(width: u32, height: u32) -> PixelBuffer {
        let mut buffer = PixelBuffer::default();
        buffer.resize(width, height);
        buffer
    }

    /// Changes the size, keeping the storage where it can. What the
    /// pixels hold afterwards is unspecified: draw over all of them.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.pixels.resize(width as usize * height as usize, 0);
    }

    /// The width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Every pixel, row by row from the top.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Every pixel, to change them.
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
    }

    /// The colour of one pixel, if it is in the buffer.
    pub fn get(&self, x: u32, y: u32) -> Option<Rgb> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(rgb_of(
            self.pixels[y as usize * self.width as usize + x as usize],
        ))
    }

    /// Makes every pixel one colour.
    pub fn fill(&mut self, rgb: Rgb) {
        self.pixels.fill(pixel(rgb));
    }

    /// The buffer as a binary PPM image.
    pub fn to_ppm(&self) -> Vec<u8> {
        let header = format!("{PPM_MAGIC}\n{} {}\n{PPM_MAX}\n", self.width, self.height);
        let mut out = Vec::with_capacity(header.len() + self.pixels.len() * PPM_BYTES_PER_PIXEL);
        out.extend_from_slice(header.as_bytes());
        for &p in &self.pixels {
            let rgb = rgb_of(p);
            out.extend_from_slice(&[rgb.r, rgb.g, rgb.b]);
        }
        out
    }

    /// Reads a binary PPM image with channels of one byte, as
    /// [`PixelBuffer::to_ppm`] writes it. Comments in the header are
    /// allowed. A mistake is returned as a sentence.
    pub fn from_ppm(bytes: &[u8]) -> Result<PixelBuffer, String> {
        let mut at = 0;
        let mut fields = [0_u32; 3];
        let magic = next_token(bytes, &mut at).ok_or("the image is empty")?;
        if magic != PPM_MAGIC.as_bytes() {
            return Err("the image is not a binary PPM: it does not start with `P6`".to_owned());
        }
        for (field, name) in fields.iter_mut().zip(["width", "height", "maximum"]) {
            let token = next_token(bytes, &mut at)
                .ok_or_else(|| format!("the image's header ends before its {name}"))?;
            *field = core::str::from_utf8(token)
                .ok()
                .and_then(|text| text.parse().ok())
                .ok_or_else(|| format!("the image's {name} is not a whole number"))?;
        }
        let [width, height, max] = fields;
        if max != PPM_MAX {
            return Err(format!(
                "the image's channels go up to {max}; only {PPM_MAX} is read"
            ));
        }
        // One whitespace byte ends the header.
        at += 1;
        let count = width as usize * height as usize;
        let data = bytes
            .get(at..)
            .filter(|data| data.len() == count * PPM_BYTES_PER_PIXEL)
            .ok_or_else(|| {
                format!(
                    "the image should hold {} bytes of pixels for {width} by {height}",
                    count * PPM_BYTES_PER_PIXEL
                )
            })?;
        let pixels = data
            .chunks_exact(PPM_BYTES_PER_PIXEL)
            .map(|rgb| pixel(Rgb::new(rgb[0], rgb[1], rgb[2])))
            .collect();
        Ok(PixelBuffer {
            width,
            height,
            pixels,
        })
    }
}

/// The next word of a PPM header, skipping whitespace and comments.
fn next_token<'a>(bytes: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    loop {
        match bytes.get(*at)? {
            b'#' => {
                while bytes.get(*at).is_some_and(|&b| b != b'\n') {
                    *at += 1;
                }
            }
            b if b.is_ascii_whitespace() => *at += 1,
            _ => break,
        }
    }
    let start = *at;
    while bytes.get(*at).is_some_and(|b| !b.is_ascii_whitespace()) {
        *at += 1;
    }
    Some(&bytes[start..*at])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_is_written_and_read_back() {
        let mut buffer = PixelBuffer::new(3, 2);
        buffer.fill(Rgb::new(1, 2, 3));
        buffer.pixels_mut()[4] = pixel(Rgb::new(250, 128, 7));
        let ppm = buffer.to_ppm();
        assert!(ppm.starts_with(b"P6\n3 2\n255\n"));
        assert_eq!(ppm.len(), 11 + 3 * 2 * 3);
        let back = PixelBuffer::from_ppm(&ppm).unwrap();
        assert_eq!(back, buffer);
        assert_eq!(back.get(1, 1), Some(Rgb::new(250, 128, 7)));
        assert_eq!(back.get(3, 0), None);
    }

    #[test]
    fn a_header_may_hold_comments_and_mistakes_are_said() {
        let image = b"P6 # made by hand\n1 1\n255\n\x01\x02\x03";
        let buffer = PixelBuffer::from_ppm(image).unwrap();
        assert_eq!(buffer.get(0, 0), Some(Rgb::new(1, 2, 3)));
        assert!(PixelBuffer::from_ppm(b"").is_err());
        assert!(
            PixelBuffer::from_ppm(b"P3\n1 1\n255\n1 2 3")
                .unwrap_err()
                .contains("P6")
        );
        assert!(
            PixelBuffer::from_ppm(b"P6\n1 1\n65535\n\0\0\0\0\0\0")
                .unwrap_err()
                .contains("65535")
        );
        assert!(
            PixelBuffer::from_ppm(b"P6\n2 1\n255\n\0\0\0")
                .unwrap_err()
                .contains("6 bytes")
        );
        assert!(PixelBuffer::from_ppm(b"P6\nwide 1\n255\n").is_err());
    }

    #[test]
    fn resizing_keeps_the_storage() {
        let mut buffer = PixelBuffer::new(10, 10);
        let capacity = buffer.pixels.capacity();
        buffer.resize(5, 4);
        assert_eq!(buffer.pixels().len(), 20);
        assert_eq!(buffer.pixels.capacity(), capacity);
        assert_eq!((buffer.width(), buffer.height()), (5, 4));
    }
}
