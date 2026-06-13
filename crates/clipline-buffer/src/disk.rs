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
        let id = self.next_id;
        self.next_id += 1;
        let path = self.dir.join(format!("seg_{id:08}.bin"));
        let tmp = self.dir.join(format!("seg_{id:08}.tmp"));
        let mut file = File::create(&tmp)?;
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

        let stored = DiskSegment {
            starts_with_keyframe: seg.starts_with_keyframe,
            pts_start_s: seg.pts_start_s,
            duration_s: seg.duration_s,
            path,
            byte_len: seg.byte_len(),
            video_len: seg.data.len(),
            samples: seg.samples,
            audio,
        };
        self.bytes += stored.byte_len;
        self.segments.push_back(stored);
        while self.bytes > self.max_bytes && self.segments.len() > 1 {
            if let Some(front) = self.segments.pop_front() {
                self.bytes -= front.byte_len;
                let _ = fs::remove_file(front.path);
            }
        }
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
        for seg in self.segments.drain(..) {
            let _ = fs::remove_file(seg.path);
        }
        let _ = fs::remove_dir(&self.dir);
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

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "clipline-disk-ring-{name}-{}-{unique}",
                std::process::id()
            ));
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

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
        let dir = TestDir::new("load");
        let mut ring = DiskReplayRing::new(10_000, dir.0.clone()).unwrap();
        ring.push(seg(0.0, 1.0, 100, true)).unwrap();

        let stored = ring.segments().next().unwrap();
        assert!(stored.path().exists());
        let loaded = stored.load().unwrap();
        assert_eq!(loaded.data.len(), 100);
        assert_eq!(loaded.audio[0].data.len(), 50);
    }

    #[test]
    fn eviction_deletes_owned_segment_files() {
        let dir = TestDir::new("evict");
        let mut ring = DiskReplayRing::new(250, dir.0.clone()).unwrap();
        ring.push(seg(0.0, 1.0, 100, true)).unwrap();
        let first = ring.segments().next().unwrap().path().to_path_buf();
        ring.push(seg(1.0, 1.0, 100, true)).unwrap();
        ring.push(seg(2.0, 1.0, 100, true)).unwrap();

        assert_eq!(ring.len(), 1);
        assert!(!first.exists());
    }
}
