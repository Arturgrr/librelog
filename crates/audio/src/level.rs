//! A microphone's level, for meters that show it hears the person. Measured
//! around each window's mean, as Hex's input level (MIT, Christopher
//! Langton), so a microphone with a DC offset never reads as loud.

/// RMS and peak of `samples` around their mean; samples that are not finite
/// are left out.
pub fn measure(samples: &[f32]) -> (f32, f32) {
    let finite = || samples.iter().copied().filter(|sample| sample.is_finite());
    let count = finite().count();
    if count == 0 {
        return (0.0, 0.0);
    }
    let mean = finite().map(f64::from).sum::<f64>() / count as f64;
    let (squares, peak) = finite().fold((0.0_f64, 0.0_f64), |(squares, peak), sample| {
        let centered = f64::from(sample) - mean;
        (squares + centered * centered, peak.max(centered.abs()))
    });
    let rms = (squares / count as f64).sqrt();
    (rms.min(1.0) as f32, peak.min(1.0) as f32)
}

/// A peak from 0 to 1 as a meter's fill, over the 60 dB below full scale, as
/// Settings' input level shows it.
pub fn meter(peak: f32) -> f32 {
    if peak <= 0.0 || !peak.is_finite() {
        return 0.0;
    }
    ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_measured_around_the_mean() {
        let (rms, peak) = measure(&[0.7, -0.3]);
        assert!((rms - 0.5).abs() < 1e-6, "{rms}");
        assert!((peak - 0.5).abs() < 1e-6, "{peak}");
        let (rms, peak) = measure(&[0.2; 512]);
        assert!(rms < 1e-6 && peak < 1e-6, "{rms} {peak}");
        assert_eq!(measure(&[f32::NAN]), (0.0, 0.0));
    }
}
