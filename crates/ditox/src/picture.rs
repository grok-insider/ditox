//! Pictures for the preview, decoded and made small off the window's thread.
//!
//! A screenshot is drawn a few hundred points wide. Kept at its own size it
//! would cost four bytes for every pixel of a 4K screen, once in memory and
//! once on the GPU, for each clip looked at.

use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

/// The longest side a picture is kept at, in pixels: enough for the preview
/// at the window's scale. The view sets it.
static LONG_SIDE: AtomicU32 = AtomicU32::new(LONG_SIDE_1X);
const LONG_SIDE_1X: u32 = 1280;
const LONG_SIDE_2X: u32 = 2048;
/// What decoding one picture may allocate.
const DECODE_BUDGET: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha sRGB, four bytes a pixel.
    pub rgba: Vec<u8>,
}

thread_local! {
    /// The window's thread keeps its own count of pixels to a point, so that
    /// what it asks for does not depend on another window's scale.
    static DENSITY: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
}

pub fn set_scale(scale: f32) {
    let dense = scale > 1.25;
    DENSITY.with(|density| density.set(if dense { 2 } else { 1 }));
    LONG_SIDE.store(
        if dense { LONG_SIDE_2X } else { LONG_SIDE_1X },
        Ordering::Relaxed,
    );
}

pub fn long_side() -> u32 {
    LONG_SIDE.load(Ordering::Relaxed)
}

/// The image in `bytes`, no larger than `long_side` on its longest side.
pub fn fit(bytes: &[u8], long_side: u32) -> Result<Picture, String> {
    fit_within(bytes, long_side, long_side)
}

/// The image in `bytes`, made smaller until it is at most `width` by `height`.
pub fn fit_within(bytes: &[u8], width: u32, height: u32) -> Result<Picture, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|err| err.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(DECODE_BUDGET);
    reader.limits(limits);
    let image = reader.decode().map_err(|err| err.to_string())?;
    let (width, height) = (width.max(1), height.max(1));
    let image = if image.width() > width || image.height() > height {
        image.thumbnail(width, height)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    Ok(Picture {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// How many device pixels a point of the window takes, as far as pictures
/// care. Asked on the window's thread, where `set_scale` is called.
pub fn density() -> u32 {
    DENSITY.with(|density| density.get())
}

/// A picture for a row of the list: every one asked for is made, in order.
pub struct ThumbJob {
    pub id: i64,
    pub bytes: Vec<u8>,
    pub fit: (u32, u32),
}

/// A thread that makes the small pictures of the list's rows.
pub fn thumb_worker(
    done: impl Fn(i64, Result<Picture, String>) + Send + 'static,
) -> Sender<ThumbJob> {
    let (jobs, queue) = mpsc::channel::<ThumbJob>();
    let _ = std::thread::Builder::new()
        .name("ditox-thumbs".into())
        .spawn(move || {
            while let Ok(job) = queue.recv() {
                let picture = fit_within(&job.bytes, job.fit.0, job.fit.1);
                drop(job.bytes);
                done(job.id, picture);
            }
        });
    jobs
}

pub struct Job {
    pub id: i64,
    pub bytes: Vec<u8>,
}

/// A thread that fits pictures and hands each to `done`. Only the newest job
/// waiting is worked on: the ones before it are for clips already passed.
pub fn worker(done: impl Fn(i64, Result<Picture, String>) + Send + 'static) -> Sender<Job> {
    let (jobs, queue) = mpsc::channel::<Job>();
    let _ = std::thread::Builder::new()
        .name("ditox-pictures".into())
        .spawn(move || {
            while let Ok(job) = queue.recv() {
                let job = newest(job, &queue);
                let picture = fit(&job.bytes, long_side());
                drop(job.bytes);
                done(job.id, picture);
            }
        });
    jobs
}

/// `job`, or the last of those that arrived after it.
fn newest(mut job: Job, queue: &Receiver<Job>) -> Job {
    while let Ok(newer) = queue.try_recv() {
        job = newer;
    }
    job
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A PNG of `width` by `height`, opaque, with a gradient so that scaling shows.
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 96, 255])
        });
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn a_large_picture_is_kept_at_the_preview_size_and_a_small_one_as_it_is() {
        let large = fit(&png(2560, 1440), 1280).unwrap();
        assert_eq!((large.width, large.height), (1280, 720));
        assert_eq!(large.rgba.len(), 1280 * 720 * 4);

        let tall = fit(&png(300, 1800), 1280).unwrap();
        assert_eq!((tall.width, tall.height), (213, 1280));

        let small = fit(&png(64, 48), 1280).unwrap();
        assert_eq!((small.width, small.height), (64, 48));
        assert_eq!(&small.rgba[..4], &[0, 0, 96, 255]);
    }

    #[test]
    fn bytes_that_are_not_a_picture_are_an_error() {
        assert!(fit(b"not an image", 1280).is_err());
        let mut cut = png(64, 48);
        cut.truncate(40);
        assert!(fit(&cut, 1280).is_err());
    }

    #[test]
    fn the_worker_fits_the_newest_job_and_reports_a_bad_one() {
        let (results, seen) = mpsc::channel();
        let jobs = worker(move |id, picture| {
            let _ = results.send((id, picture.map(|picture| (picture.width, picture.height))));
        });
        jobs.send(Job {
            id: 7,
            bytes: png(400, 200),
        })
        .unwrap();
        assert_eq!(seen.recv().unwrap(), (7, Ok((400, 200))));
        jobs.send(Job {
            id: 8,
            bytes: b"broken".to_vec(),
        })
        .unwrap();
        let (id, picture) = seen.recv().unwrap();
        assert_eq!(id, 8);
        assert!(picture.is_err());
    }

    #[test]
    fn of_the_jobs_waiting_only_the_newest_is_worked_on() {
        let (jobs, queue) = mpsc::channel();
        for id in 2..=4 {
            jobs.send(Job {
                id,
                bytes: Vec::new(),
            })
            .unwrap();
        }
        let first = Job {
            id: 1,
            bytes: Vec::new(),
        };
        assert_eq!(newest(first, &queue).id, 4);
        assert!(queue.try_recv().is_err(), "the ones before it are dropped");
        let alone = Job {
            id: 9,
            bytes: Vec::new(),
        };
        assert_eq!(newest(alone, &queue).id, 9);
    }

    #[test]
    fn the_long_side_follows_the_window_scale() {
        set_scale(2.0);
        assert_eq!(long_side(), LONG_SIDE_2X);
        set_scale(1.0);
        assert_eq!(long_side(), LONG_SIDE_1X);
    }
}
