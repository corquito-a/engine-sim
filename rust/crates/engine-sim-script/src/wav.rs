//! `ysWindowsAudioWaveFile`, as much of it as the application uses.
//!
//! The original opens an impulse response through Delta Studio's audio-file
//! layer, which on Windows is `mmio` against a RIFF file: find `fmt `, find
//! `data`, then read the data segment as raw PCM. `EngineSimApplication` passes
//! that buffer to `Synthesizer::initializeImpulseResponse` as an `int16_t *` with
//! `GetSampleCount()` -- which the file class computes as
//! `dataSize / blockAlign`, i.e. **frames**, not samples. For a mono file the two
//! are the same, and every impulse response in `es/sound-library/` is mono
//! 16-bit 44.1 kHz; a multi-channel file would be read interleaved, which is what
//! the original does with it too.
//!
//! What is kept from the original: `bitsPerSample` is derived from
//! `blockAlign / channels * 8` (not from the `fmt ` chunk's own field), the chunk
//! list is walked rather than assumed, and a chunk's size is padded to an even
//! length. What is not: the `mmio` handle and the Windows types.

use std::fs;

/// One decoded wave file.
#[derive(Clone, Debug, Default)]
pub struct Wave {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    /// `ysAudioFile::GetSampleCount()`: data bytes over block alignment.
    pub frames: usize,
    /// The first `frames` little-endian `int16`s of the data segment, which is
    /// what the application hands the synthesizer.
    pub samples: Vec<i16>,
}

/// Reads the file at `path`, which the caller has already resolved.
pub fn read(path: &str) -> Result<Wave, String> {
    let data = fs::read(path).map_err(|error| format!("could not open {}: {}", path, error))?;
    parse(&data).map_err(|error| format!("{}: {}", path, error))
}

/// The references: channels, sample rate, block alignment, and the data segment.
fn parse(data: &[u8]) -> Result<Wave, String> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a RIFF wave file".to_string());
    }

    let mut format: Option<(u16, u32, u16, u16)> = None;
    let mut segment: Option<&[u8]> = None;

    let mut offset = 12;
    while offset + 8 <= data.len() {
        let id = &data[offset..offset + 4];
        let size = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;
        let body = offset + 8;
        let end = body.saturating_add(size).min(data.len());

        if id == b"fmt " {
            if size < 16 || end < body + 16 {
                return Err("the format chunk is too short".to_string());
            }

            let channels = u16::from_le_bytes([data[body + 2], data[body + 3]]);
            let sample_rate = u32::from_le_bytes([
                data[body + 4],
                data[body + 5],
                data[body + 6],
                data[body + 7],
            ]);
            let block_align = u16::from_le_bytes([data[body + 12], data[body + 13]]);

            format = Some((channels, sample_rate, block_align, 0));
        }
        else if id == b"data" {
            segment = Some(&data[body..end]);
        }

        // A chunk's size is padded to an even boundary.
        offset = body + size + (size & 1);
    }

    let (channels, sample_rate, block_align, _) = format.ok_or("no format chunk")?;
    let segment = segment.ok_or("no data chunk")?;

    if block_align == 0 {
        return Err("the format chunk has no block alignment".to_string());
    }

    if segment.is_empty() {
        return Err("the data chunk is empty".to_string());
    }

    let bits_per_sample = if channels == 0 {
        0
    } else {
        (block_align / channels) * 8
    };
    let frames = segment.len() / block_align as usize;

    let samples = segment
        .chunks_exact(2)
        .take(frames)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();

    Ok(Wave {
        channels,
        sample_rate,
        bits_per_sample,
        frames,
        samples,
    })
}

/// FNV-1a over the decoded samples, which is how a gate compares two decoders
/// without printing tens of thousands of lines.
pub fn hash(wave: &Wave) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;

    for sample in &wave.samples {
        for byte in sample.to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16-bit mono wave file around four samples.
    fn wave(samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut file = Vec::new();

        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
        file.extend_from_slice(b"WAVE");
        file.extend_from_slice(b"fmt ");
        file.extend_from_slice(&16u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes()); // PCM
        file.extend_from_slice(&1u16.to_le_bytes()); // mono
        file.extend_from_slice(&44100u32.to_le_bytes());
        file.extend_from_slice(&88200u32.to_le_bytes());
        file.extend_from_slice(&2u16.to_le_bytes()); // block align
        file.extend_from_slice(&16u16.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&(data.len() as u32).to_le_bytes());
        file.extend_from_slice(&data);

        file
    }

    #[test]
    fn a_mono_file_decodes_to_its_samples() {
        let decoded = parse(&wave(&[0, 1000, -1000, i16::MAX])).unwrap();

        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.sample_rate, 44100);
        assert_eq!(decoded.bits_per_sample, 16);
        assert_eq!(decoded.frames, 4);
        assert_eq!(decoded.samples, vec![0, 1000, -1000, i16::MAX]);
    }

    /// Where the `data` chunk's header starts.
    fn data_chunk_at(file: &[u8]) -> usize {
        let mut offset = 12;
        while offset + 8 <= file.len() {
            if &file[offset..offset + 4] == b"data" {
                return offset;
            }

            let size = u32::from_le_bytes([
                file[offset + 4],
                file[offset + 5],
                file[offset + 6],
                file[offset + 7],
            ]) as usize;
            offset += 8 + size + (size & 1);
        }

        panic!("no data chunk");
    }

    #[test]
    fn the_chunk_list_is_walked_not_assumed() {
        // An odd-sized chunk between `fmt ` and `data`, so the walk has to skip
        // both the chunk and its pad byte.
        let original = wave(&[7, 8]);
        let at = data_chunk_at(&original);

        let mut file = original[..at].to_vec();
        file.extend_from_slice(b"odd ");
        file.extend_from_slice(&3u32.to_le_bytes());
        file.extend_from_slice(b"abc\0"); // three bytes plus the pad byte
        file.extend_from_slice(&original[at..]);

        let size = (file.len() - 8) as u32;
        file[4..8].copy_from_slice(&size.to_le_bytes());

        assert_eq!(parse(&file).unwrap().samples, vec![7, 8]);
    }

    #[test]
    fn the_frame_count_is_data_over_block_align() {
        // Two stereo frames: four `int16`s, block alignment four.
        let mut file = wave(&[1, 2, 3, 4]);
        let block_align = 4u16;
        file[32..34].copy_from_slice(&block_align.to_le_bytes());

        let decoded = parse(&file).unwrap();
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.bits_per_sample, 32);
        assert_eq!(decoded.frames, 2);
        // The synthesizer is handed `frames` values out of the interleaved run.
        assert_eq!(decoded.samples, vec![1, 2]);
    }

    #[test]
    fn a_file_that_is_not_a_wave_file_is_an_error() {
        assert!(parse(b"RIFF").is_err());
        assert!(parse(b"not a wave file at all").is_err());
    }

    #[test]
    fn a_missing_data_chunk_is_an_error() {
        let mut file = wave(&[1]);
        let at = data_chunk_at(&file);
        file[at..at + 4].copy_from_slice(b"junk");

        assert!(parse(&file).unwrap_err().contains("data"));
    }

    #[test]
    fn the_hash_covers_every_sample() {
        let one = parse(&wave(&[1, 2, 3])).unwrap();
        let two = parse(&wave(&[1, 2, 4])).unwrap();

        assert_eq!(hash(&one), hash(&parse(&wave(&[1, 2, 3])).unwrap()));
        assert_ne!(hash(&one), hash(&two));
    }
}
