use std::io::Cursor;
use std::thread;

use rodio::{Decoder, OutputStream, Sink};

const SOUND_EFFECT_OGG: &[u8] = include_bytes!("../../../soundeffect.ogg");

pub fn play_replay_saved() {
    let _ = thread::Builder::new()
        .name("clipline-replay-sound".into())
        .spawn(|| {
            if let Err(e) = play_once() {
                eprintln!("replay save sound: {e}");
            }
        });
}

fn play_once() -> Result<(), String> {
    let (_stream, handle) = OutputStream::try_default().map_err(|e| e.to_string())?;
    let sink = Sink::try_new(&handle).map_err(|e| e.to_string())?;
    let source = Decoder::new(Cursor::new(SOUND_EFFECT_OGG)).map_err(|e| e.to_string())?;
    sink.append(source);
    sink.sleep_until_end();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_sound_effect_decodes() {
        let mut decoder = Decoder::new(Cursor::new(SOUND_EFFECT_OGG)).expect("decode replay sound");
        assert!(
            decoder.next().is_some(),
            "sound effect must contain samples"
        );
    }
}
