//! WAV read/write (via `hound`) using planar `Vec<Vec<f32>>` channels.

use std::path::Path;

/// A decoded WAV file.
#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// One `Vec` per channel.
    pub channels: Vec<Vec<f32>>,
}

impl Wav {
    /// A mono file.
    pub fn mono(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self { sample_rate, channels: vec![samples] }
    }

    /// Frames per channel.
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    /// Concatenate channels into one planar buffer.
    pub fn to_planar(&self) -> Vec<f32> {
        self.channels.concat()
    }

    /// Split a planar buffer back into channels.
    pub fn from_planar(sample_rate: u32, data: &[f32], channels: usize) -> Self {
        let frames = data.len() / channels;
        let channels = (0..channels).map(|c| data[c * frames..(c + 1) * frames].to_vec()).collect();
        Self { sample_rate, channels }
    }
}

/// Read any PCM (16/24/32-bit int) or 32-bit float WAV as f32.
pub fn read(path: impl AsRef<Path>) -> Result<Wav, hound::Error> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>()?
        }
    };
    let frames = interleaved.len() / ch;
    let channels = (0..ch).map(|c| (0..frames).map(|i| interleaved[i * ch + c]).collect()).collect();
    Ok(Wav { sample_rate: spec.sample_rate, channels })
}

/// Write 32-bit float WAV.
pub fn write(path: impl AsRef<Path>, wav: &Wav) -> Result<(), hound::Error> {
    let spec = hound::WavSpec {
        channels: wav.channels.len() as u16,
        sample_rate: wav.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for i in 0..wav.frames() {
        for c in &wav.channels {
            w.write_sample(c[i])?;
        }
    }
    w.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_16_bit_pcm_scaled() {
        let path = std::env::temp_dir().join(format!("hearo-wav16-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for s in [i16::MIN, 16384, 0] {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
        assert_eq!(read(&path).unwrap().channels, vec![vec![-1.0, 0.5, 0.0]]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn planar_roundtrip_including_empty() {
        let w = Wav::from_planar(48_000, &[1.0, 2.0, 3.0, 4.0], 2);
        assert_eq!(w.channels, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
        assert_eq!(w.to_planar(), vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(Wav::from_planar(48_000, &[], 2).frames(), 0);
    }

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("hearo-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        let wav = Wav { sample_rate: 44_100, channels: vec![vec![0.1, -0.2, 0.3], vec![0.0, 0.5, -1.0]] };
        write(&path, &wav).unwrap();
        assert_eq!(read(&path).unwrap(), wav);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
