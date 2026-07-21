use crate::boxes::{full_box, mp4_box, Payload};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentError {
    SampleSizeExceedsU32,
    PayloadSizeOverflow,
    SampleCountExceedsU32,
    DataOffsetExceedsI32,
}

impl std::fmt::Display for FragmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::SampleSizeExceedsU32 => "fragment sample exceeds the 32-bit MP4 size field",
            Self::PayloadSizeOverflow => "fragment media payload size overflow",
            Self::SampleCountExceedsU32 => "fragment sample count exceeds the 32-bit trun field",
            Self::DataOffsetExceedsI32 => {
                "fragment data offset exceeds the signed 32-bit trun field"
            }
        };
        f.write_str(message)
    }
}

impl std::error::Error for FragmentError {}

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

#[derive(Debug, Clone, Copy)]
pub struct FragSampleInfo {
    pub size: u32,
    pub duration: u32,
    pub is_sync: bool,
}

/// One encoded sample borrowed from an existing GOP segment.
#[derive(Debug, Clone, Copy)]
pub struct FragSampleRef<'a> {
    /// Encoded bytes in MP4 stream format (length-prefixed NALs for AVC).
    pub data: &'a [u8],
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

#[derive(Debug)]
pub struct TrackRunInfo<'a> {
    pub track_id: u32,
    pub base_decode_time: u64,
    pub samples: &'a [FragSampleInfo],
}

/// One `moof` with a `traf` per run, plus one shared `mdat` holding all
/// runs' samples in run order.
pub fn fragment_multi(sequence: u32, runs: &[TrackRun<'_>]) -> Result<Vec<u8>, FragmentError> {
    let infos: Vec<Vec<FragSampleInfo>> = runs
        .iter()
        .map(|run| {
            run.samples
                .iter()
                .map(|s| {
                    Ok(FragSampleInfo {
                        size: u32::try_from(s.data.len())
                            .map_err(|_| FragmentError::SampleSizeExceedsU32)?,
                        duration: s.duration,
                        is_sync: s.is_sync,
                    })
                })
                .collect::<Result<_, FragmentError>>()
        })
        .collect::<Result<_, FragmentError>>()?;
    let info_runs: Vec<TrackRunInfo<'_>> = runs
        .iter()
        .zip(&infos)
        .map(|(run, samples)| TrackRunInfo {
            track_id: run.track_id,
            base_decode_time: run.base_decode_time,
            samples,
        })
        .collect();
    let moof = fragment_moof_multi(sequence, &info_runs)?;

    let total_payload =
        runs.iter()
            .flat_map(|run| run.samples.iter())
            .try_fold(0_usize, |total, sample| {
                total
                    .checked_add(sample.data.len())
                    .ok_or(FragmentError::PayloadSizeOverflow)
            })?;
    let header =
        mdat_header(u64::try_from(total_payload).map_err(|_| FragmentError::PayloadSizeOverflow)?);
    let capacity = moof
        .len()
        .checked_add(header.len())
        .and_then(|len| len.checked_add(total_payload))
        .ok_or(FragmentError::PayloadSizeOverflow)?;
    let mut out = Vec::new();
    out.try_reserve_exact(capacity)
        .map_err(|_| FragmentError::PayloadSizeOverflow)?;
    out.extend(moof);
    out.extend(header);
    for r in runs {
        for s in r.samples {
            out.extend_from_slice(&s.data);
        }
    }
    Ok(out)
}

pub fn fragment_moof_multi(
    sequence: u32,
    runs: &[TrackRunInfo<'_>],
) -> Result<Vec<u8>, FragmentError> {
    if runs
        .iter()
        .any(|run| u32::try_from(run.samples.len()).is_err())
    {
        return Err(FragmentError::SampleCountExceedsU32);
    }
    let total_payload =
        runs.iter()
            .flat_map(|run| run.samples.iter())
            .try_fold(0_u64, |total, sample| {
                total
                    .checked_add(sample.size as u64)
                    .ok_or(FragmentError::PayloadSizeOverflow)
            })?;
    let mdat_header_len = mdat_header_len(total_payload)?;

    // Two-pass: data offsets depend on the moof's size, which is stable.
    let zeros = vec![0i32; runs.len()];
    let moof = build_moof_multi(sequence, runs, &zeros);
    let mut offsets = Vec::with_capacity(runs.len());
    let first_offset = moof
        .len()
        .checked_add(mdat_header_len)
        .ok_or(FragmentError::PayloadSizeOverflow)?;
    let mut acc = i32::try_from(first_offset).map_err(|_| FragmentError::DataOffsetExceedsI32)?;
    for (index, run) in runs.iter().enumerate() {
        offsets.push(acc);
        if index + 1 < runs.len() {
            let run_size = run.samples.iter().try_fold(0_u64, |total, sample| {
                total
                    .checked_add(sample.size as u64)
                    .ok_or(FragmentError::PayloadSizeOverflow)
            })?;
            let next = (acc as u64)
                .checked_add(run_size)
                .ok_or(FragmentError::PayloadSizeOverflow)?;
            acc = i32::try_from(next).map_err(|_| FragmentError::DataOffsetExceedsI32)?;
        }
    }
    Ok(build_moof_multi(sequence, runs, &offsets))
}

/// Single-track fragment (track 1) — the original API.
pub fn fragment(
    sequence: u32,
    base_decode_time: u64,
    samples: &[FragSample],
) -> Result<Vec<u8>, FragmentError> {
    fragment_multi(
        sequence,
        &[TrackRun {
            track_id: 1,
            base_decode_time,
            samples,
        }],
    )
}

pub fn mdat_header(payload_len: u64) -> Vec<u8> {
    if payload_len <= (u32::MAX as u64 - 8) {
        let mut out = Vec::with_capacity(8);
        out.extend(((payload_len + 8) as u32).to_be_bytes());
        out.extend(b"mdat");
        out
    } else {
        let size = payload_len
            .checked_add(16)
            .expect("mdat payload exceeds the 64-bit large-size field");
        let mut out = Vec::with_capacity(16);
        out.extend(1u32.to_be_bytes());
        out.extend(b"mdat");
        out.extend(size.to_be_bytes());
        out
    }
}

fn mdat_header_len(payload_len: u64) -> Result<usize, FragmentError> {
    if payload_len <= u32::MAX as u64 - 8 {
        Ok(8)
    } else {
        payload_len
            .checked_add(16)
            .map(|_| 16)
            .ok_or(FragmentError::PayloadSizeOverflow)
    }
}

fn build_moof_multi(sequence: u32, runs: &[TrackRunInfo<'_>], data_offsets: &[i32]) -> Vec<u8> {
    let mut mfhd_p = Payload::new();
    mfhd_p.u32(sequence);
    let mut moof = full_box(*b"mfhd", 0, 0, mfhd_p.into_vec());
    for (run, &off) in runs.iter().zip(data_offsets) {
        moof.extend(traf(run, off));
    }
    mp4_box(*b"moof", moof)
}

fn traf(run: &TrackRunInfo<'_>, data_offset: i32) -> Vec<u8> {
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
        trun_p
            .u32(s.duration)
            .u32(s.size)
            .u32(if s.is_sync { FLAG_SYNC } else { FLAG_NON_SYNC });
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
            FragSample {
                data: b"KEYFRAME".to_vec(),
                duration: 1500,
                is_sync: true,
            },
            FragSample {
                data: b"delta1".to_vec(),
                duration: 1500,
                is_sync: false,
            },
        ]
    }

    #[test]
    fn fragment_is_moof_then_mdat_with_sample_bytes() {
        let buf = fragment(1, 0, &samples()).unwrap();
        let boxes = walk(&buf);
        assert_eq!(&boxes[0].fourcc, b"moof");
        assert_eq!(&boxes[1].fourcc, b"mdat");
        let mdat_payload =
            &buf[boxes[1].payload_offset as usize..(boxes[1].offset + boxes[1].size) as usize];
        assert_eq!(mdat_payload, b"KEYFRAMEdelta1");
    }

    #[test]
    fn trun_data_offset_points_at_first_sample_byte() {
        let buf = fragment(1, 0, &samples()).unwrap();
        let boxes = walk(&buf);
        let moof = &boxes[0];
        let kids = children(&buf, moof);
        let traf = find(&kids, b"traf").unwrap();
        let traf_kids = children(&buf, traf);
        let trun = find(&traf_kids, b"trun").unwrap();
        // trun payload: version/flags(4) sample_count(4) data_offset(4)…
        let p = trun.payload_offset as usize;
        let data_offset = i32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap()) as u64;
        // default-base-is-moof: offset is relative to moof start (= 0 here).
        assert_eq!(
            &buf[data_offset as usize..data_offset as usize + 8],
            b"KEYFRAME"
        );
    }

    #[test]
    fn multi_track_fragment_has_one_traf_per_track() {
        let video = samples();
        let audio = vec![FragSample {
            data: b"OPUSPKT1".to_vec(),
            duration: 960,
            is_sync: true,
        }];
        let runs = [
            TrackRun {
                track_id: 1,
                base_decode_time: 0,
                samples: &video,
            },
            TrackRun {
                track_id: 2,
                base_decode_time: 0,
                samples: &audio,
            },
        ];
        let buf = fragment_multi(9, &runs).unwrap();
        let boxes = walk(&buf);
        let kids = children(&buf, &boxes[0]);
        let trafs: Vec<_> = kids.iter().filter(|b| &b.fourcc == b"traf").collect();
        assert_eq!(trafs.len(), 2);

        // Each traf's trun data_offset points at that track's first byte
        // within the shared mdat.
        for (traf, expected) in trafs
            .iter()
            .zip([b"KEYFRAME".as_slice(), b"OPUSPKT1".as_slice()])
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
        let buf = fragment(7, 123_456, &samples()).unwrap();
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

    #[test]
    fn mdat_header_uses_large_size_past_the_32_bit_limit() {
        let header = mdat_header(u32::MAX as u64);
        assert_eq!(header.len(), 16);
        assert_eq!(&header[0..4], &1_u32.to_be_bytes());
        assert_eq!(&header[4..8], b"mdat");
        assert_eq!(
            u64::from_be_bytes(header[8..16].try_into().unwrap()),
            u32::MAX as u64 + 16
        );
    }

    #[test]
    fn moof_rejects_trun_data_offsets_past_i32() {
        let first = [FragSampleInfo {
            size: i32::MAX as u32,
            duration: 1,
            is_sync: true,
        }];
        let second = [FragSampleInfo {
            size: 1,
            duration: 1,
            is_sync: true,
        }];
        let runs = [
            TrackRunInfo {
                track_id: 1,
                base_decode_time: 0,
                samples: &first,
            },
            TrackRunInfo {
                track_id: 2,
                base_decode_time: 0,
                samples: &second,
            },
        ];

        let err = fragment_moof_multi(1, &runs).unwrap_err();
        assert_eq!(
            err.to_string(),
            "fragment data offset exceeds the signed 32-bit trun field"
        );
    }

    #[test]
    fn moof_offsets_account_for_a_large_mdat_header() {
        let samples = [
            FragSampleInfo {
                size: u32::MAX,
                duration: 1,
                is_sync: true,
            },
            FragSampleInfo {
                size: u32::MAX,
                duration: 1,
                is_sync: false,
            },
        ];
        let runs = [TrackRunInfo {
            track_id: 1,
            base_decode_time: 0,
            samples: &samples,
        }];

        let moof = fragment_moof_multi(1, &runs).unwrap();
        let top = walk(&moof);
        let traf = find(&children(&moof, &top[0]), b"traf").unwrap().clone();
        let trun = find(&children(&moof, &traf), b"trun").unwrap().clone();
        let p = trun.payload_offset as usize;
        let data_offset = i32::from_be_bytes(moof[p + 8..p + 12].try_into().unwrap());

        assert_eq!(data_offset as usize, moof.len() + 16);
    }
}
