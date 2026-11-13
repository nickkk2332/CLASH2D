//! Golden images: a picture compared with an image stored beside the
//! tests, as golden frames are for the terminal (12 §12).
//!
//! [`check_golden_image`] compares a [`PixelBuffer`] with a binary PPM
//! file. With `BLESS=1` in the environment it writes the file instead,
//! which is how an image is first made and how a deliberate change is
//! accepted:
//!
//! ```text
//! BLESS=1 cargo test -p clash-front-window
//! ```
//!
//! Look at a blessed image before committing it: any image viewer opens
//! a PPM.

use std::path::Path;

use crate::pixels::{PixelBuffer, rgb_of};

/// Compares a picture with the image stored at `path`, or with
/// `BLESS=1` in the environment writes it there. A difference is
/// returned as a sentence naming the first pixel that differs and how
/// many do.
pub fn check_golden_image(path: &Path, picture: &PixelBuffer) -> Result<(), String> {
    check_or_bless(path, picture, clash_scene::golden::blessing())
}

/// The comparison behind [`check_golden_image`], with the decision to
/// bless given by the caller.
fn check_or_bless(path: &Path, picture: &PixelBuffer, bless: bool) -> Result<(), String> {
    if bless {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
        }
        return std::fs::write(path, picture.to_ppm())
            .map_err(|e| format!("cannot write {}: {e}", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "no golden image at {} ({error}): run the test with BLESS=1 to write it, then look at it",
            path.display()
        )
    })?;
    let stored =
        PixelBuffer::from_ppm(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    if (stored.width(), stored.height()) != (picture.width(), picture.height()) {
        return Err(format!(
            "the picture is {} by {} and {} is {} by {}; run with BLESS=1 to accept the change, and say why in the change's description",
            picture.width(),
            picture.height(),
            path.display(),
            stored.width(),
            stored.height()
        ));
    }
    let width = picture.width().max(1) as usize;
    let mut differing = stored
        .pixels()
        .iter()
        .zip(picture.pixels())
        .enumerate()
        .filter(|(_, (a, b))| a != b);
    let Some((first, (was, now))) = differing.next() else {
        return Ok(());
    };
    Err(format!(
        "the picture differs from {} in {} pixels, the first at ({}, {}): stored {}, now {}; run with BLESS=1 to accept the change, and say why in the change's description",
        path.display(),
        differing.count() + 1,
        first % width,
        first / width,
        rgb_of(*was),
        rgb_of(*now)
    ))
}

/// [`check_golden_image`] for tests: panics with the difference.
///
/// # Panics
///
/// When the picture does not match the stored image, or there is none.
#[track_caller]
pub fn assert_golden_image(path: impl AsRef<Path>, picture: &PixelBuffer) {
    if let Err(problem) = check_golden_image(path.as_ref(), picture) {
        panic!("{problem}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_scene::Rgb;

    #[test]
    fn a_picture_matches_its_image_or_says_where_it_differs() {
        let dir = std::env::temp_dir().join(format!("clash-golden-image-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("one.ppm");
        let mut picture = PixelBuffer::new(4, 2);
        picture.fill(Rgb::new(9, 9, 9));
        assert!(
            check_or_bless(&path, &picture, false)
                .unwrap_err()
                .contains("BLESS=1")
        );
        std::fs::write(&path, picture.to_ppm()).unwrap();
        assert_eq!(check_or_bless(&path, &picture, false), Ok(()));
        picture.pixels_mut()[6] = 0;
        let problem = check_or_bless(&path, &picture, false).unwrap_err();
        assert!(
            problem.contains("in 1 pixels, the first at (2, 1)"),
            "{problem}"
        );
        let problem = check_or_bless(&path, &PixelBuffer::new(1, 1), false).unwrap_err();
        assert!(problem.contains("1 by 1"), "{problem}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
