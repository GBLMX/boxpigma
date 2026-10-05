//! The chain the samples pass through on their way to the device: sample-rate conversion, the
//! parametric EQ and loudness normalization.
//!
//! This is not [`super::dsp`], which is the *analysis* side — the spectrum bars and the pitch
//! estimator. These adapters change what is heard; those only look at it.
//!
//! Each stage is a `rodio::Source` adapter over the decoded samples, so the player's chain
//! reads as what it does, and a stage with nothing to do is not built at all: with the device
//! running at the file's rate and no EQ or normalization configured, the decoded samples reach
//! the sink untouched — the bit-perfect path.

use std::{
    num::NonZero,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type};
use ebur128::{EbuR128, Mode};
use rodio::{ChannelCount, Sample, SampleRate, Source};
use rubato::{Fft, FixedSync, Resampler as _};

/// Frames pulled from the inner source per call.
///
/// rubato is built around chunks of a few hundred to a few thousand frames; this keeps the
/// added latency down around what the device buffers anyway, without running the FFT on
/// something so small that its overhead dominates.
const CHUNK: usize = 1024;

/// The samples the loudness meter looks at in one go. Momentary loudness is a 400 ms window
/// internally, so a tenth of a second per look is fine-grained enough to follow a track
/// without measuring more often than the answer can change.
const LOUDNESS_BLOCK_MS: u32 = 100;

/// A source of decoded samples on its way to the device. Boxed because each stage is optional
/// and the player only sees the ends of the chain.
pub(super) type Processed = Box<dyn Source<Item = Sample> + Send>;

/// Wrap `source` with the stages the configuration asks for, in the order they act:
/// sample-rate conversion, then the EQ, then loudness.
///
/// A stage with nothing to do is not built — no EQ bands means no filter, no loudness target
/// means no meter, and a file already at the device's rate means no converter — so an
/// unconfigured chain is the decoder's own samples, untouched.
pub(super) fn build(
    source: Processed,
    device_rate: u32,
    config: &crate::config::AudioConfig,
) -> Processed {
    let mut source = source;

    if config.resample {
        source = match Resample::new(source, device_rate) {
            Ok(resampled) => Box::new(resampled),
            Err(untouched) => untouched,
        };
    }

    let rate = source.sample_rate().get();
    let channels = source.channels().get();
    if !config.eq.is_empty() {
        source = Box::new(Equalizer::new(source, &config.eq, rate, channels));
    }
    if let Some(loudness) = config.loudness {
        source = Box::new(Loudness::new(source, loudness, rate, channels));
    }

    source
}

/// Sample-rate conversion, from the file's rate to the device's.
///
/// `rubato`'s synchronous FFT resampler: the ratio is fixed for the life of a song, which is
/// exactly the case here, and the filter it applies is what keeps a converted recording from
/// aliasing where the backend's own converter would not care.
pub(super) struct Resample<S> {
    inner: S,
    resampler: Fft<f32>,
    channels: usize,
    /// Interleaved input for one chunk: `CHUNK` frames, zero-padded at the end of the file.
    input: Vec<f32>,
    /// The same chunk, per channel — what rubato's adapter reads.
    planar_in: Vec<Vec<f32>>,
    planar_out: Vec<Vec<f32>>,
    /// Interleaved output of the last call, handed out one sample at a time.
    pending: Vec<f32>,
    cursor: usize,
    out_rate: u32,
    /// The inner source ran out and the tail has been flushed; nothing more will come.
    finished: bool,
    /// Set once the tail has been padded and resampled, so the flush happens exactly once.
    flushed: bool,
}

impl<S: Source<Item = Sample>> Resample<S> {
    /// Wrap `inner`, or hand it straight back when there is nothing to do — the device already
    /// runs at the file's rate, or it has no rate to convert to. Getting the source back is
    /// what keeps a matching file bit-perfect: no adapter is built at all.
    pub(super) fn new(inner: S, out_rate: u32) -> Result<Self, S> {
        let in_rate = inner.sample_rate().get();
        let channels = usize::from(inner.channels().get());
        if in_rate == out_rate || channels == 0 || out_rate == 0 {
            return Err(inner);
        }
        let Ok(resampler) = Fft::<f32>::new(
            in_rate as usize,
            out_rate as usize,
            CHUNK,
            channels,
            FixedSync::Input,
        ) else {
            return Err(inner);
        };
        let out_max = resampler.output_frames_max();
        Ok(Self {
            inner,
            resampler,
            channels,
            input: vec![0.0; CHUNK * channels],
            planar_in: vec![vec![0.0; CHUNK]; channels],
            planar_out: vec![vec![0.0; out_max]; channels],
            pending: Vec::with_capacity(out_max * channels),
            cursor: 0,
            out_rate,
            finished: false,
            flushed: false,
        })
    }

    /// Read one chunk from the inner source and resample it. Returns whether anything was
    /// produced; the tail of a file is padded with silence so the resampler's own delay drains
    /// rather than being cut off.
    fn refill(&mut self) {
        let frames = CHUNK;
        self.input.resize(frames * self.channels, 0.0);
        self.input.fill(0.0);

        let mut read = 0usize;
        while read < frames {
            match self.inner.next() {
                Some(sample) => self.input[read * self.channels] = sample,
                None => break,
            }
            // The rest of the frame's channels come straight after it.
            for channel in 1..self.channels {
                // `next` yields interleaved samples, so the frame is filled in order; a source
                // that ends mid-frame simply leaves zeros behind.
                if let Some(sample) = self.inner.next() {
                    self.input[read * self.channels + channel] = sample;
                }
            }
            read += 1;
        }

        if read == 0 {
            if self.flushed {
                self.finished = true;
                self.pending.clear();
                self.cursor = 0;
                return;
            }
            // One padded chunk flushes the resampler's delay; after that there is nothing left.
            self.flushed = true;
        } else if read < frames {
            self.flushed = true;
        }

        for channel in 0..self.channels {
            for frame in 0..frames {
                self.planar_in[channel][frame] = self.input[frame * self.channels + channel];
            }
        }

        let input = rubato::audioadapter_buffers::direct::SequentialSliceOfVecs::new(
            &self.planar_in,
            self.channels,
            frames,
        );
        let output = rubato::audioadapter_buffers::direct::SequentialSliceOfVecs::new_mut(
            &mut self.planar_out,
            self.channels,
            self.resampler.output_frames_max(),
        );
        let (Ok(input), Ok(mut output)) = (input, output) else {
            self.finished = true;
            return;
        };
        let produced = match self
            .resampler
            .process_into_buffer(&input, &mut output, None)
        {
            Ok((_consumed, produced)) => produced,
            Err(error) => {
                log::warn!("resampler: {error}");
                self.finished = true;
                self.pending.clear();
                self.cursor = 0;
                return;
            }
        };

        self.pending.clear();
        self.pending.reserve(produced * self.channels);
        for frame in 0..produced {
            for channel in 0..self.channels {
                self.pending.push(self.planar_out[channel][frame]);
            }
        }
        self.cursor = 0;
    }
}

impl<S: Source<Item = Sample>> Iterator for Resample<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        loop {
            if self.cursor < self.pending.len() {
                let sample = self.pending[self.cursor];
                self.cursor += 1;
                return Some(sample);
            }
            if self.finished {
                return None;
            }
            self.refill();
        }
    }
}

impl<S: Source<Item = Sample>> Source for Resample<S> {
    /// `f32` samples carry no span of their own; the conversion is continuous.
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(self.channels as u16).expect("a resampler is built with channels")
    }

    fn sample_rate(&self) -> SampleRate {
        NonZero::new(self.out_rate).expect("a resampler is built with a rate")
    }

    /// The duration is the file's: the conversion changes how many frames carry it, not how
    /// long the recording is.
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(position)?;
        // The resampler carries filter state across chunks, so the seeked-to samples would
        // otherwise be filtered with the old position's tail.
        self.resampler.reset();
        self.pending.clear();
        self.cursor = 0;
        self.finished = false;
        self.flushed = false;
        Ok(())
    }
}

/// The parametric EQ: one peaking filter per band per channel.
///
/// A peaking filter is what "3 dB down at 105 Hz" means; the state has to be kept per channel,
/// because a single shared filter would mix the left channel's history into the right one.
pub(super) struct Equalizer<S> {
    inner: S,
    filters: Vec<DirectForm1<f32>>,
    channels: usize,
    rate: u32,
    /// Which channel the next sample belongs to.
    channel: usize,
}

impl<S: Source<Item = Sample>> Equalizer<S> {
    fn new(inner: S, bands: &[crate::config::EqBand], rate: u32, channels: u16) -> Self {
        let channels = usize::from(channels);
        let mut filters = Vec::with_capacity(bands.len() * channels);
        for band in bands {
            let Ok(coefficients) = Coefficients::<f32>::from_params(
                Type::PeakingEQ(band.gain_db),
                rate.hz(),
                band.freq.hz(),
                band.q,
            ) else {
                // A band the filter maths cannot express (a frequency above Nyquist, a zero
                // width) is one band missing rather than a track that does not play.
                log::warn!(
                    "eq: skipping {} Hz ({} dB, q {})",
                    band.freq,
                    band.gain_db,
                    band.q
                );
                continue;
            };
            for _ in 0..channels {
                filters.push(DirectForm1::<f32>::new(coefficients));
            }
        }
        Self {
            inner,
            filters,
            channels,
            rate,
            channel: 0,
        }
    }
}

impl<S: Source<Item = Sample>> Iterator for Equalizer<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let sample = self.inner.next()?;
        if self.filters.is_empty() {
            return Some(sample);
        }
        // The bands are chained: the output of one is the input of the next, for this channel.
        let mut value = sample;
        let base = self.channel.min(self.channels - 1);
        for band in 0..self.filters.len() / self.channels {
            value = self.filters[band * self.channels + base].run(value);
        }
        self.channel = (self.channel + 1) % self.channels;
        Some(value)
    }
}

impl<S: Source<Item = Sample>> Source for Equalizer<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        NonZero::new(self.rate).expect("the inner source has a rate")
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(position)?;
        // The filters hold the old position's tail; a fresh filter starts clean.
        for filter in &mut self.filters {
            filter.reset_state();
        }
        self.channel = 0;
        Ok(())
    }
}

/// Loudness normalization (EBU R128): measure the momentary loudness and pull the gain toward
/// the configured target, a block at a time.
///
/// The gain is smoothed rather than set per block: a level that jumps by 10 dB between blocks
/// is audible as pumping, which is worse than the difference it is correcting.
pub(super) struct Loudness<S> {
    inner: S,
    meter: EbuR128,
    channels: usize,
    /// The gain being applied, linear. Starts at 1.0 and walks toward the target.
    gain: f32,
    target_lufs: f64,
    max_gain: f32,
    block: Vec<f32>,
    cursor: usize,
    finished: bool,
}

impl<S: Source<Item = Sample>> Loudness<S> {
    fn new(inner: S, config: crate::config::LoudnessConfig, rate: u32, channels: u16) -> Self {
        let channels = usize::from(channels);
        let meter = EbuR128::new(channels as u32, rate, Mode::M).ok();
        let frames = (rate / 1000 * LOUDNESS_BLOCK_MS).max(1) as usize;
        Self {
            inner,
            meter: match meter {
                Some(meter) => meter,
                // Without a meter the stage is a pass-through: the samples still play, the
                // level is just not corrected.
                None => {
                    log::warn!("loudness: no meter at {rate} Hz, {channels} channels");
                    EbuR128::new(1, rate.max(1), Mode::M).expect("a meter for one channel")
                }
            },
            channels,
            gain: 1.0,
            target_lufs: f64::from(config.target_lufs),
            max_gain: 10f32.powf(config.max_gain_db / 20.0),
            block: vec![0.0; frames * channels],
            cursor: 0,
            finished: false,
        }
    }

    /// Read one block from the inner source, measure it, and apply the gain it implies.
    fn refill(&mut self) {
        let mut read = 0;
        while read < self.block.len() {
            match self.inner.next() {
                Some(sample) => {
                    self.block[read] = sample;
                    read += 1;
                }
                None => break,
            }
        }
        if read == 0 {
            self.finished = true;
            self.block.clear();
            self.cursor = 0;
            return;
        }
        self.block.truncate(read);

        if self.meter.add_frames_f32(&self.block).is_ok()
            && let Ok(loudness) = self.meter.loudness_momentary()
            // Below -70 LUFS the window is silence or near it; correcting for that would just
            // amplify the noise floor between tracks.
            && loudness > -70.0
        {
            let target = 10f64.powf((self.target_lufs - loudness) / 20.0) as f32;
            let wanted = target.min(self.max_gain);
            // A tenth of the way per block: about a second to settle, which is slower than a
            // note and faster than a track.
            self.gain += (wanted - self.gain) * 0.1;
        }

        for sample in &mut self.block {
            *sample *= self.gain;
        }
        self.cursor = 0;
    }
}

impl<S: Source<Item = Sample>> Iterator for Loudness<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        loop {
            if self.cursor < self.block.len() {
                let sample = self.block[self.cursor];
                self.cursor += 1;
                return Some(sample);
            }
            if self.finished {
                return None;
            }
            self.refill();
            if self.finished && self.block.is_empty() {
                return None;
            }
        }
    }
}

impl<S: Source<Item = Sample>> Source for Loudness<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(position)?;
        // The measured loudness belongs to the old position, and the gain it produced should
        // not be applied to the new one before it has been measured.
        self.gain = 1.0;
        if let Ok(fresh) = EbuR128::new(
            self.channels as u32,
            self.inner.sample_rate().get(),
            Mode::M,
        ) {
            self.meter = fresh;
        }
        self.block.clear();
        self.cursor = 0;
        self.finished = false;
        Ok(())
    }
}

/* -------------------------------------------------------------------------- */
/*                                  gain ramp                                */
/* -------------------------------------------------------------------------- */

/// How long a gain change takes, in milliseconds.
///
/// Every change of level — a volume step, pausing, resuming, seeking, switching songs — is
/// a step in the waveform where the samples used to jump from one amplitude to another, and
/// a step in the waveform is a click. Walking to the new level instead of jumping to it is
/// what the audio world calls a fade; 12 ms is short enough that pausing and the volume keys
/// still feel immediate, long enough to be inaudible as a glide.
pub(super) const GAIN_FADE_MS: u32 = 12;

/// Samples between two updates of the published current gain.
///
/// The control loop waits for the ramp to reach zero before it pauses or switches the
/// stream (see `fade_out` in [`super::player`]), and that wait only needs to be accurate to
/// a holler, so the audio thread publishes its position every few dozen samples rather than
/// taking a lock or an atomic write per sample.
const GAIN_PUBLISH_SAMPLES: u32 = 64;

/// The level of one stream, shared between the audio thread that applies it and the control
/// loop that asks for a change.
///
/// Two numbers, both `f32` in an atomic: the level being asked for (the user's volume, or
/// zero while pausing) and where the ramp has actually got to. The second one is what makes
/// "fade out, then pause" possible — pausing on the spot would cut the samples mid-wave and
/// undo the fade.
#[derive(Clone, Debug)]
pub(super) struct GainHandle {
    target: Arc<AtomicU32>,
    current: Arc<AtomicU32>,
}

impl GainHandle {
    /// A handle whose ramp starts at `initial` and walks toward `target` — a fresh stream
    /// starts at zero so it fades in rather than starting mid-amplitude.
    pub(super) fn new(initial: f32, target: f32) -> Self {
        Self {
            target: Arc::new(AtomicU32::new(target.to_bits())),
            current: Arc::new(AtomicU32::new(initial.to_bits())),
        }
    }

    /// Ask for a new level; the ramp walks there at its own speed.
    pub(super) fn set_target(&self, gain: f32) {
        self.target.store(gain.to_bits(), Ordering::Relaxed);
    }

    fn target(&self) -> f32 {
        f32::from_bits(self.target.load(Ordering::Relaxed))
    }

    /// Where the ramp has got to, as last published by the audio thread.
    pub(super) fn current(&self) -> f32 {
        f32::from_bits(self.current.load(Ordering::Relaxed))
    }
}

/// The gain stage: multiplies every sample by a level that walks toward the handle's target.
///
/// This is the stream's only volume — the sink's own volume stays at 1.0 so there is one
/// place where the level changes, and that place ramps.
pub(super) struct Gain<S> {
    inner: S,
    handle: GainHandle,
    /// The level this stage is currently at; the audio thread's copy of `handle.current`.
    value: f32,
    /// How much the level moves per sample: a full 0..1 walk takes [`GAIN_FADE_MS`].
    step: f32,
    /// Samples left before the next publication of `value`.
    until_publish: u32,
}

impl<S: Source<Item = Sample>> Gain<S> {
    pub(super) fn new(inner: S, handle: GainHandle) -> Self {
        let samples_per_fade = f64::from(GAIN_FADE_MS)
            * f64::from(inner.sample_rate().get())
            * f64::from(inner.channels().get())
            / 1000.0;
        Self {
            value: handle.current(),
            step: if samples_per_fade >= 1.0 {
                (1.0 / samples_per_fade) as f32
            } else {
                1.0
            },
            inner,
            handle,
            until_publish: 0,
        }
    }
}

impl<S: Source<Item = Sample>> Iterator for Gain<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let sample = self.inner.next()?;
        // The level is applied *before* it moves, so the very first sample of a stream is
        // exactly the initial level — silence on a fresh stream, which is the point.
        let out = sample * self.value;

        let target = self.handle.target();
        let walking = self.value != target;
        if self.value < target {
            self.value = (self.value + self.step).min(target);
        } else if self.value > target {
            self.value = (self.value - self.step).max(target);
        }

        // Publish on arrival as well as periodically: whoever is waiting for the ramp to
        // reach silence (see `fade_out`) should not have to wait out the rest of an interval.
        let arrived = walking && self.value == target;
        if arrived || self.until_publish == 0 {
            self.until_publish = GAIN_PUBLISH_SAMPLES;
            self.handle
                .current
                .store(self.value.to_bits(), Ordering::Relaxed);
        } else {
            self.until_publish -= 1;
        }
        Some(out)
    }
}

impl<S: Source<Item = Sample>> Source for Gain<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        // The level belongs to the stream, not to the position in it: seeking inside one
        // stream keeps the current level instead of fading in again.
        self.inner.try_seek(position)
    }
}

/// Wrap `source` in the gain stage driven by `handle`.
pub(super) fn gain(source: Processed, handle: GainHandle) -> Processed {
    Box::new(Gain::new(source, handle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source of `frames` frames of silence, so the resampler can be exercised
    /// without a decoder or a device.
    #[derive(Debug)]
    struct Silence {
        frames: usize,
        channels: ChannelCount,
        rate: SampleRate,
        left: usize,
    }

    impl Silence {
        fn new(frames: usize, channels: u16, rate: u32) -> Self {
            Self {
                frames,
                channels: NonZero::new(channels).expect("channels"),
                rate: NonZero::new(rate).expect("rate"),
                left: frames * usize::from(channels),
            }
        }
    }

    impl Iterator for Silence {
        type Item = Sample;

        fn next(&mut self) -> Option<Sample> {
            if self.left == 0 {
                return None;
            }
            self.left -= 1;
            Some(0.0)
        }
    }

    impl Source for Silence {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> ChannelCount {
            self.channels
        }
        fn sample_rate(&self) -> SampleRate {
            self.rate
        }
        fn total_duration(&self) -> Option<Duration> {
            Some(Duration::from_secs_f64(
                self.frames as f64 / f64::from(self.rate.get()),
            ))
        }
    }

    /// A sine at `freq` Hz, `frames` frames of it.
    #[derive(Debug)]
    struct Tone {
        freq: f64,
        amplitude: f32,
        phase: f64,
        rate: SampleRate,
        channels: ChannelCount,
        left: usize,
    }

    impl Tone {
        fn new(freq: f64, amplitude: f32, seconds: f64, rate: u32, channels: u16) -> Self {
            let rate_nz = NonZero::new(rate).expect("rate");
            let frames = (seconds * f64::from(rate)) as usize;
            Self {
                freq,
                amplitude,
                phase: 0.0,
                rate: rate_nz,
                channels: NonZero::new(channels).expect("channels"),
                left: frames * usize::from(channels),
            }
        }
    }

    impl Iterator for Tone {
        type Item = Sample;

        fn next(&mut self) -> Option<Sample> {
            if self.left == 0 {
                return None;
            }
            self.left -= 1;
            let channels = usize::from(self.channels.get());
            if self.left % channels == channels - 1 {
                self.phase += std::f64::consts::TAU * self.freq / f64::from(self.rate.get());
            }
            Some((self.phase.sin() as f32) * self.amplitude)
        }
    }

    impl Source for Tone {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> ChannelCount {
            self.channels
        }
        fn sample_rate(&self) -> SampleRate {
            self.rate
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// The settled part of a filtered signal: a biquad rings for a few cycles before its
    /// steady state, and the measurement is about the steady state.
    fn settled(samples: Vec<f32>) -> Vec<f32> {
        let skip = samples.len() / 4;
        samples.into_iter().skip(skip).collect()
    }

    /// A boost band lifts its own frequency and a cut band lowers it: what "3 dB at 100 Hz"
    /// has to mean for the reader who wrote it down.
    #[test]
    fn a_band_moves_only_its_own_frequency() {
        let band = |gain_db: f32| crate::config::EqBand {
            freq: 100.0,
            gain_db,
            q: 1.0,
        };
        let tone = || Tone::new(100.0, 0.5, 2.0, 48_000, 1);
        let flat: Vec<f32> = tone().collect();
        let flat = rms(&settled(flat));

        let boosted: Vec<f32> = Equalizer::new(tone(), &[band(6.0)], 48_000, 1).collect();
        let cut: Vec<f32> = Equalizer::new(tone(), &[band(-6.0)], 48_000, 1).collect();
        let boosted = rms(&settled(boosted));
        let cut = rms(&settled(cut));

        let boost_ratio = boosted / flat;
        let cut_ratio = cut / flat;
        assert!(
            (1.5..2.6).contains(&boost_ratio),
            "+6 dB should be about twice the amplitude, got {boost_ratio:.2}x"
        );
        assert!(
            (0.3..0.7).contains(&cut_ratio),
            "-6 dB should be about half the amplitude, got {cut_ratio:.2}x"
        );
    }

    /// A quiet track is lifted and a loud one is pulled down: the two directions of
    /// normalization, which is the whole point of having it.
    #[test]
    fn loudness_pulls_both_ends_toward_the_target() {
        let config = crate::config::LoudnessConfig {
            target_lufs: -14.0,
            max_gain_db: 20.0,
        };

        let quiet: Vec<f32> = Tone::new(1_000.0, 0.005, 4.0, 48_000, 1).collect();
        let baseline = rms(&quiet);
        let lifted_samples: Vec<f32> =
            Loudness::new(Tone::new(1_000.0, 0.005, 4.0, 48_000, 1), config, 48_000, 1).collect();
        let lifted = rms(&lifted_samples);
        assert!(
            lifted > baseline * 2.0,
            "a -46 dBFS tone should come up toward -14 LUFS: {baseline:.4} -> {lifted:.4}"
        );

        let loud_samples: Vec<f32> = Tone::new(1_000.0, 0.5, 4.0, 48_000, 1).collect();
        let loud = rms(&loud_samples);
        let pulled_samples: Vec<f32> =
            Loudness::new(Tone::new(1_000.0, 0.5, 4.0, 48_000, 1), config, 48_000, 1).collect();
        let pulled = rms(&pulled_samples);
        assert!(
            pulled < loud * 0.8,
            "a loud tone should come down: {loud:.4} -> {pulled:.4}"
        );
    }

    /// The conversion reports the device's rate, keeps the channel count, and produces about
    /// as many frames as its ratio says — which is the whole point of doing it here.
    #[test]
    fn a_converted_stream_runs_at_the_devices_rate() {
        let source = Silence::new(44_100, 2, 44_100);
        let resampled = Resample::new(source, 48_000).expect("rates differ");

        assert_eq!(resampled.sample_rate().get(), 48_000);
        assert_eq!(resampled.channels().get(), 2);

        let produced = resampled.count();
        let expected = 48_000 * 2;
        let frames = produced / 2;
        assert!(
            frames.abs_diff(48_000) < 2_000,
            "expected about {expected} samples, got {produced}"
        );
    }

    /// Nothing to convert is nothing added: the same file the device already runs at is not
    /// resampled, which is what keeps it bit-perfect.
    #[test]
    fn a_matching_rate_is_left_alone() {
        let source = Silence::new(1_000, 2, 48_000);
        assert!(
            Resample::new(source, 48_000).is_err(),
            "a matching rate comes straight back, unwrapped"
        );
    }

    /// A source that hands out `1.0` samples, so the samples that come out of the gain stage
    /// *are* the level it is applying.
    #[derive(Debug)]
    struct Ones {
        rate: SampleRate,
        channels: ChannelCount,
        left: usize,
    }

    impl Ones {
        fn new(frames: usize, channels: u16, rate: u32) -> Self {
            Self {
                rate: NonZero::new(rate).expect("rate"),
                channels: NonZero::new(channels).expect("channels"),
                left: frames * usize::from(channels),
            }
        }
    }

    impl Iterator for Ones {
        type Item = Sample;

        fn next(&mut self) -> Option<Sample> {
            if self.left == 0 {
                return None;
            }
            self.left -= 1;
            Some(1.0)
        }
    }

    impl Source for Ones {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> ChannelCount {
            self.channels
        }
        fn sample_rate(&self) -> SampleRate {
            self.rate
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    fn fade_samples(rate: u32, channels: u16) -> usize {
        (GAIN_FADE_MS * rate * u32::from(channels) / 1000) as usize
    }

    /// A stream that starts mid-amplitude is a click; a fresh one walks up from silence and
    /// lands exactly on the level it was asked for.
    #[test]
    fn a_fresh_stream_fades_in_and_settles() {
        let handle = GainHandle::new(0.0, 0.75);
        let samples: Vec<f32> = Gain::new(Ones::new(4_000, 2, 48_000), handle.clone()).collect();

        assert_eq!(
            samples[0], 0.0,
            "the first sample is silence, not the target"
        );
        for pair in samples.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "the walk up must not dip: {} -> {}",
                pair[0],
                pair[1]
            );
            assert!(
                pair[1] - pair[0] <= 2.0 / fade_samples(48_000, 2) as f32,
                "no jump larger than a step: {} -> {}",
                pair[0],
                pair[1]
            );
        }

        let settled = samples[fade_samples(48_000, 2)];
        assert!(
            (settled - 0.75).abs() < 1e-3,
            "one fade after the start the level is the target, got {settled}"
        );
        assert!(
            (samples[samples.len() - 1] - 0.75).abs() < 1e-6,
            "and it stays there: {}",
            samples[samples.len() - 1]
        );
    }

    /// What `fade_out` in the player depends on: asking for zero gets there, in one fade, and
    /// the published level says so.
    #[test]
    fn a_change_walks_to_the_new_level_and_reports_it() {
        let handle = GainHandle::new(1.0, 1.0);
        let mut stage = Gain::new(Ones::new(4_000, 2, 48_000), handle.clone());

        let settled: Vec<f32> = (0..64).map(|_| stage.next().unwrap()).collect();
        assert!(
            settled.iter().all(|s| (*s - 1.0).abs() < 1e-6),
            "already at the target"
        );

        handle.set_target(0.0);
        let ramp: Vec<f32> = (0..fade_samples(48_000, 2) + 2)
            .map(|_| stage.next().unwrap())
            .collect();

        for pair in ramp.windows(2) {
            assert!(
                pair[0] - pair[1] <= 2.0 / fade_samples(48_000, 2) as f32,
                "descending is also a ramp, not a cut: {} -> {}",
                pair[0],
                pair[1]
            );
            assert!(pair[1] <= pair[0], "and never rises on the way down");
        }
        assert_eq!(
            &ramp[ramp.len() - 2..],
            &[0.0, 0.0],
            "the walk ends at silence and stays there"
        );
        assert!(
            handle.current() < 1e-3,
            "the published level reaches silence within one fade, got {}",
            handle.current()
        );
    }

    /// A ramp that turned into an oscillation would hum; the level has to stop at its target.
    #[test]
    fn the_walk_stops_at_the_target() {
        let handle = GainHandle::new(0.0, 0.5);
        let samples: Vec<f32> = Gain::new(Ones::new(2_000, 1, 48_000), handle).collect();
        let after = &samples[fade_samples(48_000, 1)..];
        assert!(
            after.iter().all(|s| (*s - 0.5).abs() < 1e-6),
            "no ringing around the target: {:?}",
            &after[..8.min(after.len())]
        );
    }
}

/// `cargo test --release --lib -- --ignored --nocapture decoding_and_the_chain_cost`
///
/// What the samples cost on their way to the device. The other benches answer what the
/// *analysis* side and the frames cost; this is the account the listener pays for the sound
/// itself: symphonia decoding the file, then whatever `[audio]` asked the chain to do to it.
///
/// The number that means something is the ratio to the audio's own length: a chain that spends
/// 1% of a core per second of audio is one nobody can hear as a cost. A memory-source control
/// runs first, so the numbers can be read as "decoding and the chain" rather than "this loop".
#[cfg(test)]
mod chain_bench {
    use std::{
        fs,
        io::BufReader,
        num::NonZero,
        path::{Path, PathBuf},
        time::Instant,
    };

    use rodio::{Decoder, Source, buffer::SamplesBuffer};

    use crate::config::{AudioConfig, EqBand, LoudnessConfig};

    /// The device the chain is built for. 48 kHz is what this machine's sink reports and what a
    /// 44.1 kHz file has to be converted to; the file's own rate is measured, not assumed.
    const DEVICE_RATE: u32 = 48_000;

    /// The material: a real file, because decoding cost depends on the codec and on the file.
    /// `BOXPIGMA_BENCH_AUDIO` names one; otherwise the first MP3 of the user's own download
    /// cache. With neither, a printed line and a skip — the contract `scan_bench` has.
    fn material() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("BOXPIGMA_BENCH_AUDIO") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
            println!("  BOXPIGMA_BENCH_AUDIO 不是一个文件：{}", path.display());
        }

        let dir = crate::utils::boxpigma_cache_dir().join("downloads");
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("mp3"))
            })
            .collect();
        files.sort();
        files.into_iter().next()
    }

    fn decode(path: &Path) -> Option<Box<dyn Source<Item = f32> + Send>> {
        let file = fs::File::open(path).ok()?;
        Decoder::new(BufReader::new(file))
            .ok()
            .map(|decoder| Box::new(decoder) as Box<dyn Source<Item = f32> + Send>)
    }

    /// Drain a source one sample at a time, the way the sink pulls it, and hand back
    /// `(wall seconds, audio seconds, samples)`.
    fn drain(mut source: Box<dyn Source<Item = f32> + Send>) -> (f64, f64, u64) {
        let rate = f64::from(source.sample_rate().get());
        let channels = f64::from(source.channels().get());
        let start = Instant::now();
        let mut samples = 0u64;
        while source.next().is_some() {
            samples += 1;
        }
        let wall = start.elapsed().as_secs_f64();
        (wall, samples as f64 / (rate * channels), samples)
    }

    /// One line per case: cost per second of audio, and what share of one core that is when the
    /// audio is played in real time.
    fn report(label: &str, wall: f64, audio: f64, samples: u64) {
        let audio = audio.max(f64::EPSILON);
        println!(
            "  {label:<38} {:>8.2} µs/音频秒 → 实时占单核 {:>5.2}%   ({} 采样 / {:.0} 秒)",
            wall / audio * 1e6,
            wall / audio * 100.0,
            samples,
            audio,
        );
    }

    #[test]
    #[ignore = "measures decoding and the [audio] chain against a real file"]
    fn decoding_and_the_chain_cost() {
        let Some(path) = material() else {
            println!(
                "  没有可用的素材，跳过：把音频放进 {} 或用 BOXPIGMA_BENCH_AUDIO 指定",
                crate::utils::boxpigma_cache_dir()
                    .join("downloads")
                    .display()
            );
            return;
        };

        // The file's own shape, from a decode that is not timed. The length is *not* asked of
        // the container: a VBR MP3 without the header that carries it reports `None`, and a
        // header that lies is worse than no number — each line below counts its own samples.
        let Some(first) = decode(&path) else {
            println!("  解不开 {}，跳过", path.display());
            return;
        };
        let file_rate = first.sample_rate().get();
        let channels = first.channels().get();
        drop(first);
        println!(
            "  {}（{file_rate} Hz / {channels} 声道，设备 {DEVICE_RATE} Hz）:",
            path.file_name().unwrap_or_default().to_string_lossy(),
        );

        // The control: the same pull loop over samples already in memory. Everything below is
        // read against this floor.
        {
            let file = fs::File::open(&path).expect("open for the control");
            let decoded: Vec<f32> = Decoder::new(BufReader::new(file))
                .expect("decode for the control")
                .collect();
            let memory = SamplesBuffer::new(
                NonZero::new(channels).expect("channels"),
                NonZero::new(file_rate).expect("rate"),
                decoded,
            );
            let (wall, audio, samples) = drain(Box::new(memory));
            report("（对照）内存源空转", wall, audio, samples);
        }

        // The bit-perfect path: nothing is configured and the device runs at the file's rate,
        // so the sink is handed the decoder's own samples.
        let (wall, audio, samples) = drain(decode(&path).expect("decoded above"));
        report("解码（未配置 = 位完美路径）", wall, audio, samples);

        // …plus the conversion a device at another rate needs.
        let converting = AudioConfig {
            resample: true,
            ..AudioConfig::default()
        };
        let source = super::build(
            decode(&path).expect("decoded above"),
            DEVICE_RATE,
            &converting,
        );
        let (wall, audio, samples) = drain(source);
        report("解码 + 重采样", wall, audio, samples);

        // …plus everything `[audio]` can ask for: two EQ bands and a loudness target.
        let full = AudioConfig {
            resample: true,
            eq: vec![
                EqBand {
                    freq: 105.0,
                    gain_db: -3.0,
                    q: 1.0,
                },
                EqBand {
                    freq: 10_000.0,
                    gain_db: 2.0,
                    q: 0.7,
                },
            ],
            loudness: Some(LoudnessConfig {
                target_lufs: -14.0,
                max_gain_db: 12.0,
            }),
        };
        let source = super::build(decode(&path).expect("decoded above"), DEVICE_RATE, &full);
        let (wall, audio, samples) = drain(source);
        report(
            "解码 + 重采样 + EQ(2 段) + 响度(EBU R128)",
            wall,
            audio,
            samples,
        );

        println!("  → 播放时这一项再加分析帧（见 analysis_bench）与整帧（frame_bench）即整机开销");
    }
}
