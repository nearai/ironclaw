//! `ironclaw-silk-decoder` — reads raw SILK v3 bytes on stdin, writes a WAV-wrapped
//! 16-bit little-endian mono PCM stream on stdout.
//!
//! Isolated here so the main IronClaw build does not need libclang. The host
//! invokes this binary as a subprocess; if it isn't installed, WeChat voice
//! notes simply remain `audio/silk` blobs (graceful degradation).
//!
//! Usage:
//!   ironclaw-silk-decoder [--sample-rate 24000]

use std::io::{self, Read, Write};
use std::process::ExitCode;

use silk_codec::decode_silk;

const DEFAULT_SAMPLE_RATE_HZ: i32 = 24_000;
const MIN_SAMPLE_RATE_HZ: i32 = 8_000;
const MAX_SAMPLE_RATE_HZ: i32 = 48_000;

/// Cap on input SILK bytes — matches the host attachment-size cap (20 MiB).
const MAX_INPUT_BYTES: usize = 20 * 1024 * 1024;
/// Cap on output PCM. SILK→PCM expands ~25× at 24 kHz mono; 60 s of voice
/// produces ~3 MiB. 50 MiB allows generous headroom while preventing
/// decompression-bomb behavior.
const MAX_PCM_BYTES: usize = 50 * 1024 * 1024;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sample_rate = match parse_sample_rate(&args) {
        Ok(rate) => rate,
        Err(message) => {
            eprintln!("ironclaw-silk-decoder: {message}");
            return ExitCode::from(2);
        }
    };

    let input = match read_bounded_stdin(MAX_INPUT_BYTES) {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("ironclaw-silk-decoder: {message}");
            return ExitCode::from(1);
        }
    };
    if input.is_empty() {
        eprintln!("ironclaw-silk-decoder: empty input on stdin");
        return ExitCode::from(1);
    }

    let pcm = match decode_silk(&input, sample_rate) {
        Ok(pcm) => pcm,
        Err(error) => {
            eprintln!("ironclaw-silk-decoder: SILK decode failed: {error}");
            return ExitCode::from(3);
        }
    };
    if pcm.is_empty() {
        eprintln!("ironclaw-silk-decoder: SILK decoder returned empty PCM");
        return ExitCode::from(3);
    }
    if pcm.len() > MAX_PCM_BYTES {
        eprintln!(
            "ironclaw-silk-decoder: decoded PCM exceeds {MAX_PCM_BYTES} bytes ({} bytes)",
            pcm.len()
        );
        return ExitCode::from(3);
    }

    let wav = match pcm_s16le_to_wav(&pcm, sample_rate as u32) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("ironclaw-silk-decoder: {error}");
            return ExitCode::from(3);
        }
    };

    let mut stdout = io::stdout().lock();
    if let Err(error) = stdout.write_all(&wav) {
        eprintln!("ironclaw-silk-decoder: failed to write WAV to stdout: {error}");
        return ExitCode::from(1);
    }
    if let Err(error) = stdout.flush() {
        eprintln!("ironclaw-silk-decoder: failed to flush stdout: {error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn parse_sample_rate(args: &[String]) -> Result<i32, String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--sample-rate" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--sample-rate requires a value".to_string())?;
                let parsed: i32 = raw
                    .parse()
                    .map_err(|e| format!("--sample-rate '{raw}' is not an integer: {e}"))?;
                if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&parsed) {
                    return Err(format!(
                        "--sample-rate must be between {MIN_SAMPLE_RATE_HZ} and {MAX_SAMPLE_RATE_HZ} Hz, got {parsed}"
                    ));
                }
                return Ok(parsed);
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: ironclaw-silk-decoder [--sample-rate HZ]\n\nReads raw SILK v3 bytes on stdin, writes 16-bit mono PCM\nwrapped in a WAV container on stdout."
                );
                return Err("help".to_string());
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(DEFAULT_SAMPLE_RATE_HZ)
}

fn read_bounded_stdin(cap: usize) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    let stdin = io::stdin();
    let mut handle = stdin.lock().take((cap as u64).saturating_add(1));
    handle
        .read_to_end(&mut buffer)
        .map_err(|e| format!("failed to read stdin: {e}"))?;
    if buffer.len() > cap {
        return Err(format!("input exceeds {cap} bytes"));
    }
    Ok(buffer)
}

fn pcm_s16le_to_wav(pcm: &[u8], sample_rate_hz: u32) -> Result<Vec<u8>, String> {
    if !pcm.len().is_multiple_of(2) {
        return Err("PCM buffer length must be even for 16-bit mono audio".to_string());
    }

    let data_len = u32::try_from(pcm.len())
        .map_err(|_| "PCM buffer exceeds WAV container size limits".to_string())?;
    let total_len = 44u32
        .checked_add(data_len)
        .ok_or_else(|| "WAV container size overflowed".to_string())?;
    let byte_rate = sample_rate_hz
        .checked_mul(2)
        .ok_or_else(|| "WAV byte rate overflowed".to_string())?;

    let mut wav = Vec::with_capacity(total_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(total_len - 8).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate_hz.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SAMPLE_RATE_HZ, MAX_SAMPLE_RATE_HZ, MIN_SAMPLE_RATE_HZ, parse_sample_rate,
        pcm_s16le_to_wav,
    };

    #[test]
    fn parse_sample_rate_default_when_no_flag() {
        assert_eq!(parse_sample_rate(&[]).unwrap(), DEFAULT_SAMPLE_RATE_HZ);
    }

    #[test]
    fn parse_sample_rate_accepts_explicit_value() {
        let args = vec!["--sample-rate".to_string(), "16000".to_string()];
        assert_eq!(parse_sample_rate(&args).unwrap(), 16_000);
    }

    #[test]
    fn parse_sample_rate_rejects_out_of_range() {
        let too_low = vec!["--sample-rate".to_string(), "1000".to_string()];
        assert!(parse_sample_rate(&too_low).is_err());
        let too_high = vec![
            "--sample-rate".to_string(),
            (MAX_SAMPLE_RATE_HZ + 1).to_string(),
        ];
        assert!(parse_sample_rate(&too_high).is_err());
        let _ = MIN_SAMPLE_RATE_HZ;
    }

    #[test]
    fn parse_sample_rate_rejects_unknown_arg() {
        let args = vec!["--what".to_string()];
        assert!(parse_sample_rate(&args).is_err());
    }

    #[test]
    fn pcm_s16le_to_wav_writes_riff_wave_header() {
        let wav = pcm_s16le_to_wav(&[0x00, 0x00, 0x01, 0x00], 24_000).expect("wav wrap");
        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(&wav[40..44], &(4u32).to_le_bytes());
        assert_eq!(&wav[44..], &[0x00, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn pcm_s16le_to_wav_rejects_odd_length() {
        assert!(pcm_s16le_to_wav(&[0x00], 24_000).is_err());
    }
}
