use std::io::Cursor;

use clipline_mp4::walker::{children, find, walk};
use clipline_mp4::{FragSample, HybridMp4Writer, VideoTrackConfig};

fn cfg() -> VideoTrackConfig {
    VideoTrackConfig::h264(
        64,
        64,
        90_000,
        vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
        vec![0x68, 0xEE, 0x38, 0x80],
    )
}

fn gop(start: u32) -> Vec<FragSample> {
    (0..3)
        .map(|i| FragSample {
            data: format!("sample-{:04}", start + i).into_bytes(),
            duration: 3000, // 30 fps @ 90kHz
            is_sync: i == 0,
        })
        .collect()
}

#[test]
fn while_recording_file_is_fragmented_and_walkable() {
    let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg()).unwrap();
    w.write_fragment(&gop(0)).unwrap();
    w.write_fragment(&gop(3)).unwrap();
    // Simulate a crash: inspect the bytes WITHOUT finalize.
    let buf = w.into_inner().into_inner();
    let boxes = walk(&buf);
    let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
    assert_eq!(
        fourccs,
        vec![b"ftyp", b"free", b"moov", b"moof", b"mdat", b"moof", b"mdat"],
        "fragmented layout must survive a crash mid-recording"
    );
}

#[test]
fn finalized_file_reads_as_standard_mp4() {
    let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg()).unwrap();
    w.write_fragment(&gop(0)).unwrap();
    w.write_fragment(&gop(3)).unwrap();
    let buf = w.finalize().unwrap().into_inner();

    // Standard layout: the free box became a giant mdat hiding the
    // fragments; a full moov sits at the end.
    let boxes = walk(&buf);
    let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
    assert_eq!(fourccs, vec![b"ftyp", b"mdat", b"moov"]);

    // The final moov has populated tables.
    let moov = find(&boxes, b"moov").unwrap();
    let buf_ref = &buf;
    let moov_kids = children(buf_ref, moov);
    assert!(
        find(&moov_kids, b"mvex").is_none(),
        "final moov is not fragmented"
    );

    // stsz lists 6 samples; co64 chunk offsets point at real sample bytes.
    let stsz = find_deep(buf_ref, moov, b"stsz").expect("stsz");
    let p = stsz.payload_offset as usize;
    let sample_count = u32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap());
    assert_eq!(sample_count, 6);

    let co64 = find_deep(buf_ref, moov, b"co64").expect("co64");
    let p = co64.payload_offset as usize;
    let entry_count = u32::from_be_bytes(buf[p + 4..p + 8].try_into().unwrap());
    assert_eq!(entry_count, 2, "one chunk per fragment");
    let first_chunk = u64::from_be_bytes(buf[p + 8..p + 16].try_into().unwrap()) as usize;
    assert_eq!(&buf[first_chunk..first_chunk + 11], b"sample-0000");

    // stss marks samples 1 and 4 as sync.
    let stss = find_deep(buf_ref, moov, b"stss").expect("stss");
    let p = stss.payload_offset as usize;
    let n = u32::from_be_bytes(buf[p + 4..p + 8].try_into().unwrap());
    assert_eq!(n, 2);
    assert_eq!(
        u32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap()),
        1
    );
    assert_eq!(
        u32::from_be_bytes(buf[p + 12..p + 16].try_into().unwrap()),
        4
    );
}

/// Depth-first search for a fourcc under a container box.
fn find_deep(
    buf: &[u8],
    parent: &clipline_mp4::walker::BoxInfo,
    fourcc: &[u8; 4],
) -> Option<clipline_mp4::walker::BoxInfo> {
    const CONTAINERS: [&[u8; 4]; 6] = [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts"];
    for child in children(buf, parent) {
        if &child.fourcc == fourcc {
            return Some(child);
        }
        if CONTAINERS.contains(&&child.fourcc) {
            if let Some(hit) = find_deep(buf, &child, fourcc) {
                return Some(hit);
            }
        }
    }
    None
}
