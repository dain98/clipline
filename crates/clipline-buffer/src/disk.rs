use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::segment::{SampleInfo, Segment, TrackSamples};

#[derive(Debug)]
pub struct DiskReplayRing {
    max_bytes: usize,
    dir: PathBuf,
    segments: VecDeque<DiskSegment>,
    bytes: usize,
    next_id: u64,
}

#[derive(Debug, Clone)]
pub struct DiskSegment {
    pub starts_with_keyframe: bool,
    pub pts_start_s: f64,
    pub duration_s: f64,
    path: PathBuf,
    byte_len: usize,
    video_len: usize,
    samples: Vec<SampleInfo>,
    audio: Vec<DiskTrack>,
}

#[derive(Debug, Clone)]
struct DiskTrack {
    offset: usize,
    len: usize,
    samples: Vec<SampleInfo>,
}

impl DiskReplayRing {
    pub fn new(max_bytes: usize, dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self {
            max_bytes,
            dir,
            segments: VecDeque::new(),
            bytes: 0,
            next_id: 0,
        })
    }

    pub fn push(&mut self, seg: Segment) -> io::Result<()> {
        self.push_ref(&seg)
    }

    /// Persist a borrowed segment so another immutable consumer can retain
    /// the same payload without a deep clone.
    pub fn push_ref(&mut self, seg: &Segment) -> io::Result<()> {
        let id = self.next_id;
        let path = self.dir.join(format!("seg_{id:08}.bin"));
        let tmp = self.dir.join(format!("seg_{id:08}.tmp"));
        let created = File::create(&tmp)?;
        let mut tmp_owner = OwnedFile::new(tmp.clone());
        let mut file = created;
        file.write_all(&seg.data)?;
        let mut offset = seg.data.len();
        let mut audio = Vec::with_capacity(seg.audio.len());
        for track in &seg.audio {
            file.write_all(&track.data)?;
            audio.push(DiskTrack {
                offset,
                len: track.data.len(),
                samples: track.samples.clone(),
            });
            offset += track.data.len();
        }
        file.flush()?;
        drop(file);
        fs::rename(&tmp, &path)?;
        tmp_owner.disarm();
        let mut final_owner = OwnedFile::new(path.clone());

        let stored = DiskSegment {
            starts_with_keyframe: seg.starts_with_keyframe,
            pts_start_s: seg.pts_start_s,
            duration_s: seg.duration_s,
            path,
            byte_len: seg.byte_len(),
            video_len: seg.data.len(),
            samples: seg.samples.clone(),
            audio,
        };
        let mut committed_bytes = self.bytes.saturating_add(stored.byte_len);
        while committed_bytes > self.max_bytes && !self.segments.is_empty() {
            let front = self
                .segments
                .front()
                .expect("non-empty ring has a front segment");
            fs::remove_file(&front.path)?;
            let front = self.segments.pop_front().expect("front segment exists");
            self.bytes -= front.byte_len;
            committed_bytes -= front.byte_len;
        }
        self.bytes = committed_bytes;
        self.segments.push_back(stored);
        self.next_id += 1;
        final_owner.disarm();
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn segments(&self) -> impl Iterator<Item = &DiskSegment> {
        self.segments.iter()
    }

    pub fn save_window(&self, window_s: f64, exclude_before_s: Option<f64>) -> Vec<&DiskSegment> {
        let Some(last) = self.segments.back() else {
            return Vec::new();
        };
        let mut start_target = last.pts_end_s() - window_s;
        if let Some(x) = exclude_before_s {
            start_target = start_target.max(x);
        }

        let mut start_idx = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.starts_with_keyframe && s.pts_start_s <= start_target)
            .map(|(i, _)| i)
            .next_back();
        if start_idx.is_none() {
            start_idx = self.segments.iter().position(|s| s.starts_with_keyframe);
        }
        let Some(mut idx) = start_idx else {
            return Vec::new();
        };

        if let Some(x) = exclude_before_s {
            while idx < self.segments.len() && self.segments[idx].pts_end_s() <= x {
                idx += 1;
            }
            while idx < self.segments.len() && !self.segments[idx].starts_with_keyframe {
                idx += 1;
            }
        }

        self.segments.iter().skip(idx).collect()
    }
}

impl Drop for DiskReplayRing {
    fn drop(&mut self) {
        self.segments.clear();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct OwnedFile {
    path: PathBuf,
    armed: bool,
}

impl OwnedFile {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for OwnedFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl DiskSegment {
    pub fn pts_end_s(&self) -> f64 {
        self.pts_start_s + self.duration_s
    }

    pub fn byte_len(&self) -> usize {
        self.byte_len
    }

    pub fn load(&self) -> io::Result<Segment> {
        let mut buf = Vec::new();
        File::open(&self.path)?.read_to_end(&mut buf)?;
        if buf.len() < self.byte_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("replay segment {:?} is truncated", self.path),
            ));
        }
        let data = buf[..self.video_len].to_vec();
        let audio = self
            .audio
            .iter()
            .map(|track| {
                let start = track.offset;
                let end = start + track.len;
                TrackSamples {
                    data: buf[start..end].to_vec(),
                    samples: track.samples.clone(),
                }
            })
            .collect();
        Ok(Segment {
            starts_with_keyframe: self.starts_with_keyframe,
            pts_start_s: self.pts_start_s,
            duration_s: self.duration_s,
            data,
            samples: self.samples.clone(),
            audio,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipline_test_utils::TestDir;

    fn seg(pts: f64, dur: f64, bytes: usize, key: bool) -> Segment {
        Segment {
            starts_with_keyframe: key,
            pts_start_s: pts,
            duration_s: dur,
            data: vec![b'v'; bytes],
            samples: vec![SampleInfo {
                size: bytes as u32,
                duration_s: dur,
                is_sync: key,
            }],
            audio: vec![TrackSamples {
                data: vec![b'a'; bytes / 2],
                samples: vec![SampleInfo {
                    size: (bytes / 2) as u32,
                    duration_s: dur,
                    is_sync: true,
                }],
            }],
        }
    }

    #[test]
    fn stores_payloads_on_disk_and_loads_segments() {
        let dir = TestDir::new("clipline-disk-ring", "load");
        let mut ring = DiskReplayRing::new(10_000, dir.path().to_path_buf()).unwrap();
        ring.push(seg(0.0, 1.0, 100, true)).unwrap();

        let stored = ring.segments().next().unwrap();
        assert!(stored.path().exists());
        let loaded = stored.load().unwrap();
        assert_eq!(loaded.data.len(), 100);
        assert_eq!(loaded.audio[0].data.len(), 50);
    }

    #[test]
    fn eviction_deletes_owned_segment_files() {
        let dir = TestDir::new("clipline-disk-ring", "evict");
        let mut ring = DiskReplayRing::new(250, dir.path().to_path_buf()).unwrap();
        ring.push(seg(0.0, 1.0, 100, true)).unwrap();
        let first = ring.segments().next().unwrap().path().to_path_buf();
        ring.push(seg(1.0, 1.0, 100, true)).unwrap();
        ring.push(seg(2.0, 1.0, 100, true)).unwrap();

        assert_eq!(ring.len(), 1);
        assert!(!first.exists());
    }

    #[test]
    fn failed_publish_cleans_owned_temp_without_touching_collision() {
        let dir = TestDir::new("clipline-disk-ring", "publish-failure");
        let run = dir.path().join("run");
        let mut ring = DiskReplayRing::new(10_000, run.clone()).unwrap();
        let collision = run.join("seg_00000000.bin");
        std::fs::create_dir(&collision).unwrap();

        let error = ring.push(seg(0.0, 1.0, 100, true)).unwrap_err();

        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert!(!run.join("seg_00000000.tmp").exists());
        assert!(collision.is_dir());
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.bytes(), 0);
    }

    #[test]
    fn eviction_failure_discards_new_segment_and_keeps_bookkeeping_bounded() {
        let dir = TestDir::new("clipline-disk-ring", "eviction-failure");
        let run = dir.path().join("run");
        let mut ring = DiskReplayRing::new(200, run.clone()).unwrap();
        ring.push(seg(0.0, 1.0, 100, true)).unwrap();
        let first = ring.segments().next().unwrap().path().to_path_buf();
        std::fs::remove_file(&first).unwrap();
        std::fs::create_dir(&first).unwrap();

        let error = ring.push(seg(1.0, 1.0, 100, true)).unwrap_err();

        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.bytes(), 150);
        assert!(!run.join("seg_00000001.bin").exists());
        assert!(!run.join("seg_00000001.tmp").exists());
    }

    #[test]
    fn drop_removes_owned_run_directory_including_orphan_temps() {
        let dir = TestDir::new("clipline-disk-ring", "drop-run");
        let run = dir.path().join("run");
        let mut ring = DiskReplayRing::new(10_000, run.clone()).unwrap();
        ring.push(seg(0.0, 1.0, 100, true)).unwrap();
        std::fs::write(run.join("orphan.tmp"), b"partial").unwrap();

        drop(ring);

        assert!(!run.exists());
    }
}
