use super::*;

/// Half a second of red then half a second of blue, 50 by 30 pixels at
/// 12 frames a second with a keyframe every 4, and a mono Vorbis tone.
/// Made with FFmpeg's `color` and `sine` sources. The encoder left out
/// the frames that repeat, keeping only keyframes 0, 4, 6 and 10.
const TEST_VIDEO: &[u8] = include_bytes!("test.ogv");

fn open() -> VideoStream {
    VideoStream::from_bytes(TEST_VIDEO).unwrap()
}

/// The average color of the picture.
fn color(stream: &VideoStream) -> [u8; 3] {
    let pixels = stream.pixels();
    let count = pixels.len() / 4;
    let mut sum = [0usize; 3];
    for pixel in pixels.as_chunks::<4>().0.iter() {
        for channel in 0..3 {
            sum[channel] += usize::from(pixel[channel]);
        }
    }
    sum.map(|total| (total / count) as u8)
}

fn is_red(rgb: [u8; 3]) -> bool {
    rgb[0] > 230 && rgb[1] < 25 && rgb[2] < 25
}

fn is_blue(rgb: [u8; 3]) -> bool {
    rgb[0] < 25 && rgb[1] < 25 && rgb[2] > 230
}

#[test]
fn the_headers_give_the_size_rate_and_length() {
    let stream = open();
    assert_eq!(stream.dimensions(), (50, 30));
    assert_eq!(stream.frame_rate(), 12.0);
    assert_eq!(stream.frame_count(), 11);
    assert!((stream.duration().as_secs_f64() - 11.0 / 12.0).abs() < 1e-9);
    assert_eq!(stream.pixels().len(), 50 * 30 * 4);
}

#[test]
fn the_first_frame_is_ready_at_once() {
    let stream = open();
    assert_eq!(stream.frame(), 0);
    assert!(is_red(color(&stream)), "{:?}", color(&stream));
    assert!(
        stream
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255)
    );
}

#[test]
fn seeking_shows_the_frame_at_that_time_in_either_direction() {
    let mut stream = open();
    assert!(stream.seek(Duration::from_secs_f64(0.6)));
    // Frame 7 repeats frame 6, the first blue one.
    assert_eq!(stream.frame(), 6);
    assert!(is_blue(color(&stream)), "{:?}", color(&stream));

    assert!(stream.seek(Duration::from_secs_f64(0.4)));
    assert_eq!(stream.frame(), 4);
    assert!(is_red(color(&stream)), "{:?}", color(&stream));

    // Frame 5 is still frame 4, so nothing changes.
    assert!(!stream.seek(Duration::from_secs_f64(0.45)));
    // Past the end holds the last frame.
    stream.seek(Duration::from_secs_f64(5.0));
    assert_eq!(stream.frame(), 10);
}

#[test]
fn a_frame_reached_by_seeking_matches_one_reached_by_playing() {
    let mut played = open();
    for frame in 0..=10 {
        played.seek(Duration::from_secs_f64(frame as f64 / 12.0));
    }
    let mut jumped = open();
    jumped.seek(Duration::from_secs_f64(10.0 / 12.0));
    assert_eq!(played.frame(), jumped.frame());
    assert_eq!(played.pixels(), jumped.pixels());
}

#[test]
fn the_sound_track_can_be_decoded_from_the_same_file() {
    let sound = crate::audio::decode::Decoder::open(Arc::from(TEST_VIDEO)).unwrap();
    assert_eq!(sound.rate(), 22_050);
    assert_eq!(sound.channels(), 1);
    let decoded = sound.decode_all();
    assert!(decoded.frames.len() > 20_000, "{}", decoded.frames.len());
}

#[test]
fn files_without_theora_are_errors() {
    assert!(VideoStream::from_bytes(&b"not a video"[..]).is_err());
    let music = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../examples/assets/loop.ogg"
    ))
    .unwrap();
    let error = VideoStream::from_bytes(music).unwrap_err();
    assert!(error.to_string().contains("no Theora"), "{error}");
}

/// FFmpeg's moving `testsrc2` pattern, 48 by 32 pixels, 1.5 seconds at
/// 12 frames a second with a keyframe at least every 8.
const MOVING_VIDEO: &[u8] = include_bytes!("motion.ogv");

#[test]
fn frames_between_keyframes_decode_the_same_however_they_are_reached() {
    let mut played = VideoStream::from_bytes(MOVING_VIDEO).unwrap();
    let count = played.frame_count();
    assert!(count >= 17, "{count}");
    let mut seen = Vec::new();
    for frame in 0..count {
        played.seek(Duration::from_secs_f64(frame as f64 / 12.0 + 0.01));
        seen.push(played.pixels().to_vec());
    }
    // The pattern moves, so most frames differ from the one before.
    let changes = seen.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert!(
        changes as u64 >= count / 2,
        "{changes} changes in {count} frames"
    );

    // Jump about, backwards and forwards, and land on the same pictures.
    let mut jumping = VideoStream::from_bytes(MOVING_VIDEO).unwrap();
    for frame in [count - 1, 3, 9, 2, 14, 7, 0, 11] {
        jumping.seek(Duration::from_secs_f64(frame as f64 / 12.0 + 0.01));
        let index = jumping.frame() as usize;
        assert!(
            jumping.pixels() == &seen[index][..],
            "frame {frame} differs"
        );
    }
}
