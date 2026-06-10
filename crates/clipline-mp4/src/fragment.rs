use crate::boxes::{full_box, mp4_box, Payload};

/// trun sample_flags for a sync sample (I-frame).
const FLAG_SYNC: u32 = 0x0200_0000;
/// trun sample_flags for a non-sync sample (depends on others).
const FLAG_NON_SYNC: u32 = 0x0101_0000;

/// One encoded sample handed to the muxer.
#[derive(Debug, Clone)]
pub struct FragSample {
    /// Encoded bytes in MP4 stream format (length-prefixed NALs for AVC).
    pub data: Vec<u8>,
    /// Duration in media-timescale ticks.
    pub duration: u32,
    pub is_sync: bool,
}

/// One track's slice of a fragment.
#[derive(Debug)]
pub struct TrackRun<'a> {
    pub track_id: u32,
    /// In this track's timescale ticks.
    pub base_decode_time: u64,
    pub samples: &'a [FragSample],
}

/// One `moof` with a `traf` per run, plus one shared `mdat` holding all
/// runs' samples in run order.
pub fn fragment_multi(sequence: u32, runs: &[TrackRun<'_>]) -> Vec<u8> {
    // Two-pass: data offsets depend on the moof's size, which is stable.
    let zeros = vec![0i32; runs.len()];
    let moof = build_moof_multi(sequence, runs, &zeros);
    let mut offsets = Vec::with_capacity(runs.len());
    let mut acc = (moof.len() + 8) as i32; // + mdat header
    for r in runs {
        offsets.push(acc);
        acc += r.samples.iter().map(|s| s.data.len()).sum::<usize>() as i32;
    }
    let moof = build_moof_multi(sequence, runs, &offsets);

    let mut mdat_payload = Vec::new();
    for r in runs {
        for s in r.samples {
            mdat_payload.extend_from_slice(&s.data);
        }
    }
    let mut out = moof;
    out.extend(mp4_box(*b"mdat", mdat_payload));
    out
}

/// Single-track fragment (track 1) — the original API.
pub fn fragment(sequence: u32, base_decode_time: u64, samples: &[FragSample]) -> Vec<u8> {
    fragment_multi(sequence, &[TrackRun { track_id: 1, base_decode_time, samples }])
}

fn build_moof_multi(sequence: u32, runs: &[TrackRun<'_>], data_offsets: &[i32]) -> Vec<u8> {
    let mut mfhd_p = Payload::new();
    mfhd_p.u32(sequence);
    let mut moof = full_box(*b"mfhd", 0, 0, mfhd_p.into_vec());
    for (run, &off) in runs.iter().zip(data_offsets) {
        moof.extend(traf(run, off));
    }
    mp4_box(*b"moof", moof)
}

fn traf(run: &TrackRun<'_>, data_offset: i32) -> Vec<u8> {
    let mut tfhd_p = Payload::new();
    tfhd_p.u32(run.track_id);
    let tfhd = full_box(*b"tfhd", 0, 0x020000, tfhd_p.into_vec()); // default-base-is-moof

    let mut tfdt_p = Payload::new();
    tfdt_p.u64(run.base_decode_time);
    let tfdt = full_box(*b"tfdt", 1, 0, tfdt_p.into_vec());

    // flags: data-offset(0x1) | sample-duration(0x100) | sample-size(0x200)
    //        | sample-flags(0x400)
    let mut trun_p = Payload::new();
    trun_p.u32(run.samples.len() as u32).i32(data_offset);
    for s in run.samples {
        trun_p.u32(s.duration).u32(s.data.len() as u32).u32(if s.is_sync {
            FLAG_SYNC
        } else {
            FLAG_NON_SYNC
        });
    }
    let trun = full_box(*b"trun", 0, 0x000701, trun_p.into_vec());

    let mut t = tfhd;
    t.extend(tfdt);
    t.extend(trun);
    mp4_box(*b"traf", t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walker::{children, find, walk};

    fn samples() -> Vec<FragSample> {
        vec![
            FragSample { data: b"KEYFRAME".to_vec(), duration: 1500, is_sync: true },
            FragSample { data: b"delta1".to_vec(), duration: 1500, is_sync: false },
        ]
    }

    #[test]
    fn fragment_is_moof_then_mdat_with_sample_bytes() {
        let buf = fragment(1, 0, &samples());
        let boxes = walk(&buf);
        assert_eq!(&boxes[0].fourcc, b"moof");
        assert_eq!(&boxes[1].fourcc, b"mdat");
        let mdat_payload =
            &buf[boxes[1].payload_offset as usize..(boxes[1].offset + boxes[1].size) as usize];
        assert_eq!(mdat_payload, b"KEYFRAMEdelta1");
    }

    #[test]
    fn trun_data_offset_points_at_first_sample_byte() {
        let buf = fragment(1, 0, &samples());
        let boxes = walk(&buf);
        let moof = &boxes[0];
        let kids = children(&buf, moof);
        let traf = find(&kids, b"traf").unwrap();
        let traf_kids = children(&buf, traf);
        let trun = find(&traf_kids, b"trun").unwrap();
        // trun payload: version/flags(4) sample_count(4) data_offset(4)…
        let p = trun.payload_offset as usize;
        let data_offset =
            i32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap()) as u64;
        // default-base-is-moof: offset is relative to moof start (= 0 here).
        assert_eq!(&buf[data_offset as usize..data_offset as usize + 8], b"KEYFRAME");
    }

    #[test]
    fn multi_track_fragment_has_one_traf_per_track() {
        let video = samples();
        let audio =
            vec![FragSample { data: b"OPUSPKT1".to_vec(), duration: 960, is_sync: true }];
        let runs = [
            TrackRun { track_id: 1, base_decode_time: 0, samples: &video },
            TrackRun { track_id: 2, base_decode_time: 0, samples: &audio },
        ];
        let buf = fragment_multi(9, &runs);
        let boxes = walk(&buf);
        let kids = children(&buf, &boxes[0]);
        let trafs: Vec<_> = kids.iter().filter(|b| &b.fourcc == b"traf").collect();
        assert_eq!(trafs.len(), 2);

        // Each traf's trun data_offset points at that track's first byte
        // within the shared mdat.
        for (traf, expected) in
            trafs.iter().zip([b"KEYFRAME".as_slice(), b"OPUSPKT1".as_slice()])
        {
            let tk = children(&buf, traf);
            let trun = find(&tk, b"trun").unwrap();
            let p = trun.payload_offset as usize;
            let off = i32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap()) as usize;
            assert_eq!(&buf[off..off + expected.len()], expected);
        }
    }

    #[test]
    fn tfdt_carries_base_decode_time() {
        let buf = fragment(7, 123_456, &samples());
        let boxes = walk(&buf);
        let kids = children(&buf, &boxes[0]);
        let traf = find(&kids, b"traf").unwrap();
        let traf_kids = children(&buf, traf);
        let tfdt = find(&traf_kids, b"tfdt").unwrap();
        let p = tfdt.payload_offset as usize;
        assert_eq!(buf[p], 1, "tfdt version 1 (64-bit)");
        let bdt = u64::from_be_bytes(buf[p + 4..p + 12].try_into().unwrap());
        assert_eq!(bdt, 123_456);
    }
}
