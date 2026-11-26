//! A 16-bit PCM wave file, written the way the application's audio device would
//! have played it.
//!
//! `engine-sim-script`'s `wav` module is the *reader* (`ysWindowsAudioWaveFile`,
//! which the application loads impulse responses with). This is the other
//! direction, and it is deliberately minimal: one `fmt ` chunk, one `data` chunk,
//! mono, 16-bit, 44.1 kHz -- the synthesizer's own format, so nothing is resampled
//! on the way out.

use std::fs::File;
use std::io::{BufWriter, Write};

/// Writes `samples` as a mono 16-bit PCM wave file at `sample_rate`.
pub fn write(path: &str, samples: &[i16], sample_rate: u32) -> Result<(), String> {
    let file = File::create(path).map_err(|error| format!("could not write {}: {}", path, error))?;
    let mut file = BufWriter::new(file);

    let data_size = (samples.len() * 2) as u32;

    let write = |file: &mut BufWriter<File>| -> std::io::Result<()> {
        file.write_all(b"RIFF")?;
        file.write_all(&(36 + data_size).to_le_bytes())?;
        file.write_all(b"WAVE")?;

        file.write_all(b"fmt ")?;
        file.write_all(&16u32.to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?; // PCM
        file.write_all(&1u16.to_le_bytes())?; // mono
        file.write_all(&sample_rate.to_le_bytes())?;
        file.write_all(&(sample_rate * 2).to_le_bytes())?; // byte rate
        file.write_all(&2u16.to_le_bytes())?; // block alignment
        file.write_all(&16u16.to_le_bytes())?; // bits per sample

        file.write_all(b"data")?;
        file.write_all(&data_size.to_le_bytes())?;
        for sample in samples {
            file.write_all(&sample.to_le_bytes())?;
        }

        Ok(())
    };

    write(&mut file).map_err(|error| format!("could not write {}: {}", path, error))?;
    file.flush()
        .map_err(|error| format!("could not write {}: {}", path, error))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_describes_a_mono_16_bit_file() {
        let path = std::env::temp_dir().join("engine-sim-cli-writer-test.wav");
        let path = path.to_string_lossy().to_string();

        write(&path, &[0, 1, -1, i16::MIN, i16::MAX], 44100).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 16);

        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 1); // PCM
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 1); // mono
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 44100);
        assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 88200);
        assert_eq!(u16::from_le_bytes(bytes[32..34].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 16);

        assert_eq!(&bytes[36..40], b"data");
        let size = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        assert_eq!(size, 10);
        assert_eq!(bytes.len(), 44 + 10);

        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 10);
        assert_eq!(i16::from_le_bytes(bytes[50..52].try_into().unwrap()), i16::MIN);
        assert_eq!(i16::from_le_bytes(bytes[52..54].try_into().unwrap()), i16::MAX);
    }
}
