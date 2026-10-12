//! Dependency-free offline renderer for PCM/IEEE-float RIFF WAV test signals.
use reveler_dsp::{Parameters, Processor};
use std::{env, error::Error, fs, io};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn u16_at(x: &[u8], p: usize) -> u16 {
    u16::from_le_bytes(x[p..p + 2].try_into().unwrap())
}
fn u32_at(x: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(x[p..p + 4].try_into().unwrap())
}

fn read_wav(path: &str) -> Result<(u32, usize, Vec<f32>), Box<dyn Error>> {
    let b = fs::read(path)?;
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(invalid("expected RIFF WAVE").into());
    }
    let mut fmt = None;
    let mut data = None;
    let mut pos = 12;
    while pos + 8 <= b.len() {
        let size = u32_at(&b, pos + 4) as usize;
        let end = pos + 8 + size;
        if end > b.len() {
            return Err(invalid("truncated WAV chunk").into());
        }
        match &b[pos..pos + 4] {
            b"fmt " => fmt = Some(&b[pos + 8..end]),
            b"data" => data = Some(&b[pos + 8..end]),
            _ => (),
        }
        pos = end + (size & 1);
    }
    let fmt = fmt.ok_or_else(|| invalid("missing fmt chunk"))?;
    let data = data.ok_or_else(|| invalid("missing data chunk"))?;
    if fmt.len() < 16 {
        return Err(invalid("truncated format").into());
    }
    let mut tag = u16_at(fmt, 0);
    let channels = u16_at(fmt, 2) as usize;
    let rate = u32_at(fmt, 4);
    let align = u16_at(fmt, 12) as usize;
    let bits = u16_at(fmt, 14);
    if tag == 0xfffe {
        if fmt.len() < 40 {
            return Err(invalid("truncated extensible format").into());
        }
        tag = u16_at(fmt, 24);
    }
    let bytes = usize::from(bits / 8);
    if !matches!((tag, bits), (1, 8 | 16 | 24 | 32) | (3, 32 | 64))
        || channels == 0
        || align != channels * bytes
        || data.len() % align != 0
    {
        return Err(invalid("supported formats: interleaved PCM 8/16/24/32 or float 32/64").into());
    }
    let mut samples = Vec::with_capacity(data.len() / bytes);
    for x in data.chunks_exact(bytes) {
        let v = match (tag, bits) {
            (1, 8) => (x[0] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes(x.try_into().unwrap()) as f32 / 32768.0,
            (1, 24) => ((i32::from_le_bytes([x[0], x[1], x[2], 0]) << 8) >> 8) as f32 / 8388608.0,
            (1, 32) => i32::from_le_bytes(x.try_into().unwrap()) as f32 / 2147483648.0,
            (3, 32) => f32::from_le_bytes(x.try_into().unwrap()),
            (3, 64) => f64::from_le_bytes(x.try_into().unwrap()) as f32,
            _ => unreachable!(),
        };
        samples.push(v);
    }
    Ok((rate, channels, samples))
}
fn write_wav(
    path: &str,
    rate: u32,
    channels: usize,
    samples: &[f32],
) -> Result<(), Box<dyn Error>> {
    let bytes = u32::try_from(samples.len() * 4)?;
    let mut b = Vec::with_capacity(bytes as usize + 56);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(bytes + 48).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16_u32.to_le_bytes());
    b.extend_from_slice(&3_u16.to_le_bytes());
    b.extend_from_slice(&(channels as u16).to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * channels as u32 * 4).to_le_bytes());
    b.extend_from_slice(&(channels as u16 * 4).to_le_bytes());
    b.extend_from_slice(&32_u16.to_le_bytes());
    b.extend_from_slice(b"fact");
    b.extend_from_slice(&4_u32.to_le_bytes());
    b.extend_from_slice(&(samples.len() as u32 / channels as u32).to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&bytes.to_le_bytes());
    for x in samples {
        b.extend_from_slice(&x.to_le_bytes());
    }
    fs::write(path, b)?;
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() < 2 {
        return Err(invalid("usage: cargo run -p reveler-dsp --release --example render -- input.wav output.wav [--trim-latency] [target=-20 peak=-1 max_gain=10 freeze=-40 input=-23 high=100 low=100 mix=0 lfe=0 bypass=false]").into());
    }
    let mut p = Parameters::default();
    let mut trim = false;
    for arg in &args[2..] {
        if arg == "--trim-latency" {
            trim = true;
            continue;
        }
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| invalid("expected key=value"))?;
        match key {
            "target" => p.target_lufs = value.parse()?,
            "peak" => p.true_peak_db = value.parse()?,
            "max_gain" => p.max_gain_db = value.parse()?,
            "freeze" => p.freeze_lufs = value.parse()?,
            "input" => p.input_lufs = value.parse()?,
            "high" => p.correction_high = value.parse::<f64>()? / 100.0,
            "low" => p.correction_low = value.parse::<f64>()? / 100.0,
            "mix" => p.mix_mode = value.parse()?,
            "lfe" => p.lfe_gain_db = value.parse()?,
            "bypass" => p.bypass = value.parse()?,
            _ => return Err(invalid("unknown parameter").into()),
        }
    }
    let (rate, channels, mut samples) = read_wav(&args[0])?;
    let mut dsp = Processor::new(rate as f64, channels).map_err(invalid)?;
    dsp.set_parameters(p);
    dsp.reset();
    let latency = dsp.latency_samples();
    let input_samples = samples.len();
    // The FIR is a detector only; audio needs exactly the reported
    // delay flushed, without an additional convolution tail.
    samples.resize(input_samples + latency * channels, 0.0);
    let start = std::time::Instant::now();
    for frame in samples.chunks_exact_mut(channels) {
        dsp.process_frame(frame);
    }
    let samples = if trim {
        &samples[latency * channels..latency * channels + input_samples]
    } else {
        &samples[..]
    };
    write_wav(&args[1], rate, channels, samples)?;
    eprintln!(
        "Rendered {} channels at {} Hz; latency {} samples; processing {:.3}s; detector {:.2} LUFS, gain {:.2} dB, limiting {:.2} dB",
        channels,
        rate,
        latency,
        start.elapsed().as_secs_f64(),
        dsp.meters().input_lufs,
        dsp.meters().gain_db,
        dsp.meters().limiter_reduction_db
    );
    Ok(())
}
