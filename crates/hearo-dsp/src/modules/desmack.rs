//! De-smack (pause-aware): ducks lip smacks that sit in speech pauses (human H2 feedback).
//!
//! ```text
//! in ──▶ delay (40 ms lookahead) ──────────────────────────▶ × gain ──▶ out
//!   └─▶ mono ─▶ HP 1.5 kHz ─▶ fast HF power ┐
//!          └─▶ background power (pause floor, clamped) ├─▶ event detector ─▶ duck envelope
//!          └─▶ speech reference (slow) ────────────────┘
//! ```
//!
//! A lip smack is a short (5–30 ms), bright transient that jumps far above the *pause* it sits
//! in and then dies away within milliseconds. Detection needs all of:
//! 1. fast HF power ≥ `threshold` dB above the background (the pause floor just before it);
//! 2. that background is a real pause: ≥ `pause` dB below the recent speech level;
//! 3. the burst *decays* (−12 dB from its peak, and the whole sound −6 dB) within 40 ms —
//!    fricatives ("s") and drum bodies don't;
//! 4. it then *stays quiet*: back at the pause floor for 20 ms. Stop consonants ("t", "k",
//!    "p") look like "pause + decaying burst" too, but aspiration or voicing follows at once.
//!    The 40 ms lookahead lets us wait for all of this before the burst reaches the output
//!    (latency trade-off: ADR 0011).
//! 5. it is *salient*: its 2 ms peak within `below_speech` dB of the speech level at its onset,
//!    and ≥ 10 % of its energy above 1.5 kHz. Quiet or dull bursts were most of the catches the
//!    human rejected (2026-11-15).
//!
//! Repair: the burst is turned down to about the pause level (at most `range` dB; 0 = detect
//! only) with a 1 ms fade-in before the onset and a 5 ms fade-out, so speech starting right after
//! is untouched.
//!
//! The *background* ("pause floor") follows the 5 ms broadband level down within milliseconds and
//! may rise at most 30 dB/s, so a short burst can't lift the floor it is measured against.

use crate::delay::MultiDelay;
use crate::filters::{Biquad, one_pole_coeff};
use crate::module::{ParamDesc, ParamUnit};
use crate::telemetry::{Event, EventKind};
use crate::{AudioBlock, Module, ModuleId, TelemetryWriter};

/// Long enough to see a smack decay *and stay quiet* before it reaches the output.
const LOOKAHEAD_MS: f32 = 40.0;
/// After decaying, the sound must stay at the pause floor this long. Stop consonants are followed
/// by aspiration or voicing within 5–20 ms (measured trade-off, ADR 0011: 10 ms → 26 % of stops
/// ducked, 15 ms → 15 %, 20 ms → 2 %).
const QUIET_AFTER_MS: f32 = 20.0;
/// Envelope for the "stays quiet" check: fast enough not to smear the smack's own decay.
const QUIET_TAU_S: f32 = 0.002;
/// "At the floor" for the stay-quiet check (room noise wobbles a few dB on a 2 ms envelope).
const QUIET_ABOVE_BG_LIN: f32 = 6.309_573; // +8 dB
/// A long burst still not confirmed when it reaches the output is ducked provisionally once its
/// tail has been back at the pause floor this long (human-labelled miss, 2026-11-14).
const PROVISIONAL_QUIET_MS: f32 = 2.0;
const PRE_ONSET_MS: f32 = 1.0;
const RELEASE_MS: f32 = 5.0;
const MAX_LEN_MS: f32 = 40.0;
const LOCKOUT_MS: f32 = 15.0;
const HP_HZ: f64 = 1500.0;
const FAST_TAU_S: f32 = 0.0005;
/// The pause floor may rise at most this fast (a 40 ms burst lifts it ~1 dB).
const BG_RISE_DB_PER_S: f32 = 30.0;
const BG_INIT: f32 = 1e-6;
const BG_FALL_TAU_S: f32 = 0.003;
/// Envelope used to judge decay: long enough to bridge glottal pulses (7–10 ms apart).
const MID_TAU_S: f32 = 0.005;
const SPEECH_ATTACK_S: f32 = 0.05;
const SPEECH_RELEASE_S: f32 = 3.0;
const SPEECH_ENV_TAU_S: f32 = 0.02;
#[cfg(test)]
const CONFIRM_DROP_DB: f32 = 12.0;
/// …and the *whole* sound (broadband, incl. any low body) must have dropped this much too:
/// a smack's thump dies with it, a drum's body keeps sounding.
#[cfg(test)]
const CONFIRM_BB_DROP_DB: f32 = 6.0;
#[cfg(test)]
const END_ABOVE_BG_DB: f32 = 6.0;
const TARGET_ABOVE_BG_DB: f32 = 3.0;
const MIN_REDUCTION_DB: f32 = 3.0;
const MAX_EVENTS: usize = 8;
/// Precomputed linear power factors (keep in sync with the dB constants; checked by a test).
const CONFIRM_DROP_LIN: f32 = 0.063_095_73; // −12 dB
const CONFIRM_BB_DROP_LIN: f32 = 0.251_188_64; // −6 dB
#[cfg(test)]
const END_ABOVE_BG_LIN: f32 = 3.981_071_7; // +6 dB
const RISING_LIN: f32 = 3.981_071_7; // +6 dB
/// A smack follows a moment of quiet: the pause must have lasted this long before the burst
/// (rejects the spiky last glottal pulses of a syllable fading into a pause).
const MIN_PAUSE_MS: f32 = 8.0;
/// A burst within this long after a qualifying pause still counts as "in the pause".
const PAUSE_GRACE_MS: f32 = 3.0;
/// A smack's energy above 1.5 kHz must be at most this many dB below its broadband energy
/// (bright). Dull bursts in pauses — thumps, breath, low mouth noise — were the human-rejected
/// catches' second most common shape, after quiet ones (2026-11-15). 10 dB = ≥ 10 % HF energy.
/// Swept on 106 reviewed clips with *below speech* 18 dB: 6 dB → 58 false positives / 34 smacks
/// found, 8 → 60 / 34, 10 → 60 / 35, 12 → 62 / 35, off → 70 / 35. Synthetic smacks (with a low
/// thump) measure −1…−6.3 dB, so 10 dB leaves ≈ 3.7 dB margin.
const SALIENT_HF_SHARE_DB: f32 = 10.0;

const PARAMS: [ParamDesc; 4] = [
    ParamDesc {
        key: "threshold",
        name: "Threshold",
        unit: ParamUnit::Db,
        min: 6.0,
        max: 40.0,
        default: 14.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "pause",
        name: "Pause depth",
        unit: ParamUnit::Db,
        min: 0.0,
        max: 40.0,
        default: 15.0,
        step: 0.0,
        choices: &[],
    },
    ParamDesc {
        key: "range",
        name: "Range",
        unit: ParamUnit::Db,
        min: 0.0,
        max: 48.0,
        default: 24.0,
        step: 0.0,
        choices: &[],
    },
    // How far below the speech level a smack may peak and still be ducked: quieter ones are
    // left alone (most were false positives). Tuned 2026-11-15 on 111 reviewed snapshots
    // (`hearo dataset eval`, with the 10 dB brightness gate; before this gate: 132 false
    // positives, 37 smacks found): 22 dB → 81 / 37, 18 → 66 / 35, 15 → 53 / 35, 12 → 35 / 33.
    // 22 dB is the strictest value that still finds every smack the ungated detector found (the
    // human's bar: fewer false positives, at least as many smacks); the gate never adds a catch.
    // 48 dB ≈ off for any realistic level.
    ParamDesc {
        key: "below_speech",
        name: "Below speech",
        unit: ParamUnit::Db,
        min: 0.0,
        max: 48.0,
        default: 22.0,
        step: 0.0,
        choices: &[],
    },
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Burst rising/peaking; not yet known to be short.
    Open,
    /// Decayed fast enough; waiting to see it stay quiet (else it's a consonant).
    Decayed,
    /// Finished (end known).
    Closed,
}

#[derive(Debug, Clone, Copy)]
struct Smack {
    phase: Phase,
    /// Input-sample positions (module input clock).
    start: u64,
    end: u64,
    /// Output position where the fade-in starts (set at confirmation).
    duck_from: u64,
    peak_hf: f32,
    peak_mid: f32,
    peak_bb: f32,
    peak_bb_mid: f32,
    tail_min: f32,
    /// When the decayed burst reached the pause floor (u64::MAX = not yet).
    quiet_from: u64,
    bg: f32,
    /// Linear gain at the bottom of the duck.
    gain: f32,
    reported: bool,
    /// Speech reference when the burst started (the burst itself mustn't lift it).
    speech: f32,
    /// Summed HF (> 1.5 kHz) and broadband power over the burst (while Open).
    sum_hf: f32,
    sum_bb: f32,
}

impl Smack {
    /// Loud against the speech around it and bright: the smacks the human hears and marks
    /// (2026-11-15, 108 reviewed auto-saved catches). Quiet or dull bursts in a pause — breath
    /// noise, room thumps, a speaker's soft mouth noises — were 85 % of the false positives.
    fn salient(&self, loud_lin: f32, bright_lin: f32) -> bool {
        self.peak_bb_mid >= self.speech * loud_lin && self.sum_hf >= self.sum_bb * bright_lin
    }
}

/// The de-smack module.
#[derive(Debug)]
pub struct DeSmack {
    threshold_db: f32,
    pause_db: f32,
    range_db: f32,
    /// `threshold_db` / `−pause_db` as linear power factors (no powf per sample).
    threshold_lin: f32,
    pause_lin: f32,
    sr: f32,
    lookahead: usize,
    pre: u64,
    release: u64,
    max_len: u64,
    lockout: u64,
    hp: [Biquad; 2],
    hp_side: [Biquad; 2],
    a_fast: f32,
    bg_rise: f32,
    a_bg_fall: f32,
    a_mid: f32,
    e_mid: f32,
    e_bb_mid: f32,
    e_bb_q: f32,
    e_hf_q: f32,
    a_q: f32,
    a_sp_att: f32,
    a_sp_rel: f32,
    a_sp_env: f32,
    e_hf: f32,
    e_bb: f32,
    bg: f32,
    sp_env: f32,
    speech: f32,
    pos_in: u64,
    lock_until: u64,
    quiet_since: u64,
    pause_ok_until: u64,
    grace: u64,
    min_pause: u64,
    quiet_after: u64,
    prov_quiet: u64,
    events: [Option<Smack>; MAX_EVENTS],
    below_speech_db: f32,
    /// `−below_speech_db` / `−SALIENT_HF_SHARE_DB` as linear power factors.
    loud_lin: f32,
    bright_lin: f32,
    delay: MultiDelay,
    gain_now: f32,
}

impl Default for DeSmack {
    fn default() -> Self {
        Self::new()
    }
}

fn db_to_pow(db: f32) -> f32 {
    10f32.powf(db / 10.0)
}

fn pow_to_db(p: f32) -> f32 {
    10.0 * p.max(1e-20).log10()
}

impl DeSmack {
    /// De-smack with default parameters.
    pub fn new() -> Self {
        Self {
            threshold_db: PARAMS[0].default,
            pause_db: PARAMS[1].default,
            range_db: PARAMS[2].default,
            threshold_lin: db_to_pow(PARAMS[0].default),
            pause_lin: db_to_pow(-PARAMS[1].default),
            sr: 48_000.0,
            lookahead: 0,
            pre: 0,
            release: 1,
            max_len: 1,
            lockout: 0,
            hp: [Biquad::default(); 2],
            hp_side: [Biquad::default(); 2],
            a_fast: 0.0,
            bg_rise: 1.0,
            a_bg_fall: 0.0,
            a_mid: 0.0,
            e_mid: 0.0,
            e_bb_mid: 0.0,
            e_bb_q: 0.0,
            e_hf_q: 0.0,
            a_q: 0.0,
            a_sp_att: 0.0,
            a_sp_rel: 0.0,
            a_sp_env: 0.0,
            e_hf: 0.0,
            e_bb: 0.0,
            bg: BG_INIT,
            sp_env: 0.0,
            speech: 1e-12,
            pos_in: 0,
            lock_until: 0,
            quiet_since: u64::MAX,
            pause_ok_until: 0,
            grace: 0,
            min_pause: 0,
            quiet_after: 0,
            prov_quiet: 0,
            events: [None; MAX_EVENTS],
            below_speech_db: PARAMS[3].default,
            loud_lin: db_to_pow(-PARAMS[3].default),
            bright_lin: db_to_pow(-SALIENT_HF_SHARE_DB),
            delay: MultiDelay::default(),
            gain_now: 1.0,
        }
    }

    fn samples(&self, ms: f32) -> u64 {
        (self.sr * ms / 1000.0).round() as u64
    }

    /// Feed one (future) mono sample to the detector. `abs` = absolute chain position for telemetry.
    #[inline]
    fn detect(&mut self, mid: f32, side: f32) {
        let (loud_lin, bright_lin) = (self.loud_lin, self.bright_lin);
        // Clamp so absurd (finite) input can't overflow x² into inf and NaN the envelopes.
        let x = mid.clamp(-1e4, 1e4);
        let sd = side.clamp(-1e4, 1e4);
        let i = self.pos_in;
        // Mean channel power = mid² + side² (mid = (L+R)/2, side = (L−R)/2): a smack in one
        // channel keeps its margin and opposite-polarity channels can't cancel.
        let mut h = x as f64;
        for f in &mut self.hp {
            h = f.tick(h);
        }
        let mut hs = sd as f64;
        for f in &mut self.hp_side {
            hs = f.tick(hs);
        }
        let hf_p = (h * h + hs * hs) as f32;
        let bb_p = x * x + sd * sd;
        self.e_hf += self.a_fast * (hf_p - self.e_hf);
        self.e_mid += self.a_mid * (hf_p - self.e_mid);
        self.e_bb_mid += self.a_mid * (bb_p - self.e_bb_mid);
        self.e_bb_q += self.a_q * (bb_p - self.e_bb_q);
        self.e_hf_q += self.a_q * (hf_p - self.e_hf_q);
        let loud = (x.abs() + sd.abs()) * (x.abs() + sd.abs()); // = max(L², R²): depth uses the louder channel
        self.e_bb += self.a_fast * (loud - self.e_bb);
        self.sp_env += self.a_sp_env * (bb_p - self.sp_env);
        let a = if self.sp_env > self.speech { self.a_sp_att } else { self.a_sp_rel };
        self.speech += a * (self.sp_env - self.speech);

        let mut busy = false;
        let (quiet_after, range_db, pre) = (self.quiet_after, self.range_db, self.pre);
        let (e_hf, e_bb, max_len, out_pos) =
            (self.e_hf, self.e_bb, self.max_len, i.saturating_sub(self.lookahead as u64));
        let (e_q, e_hfq) = (self.e_bb_q, self.e_hf_q);
        let prov_quiet = self.prov_quiet;
        for slot in self.events.iter_mut() {
            let Some(ev) = slot else { continue };
            match ev.phase {
                Phase::Open => {
                    busy = true;
                    ev.peak_hf = ev.peak_hf.max(e_hf);
                    ev.peak_mid = ev.peak_mid.max(e_hfq);
                    ev.peak_bb = ev.peak_bb.max(e_bb);
                    ev.peak_bb_mid = ev.peak_bb_mid.max(e_q);
                    ev.sum_hf += hf_p;
                    ev.sum_bb += bb_p;
                    if i - ev.start > max_len {
                        *slot = None; // too long: a fricative or a real sound, not a smack
                    } else if e_hfq < ev.peak_mid * CONFIRM_DROP_LIN && e_q < ev.peak_bb_mid * CONFIRM_BB_DROP_LIN {
                        // Decay is judged on 2 ms envelopes; voiced speech (whose pulses dip
                        // between periods) is rejected by the stay-quiet check that follows.
                        // Decayed. Now it must *stay* quiet: back at the pause floor for
                        // QUIET_AFTER. A stop consonant is followed by aspiration or voicing.
                        ev.phase = Phase::Decayed;
                        ev.tail_min = e_q;
                        ev.quiet_from = u64::MAX;
                    }
                }
                Phase::Decayed => {
                    busy = true;
                    let mut drop_it = false;
                    ev.tail_min = ev.tail_min.min(e_q);
                    let floor = ev.bg * QUIET_ABOVE_BG_LIN;
                    let at_floor = e_q < floor;
                    let quiet_ok = at_floor && ev.quiet_from != u64::MAX && i - ev.quiet_from >= prov_quiet;
                    // "Rose again" relative to the floor, not the running minimum: near the floor
                    // a short envelope of room noise wobbles several dB on its own.
                    let rising = e_q > ev.tail_min.max(floor) * RISING_LIN;
                    // The burst and its tail must reach the floor in time; the time spent
                    // confirming quiet after that doesn't count (long smacks with a room tail).
                    let too_long = ev.quiet_from == u64::MAX && i - ev.start > max_len + quiet_after;
                    if rising && ev.duck_from == u64::MAX && i - ev.start <= max_len {
                        // Another tick of the same smack (multi-tick smacks, 2026-11-14): back to
                        // Open. Voicing after a stop rises too, but doesn't decay again within
                        // MAX_LEN, so it is still rejected there.
                        ev.phase = Phase::Open;
                    } else if rising || too_long {
                        // Sound rose again (consonant + speech) or never settled: not a smack.
                        if ev.duck_from == u64::MAX {
                            drop_it = true;
                        } else {
                            // A provisional duck is already playing: release it smoothly from here.
                            ev.end = out_pos.max(ev.start);
                            ev.phase = Phase::Closed;
                            ev.reported = true; // not a smack: no event (DSP review)
                        }
                    } else if ev.duck_from == u64::MAX
                        && out_pos + pre >= ev.start
                        && quiet_ok
                        && ev.salient(loud_lin, bright_lin)
                    {
                        // Deadline: the burst is (about to be) playing, and has decayed back to
                        // the pause floor. Only long bursts get here (a short stop burst finishes
                        // its stay-quiet check first; aspiration or voicing keeps a stop far above
                        // the floor), so duck now — late by however long the tail took — and keep
                        // checking; release if the sound rises again.
                        let reduction = (pow_to_db(ev.peak_bb) - pow_to_db(ev.bg) - TARGET_ABOVE_BG_DB).max(0.0);
                        if reduction >= MIN_REDUCTION_DB {
                            ev.gain = crate::db_to_amp(-reduction.min(range_db));
                            ev.duck_from = ev.start.saturating_sub(pre).max(out_pos);
                        }
                    }
                    if drop_it || ev.phase != Phase::Decayed {
                        // dropped or released above
                    } else if at_floor {
                        if ev.quiet_from == u64::MAX {
                            ev.quiet_from = i;
                        }
                        if i - ev.quiet_from >= quiet_after {
                            // A smack in a pause. Duck it to about the pause level, within range.
                            let reduction = (pow_to_db(ev.peak_bb) - pow_to_db(ev.bg) - TARGET_ABOVE_BG_DB).max(0.0);
                            if ev.duck_from == u64::MAX
                                && (reduction < MIN_REDUCTION_DB || !ev.salient(loud_lin, bright_lin))
                            {
                                drop_it = true;
                            } else {
                                if ev.duck_from == u64::MAX {
                                    ev.gain = crate::db_to_amp(-reduction.min(range_db));
                                    ev.duck_from = ev.start.saturating_sub(pre).max(out_pos);
                                }
                                // Never before what has already been rendered fully ducked.
                                ev.end = ev.quiet_from.max(out_pos);
                                ev.phase = Phase::Closed;
                            }
                        }
                    } else {
                        ev.quiet_from = u64::MAX;
                    }
                    if drop_it {
                        *slot = None;
                    }
                }
                // Removed in `process` once the gain loop has rendered its release.
                Phase::Closed => {}
            }
        }

        // Background: the pause floor. Grows slowly (clamped) so a burst can't lift it, and is
        // frozen while an event is open.
        // Background = the pause floor: falls fast (sits on the floor within ms of speech
        // ending) and rises slowly and clamped (a burst can't lift it).
        let p = self.e_bb_mid; // smoothed: instantaneous power dips at every zero crossing
        if p < self.bg {
            self.bg += self.a_bg_fall * (p - self.bg);
        } else {
            self.bg = p.min(self.bg * self.bg_rise);
        }
        self.bg = self.bg.max(1e-12);

        // Pause tracking on the smoothed level. A smack *ends* the quiet it sits in (it's loud),
        // so a burst counts as "in a pause" if a qualifying pause (≥ MIN_PAUSE) was still going
        // on within the last GRACE ms.
        let quiet_level = self.speech * self.pause_lin;
        if self.e_bb_mid < quiet_level {
            if self.quiet_since == u64::MAX {
                self.quiet_since = i;
            }
            if i - self.quiet_since >= self.min_pause {
                self.pause_ok_until = i + self.grace;
            }
        } else {
            self.quiet_since = u64::MAX;
        }
        let in_pause = i <= self.pause_ok_until && self.bg < quiet_level;
        let burst = e_hf > self.bg * self.threshold_lin;
        if !busy
            && i >= self.lock_until
            && in_pause
            && burst
            && let Some(slot) = self.events.iter_mut().find(|s| s.is_none())
        {
            {
                *slot = Some(Smack {
                    phase: Phase::Open,
                    start: i,
                    end: u64::MAX,
                    duck_from: u64::MAX,
                    peak_hf: e_hf,
                    peak_mid: e_hfq,
                    peak_bb: e_bb,
                    peak_bb_mid: e_q,
                    tail_min: e_hf,
                    quiet_from: u64::MAX,
                    bg: self.bg,
                    gain: 1.0,
                    reported: false,
                    speech: self.speech,
                    sum_hf: 0.0,
                    sum_bb: 0.0,
                });
                self.lock_until = i + self.lockout;
            }
        }
        self.pos_in = i + 1;
    }

    /// Gain for output position `n` (input clock minus lookahead).
    #[inline]
    fn gain_at(&self, n: u64) -> f32 {
        let mut g = 1.0f32;
        for ev in self.events.iter().flatten() {
            // Closed events, and provisional ducks of long bursts (duck_from set, end not yet).
            if ev.duck_from == u64::MAX || n < ev.duck_from {
                continue;
            }
            // Fade in over ≥ PRE_ONSET even when confirmation came late (never a 1-sample step).
            let attack_end = ev.start.max(ev.duck_from + self.pre.max(1));
            let ramp_in =
                if n >= attack_end { 1.0 } else { (n - ev.duck_from) as f32 / (attack_end - ev.duck_from) as f32 };
            let ramp_out = if n > ev.end { 1.0 - ((n - ev.end) as f32 / self.release as f32).min(1.0) } else { 1.0 };
            let depth = ramp_in.min(ramp_out);
            g = g.min(1.0 - depth * (1.0 - ev.gain));
        }
        g
    }
}

impl Module for DeSmack {
    fn id(&self) -> ModuleId {
        ModuleId::DeSmack
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sample_rate;
        self.lookahead = self.samples(LOOKAHEAD_MS) as usize;
        self.pre = self.samples(PRE_ONSET_MS);
        self.release = self.samples(RELEASE_MS).max(1);
        self.max_len = self.samples(MAX_LEN_MS);
        self.lockout = self.samples(LOCKOUT_MS);
        self.min_pause = self.samples(MIN_PAUSE_MS);
        self.quiet_after = self.samples(QUIET_AFTER_MS);
        self.prov_quiet = self.samples(PROVISIONAL_QUIET_MS);
        self.grace = self.samples(PAUSE_GRACE_MS);
        let sr = sample_rate as f64;
        self.hp = [Biquad::highpass(sr, HP_HZ, 0.707); 2];
        self.hp_side = self.hp;
        self.a_fast = one_pole_coeff(sample_rate, FAST_TAU_S);
        self.bg_rise = 10f32.powf(BG_RISE_DB_PER_S / 10.0 / sample_rate);
        self.a_bg_fall = one_pole_coeff(sample_rate, BG_FALL_TAU_S);
        self.a_mid = one_pole_coeff(sample_rate, MID_TAU_S);
        self.a_q = one_pole_coeff(sample_rate, QUIET_TAU_S);
        self.a_sp_att = one_pole_coeff(sample_rate, SPEECH_ATTACK_S);
        self.a_sp_rel = one_pole_coeff(sample_rate, SPEECH_RELEASE_S);
        self.a_sp_env = one_pole_coeff(sample_rate, SPEECH_ENV_TAU_S);
        self.delay.prepare(channels, self.lookahead);
        self.reset();
    }

    fn reset(&mut self) {
        self.hp.iter_mut().chain(self.hp_side.iter_mut()).for_each(Biquad::reset);
        self.e_hf = 0.0;
        self.e_bb = 0.0;
        self.e_mid = 0.0;
        self.e_bb_mid = 0.0;
        self.e_bb_q = 0.0;
        self.e_hf_q = 0.0;
        self.bg = BG_INIT;
        self.sp_env = 0.0;
        self.speech = 1e-12;
        self.pos_in = 0;
        self.lock_until = 0;
        self.quiet_since = u64::MAX;
        self.pause_ok_until = 0;
        self.events = [None; MAX_EVENTS];
        self.delay.reset();
        self.gain_now = 1.0;
    }

    fn latency_samples(&self) -> usize {
        self.lookahead
    }

    fn process(&mut self, block: &mut AudioBlock<'_>, telemetry: &mut TelemetryWriter) {
        let n = block.frames();
        let ch = block.channels();

        let block_in_start = self.pos_in;
        let chain_start = telemetry.block_start();
        // Detector reads the undelayed (future) input …
        for i in 0..n {
            let (mid, side) = match ch {
                1 => (block.channel(0)[i], 0.0),
                _ => {
                    let (l, r) = (block.channel(0)[i], block.channel(1)[i]);
                    (0.5 * (l + r), 0.5 * (l - r))
                }
            };
            self.detect(mid, side);
            // … report confirmed smacks once (chain-input clock).
            for ev in self.events.iter_mut().flatten() {
                if ev.phase == Phase::Closed && !ev.reported {
                    ev.reported = true;
                    telemetry.push_event(Event {
                        module: ModuleId::DeSmack,
                        kind: EventKind::Smack,
                        // Module-input clock → chain-input clock (offset of this block).
                        start_sample: (chain_start as i64 + ev.start as i64 - block_in_start as i64).max(0) as u64,
                        duration_samples: (ev.end - ev.start) as u32,
                        energy_removed_db: -crate::amp_to_db(ev.gain),
                    });
                }
            }
        }
        // … the audio is delayed by the lookahead and ducked.
        self.delay.process(block);
        let mut g = self.gain_now;
        let mut min_g = 1.0f32;
        for i in 0..n {
            let pos = block_in_start + i as u64;
            g = if pos < self.lookahead as u64 { 1.0 } else { self.gain_at(pos - self.lookahead as u64) };
            if g < 1.0 {
                min_g = min_g.min(g);
                for c in 0..ch {
                    block.channel_mut(c)[i] *= g;
                }
            }
        }
        self.gain_now = g;
        // Events whose release has been fully rendered can go (never earlier: see gain_at).
        let rendered_until = self.pos_in.saturating_sub(self.lookahead as u64);
        let release = self.release;
        for slot in &mut self.events {
            if slot.is_some_and(|ev| ev.phase == Phase::Closed && ev.end + release < rendered_until) {
                *slot = None;
            }
        }
        // Block maximum, so a 10 ms duck still shows on a meter polled every few blocks.
        telemetry.set_gain_reduction(ModuleId::DeSmack, -crate::amp_to_db(min_g));
    }

    fn params(&self) -> &[ParamDesc] {
        &PARAMS
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(p) = PARAMS.get(index) else { return };
        let v = p.clamp(value);
        match index {
            0 => {
                self.threshold_db = v;
                self.threshold_lin = db_to_pow(v);
            }
            1 => {
                self.pause_db = v;
                self.pause_lin = db_to_pow(-v);
            }
            2 => self.range_db = v,
            _ => {
                self.below_speech_db = v;
                self.loud_lin = db_to_pow(-v);
            }
        }
    }

    fn primary_param(&self) -> Option<usize> {
        Some(2) // range
    }

    fn param(&self, index: usize) -> f32 {
        match index {
            0 => self.threshold_db,
            1 => self.pause_db,
            2 => self.range_db,
            3 => self.below_speech_db,
            _ => f32::NAN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_constants_match_their_db() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5 * b.max(1.0);
        assert!(close(CONFIRM_DROP_LIN, db_to_pow(-CONFIRM_DROP_DB)));
        assert!(close(CONFIRM_BB_DROP_LIN, db_to_pow(-CONFIRM_BB_DROP_DB)));
        assert!(close(END_ABOVE_BG_LIN, db_to_pow(END_ABOVE_BG_DB)));
        assert!(close(RISING_LIN, db_to_pow(6.0)));
        assert!(close(QUIET_ABOVE_BG_LIN, db_to_pow(8.0)));
    }

    #[test]
    fn params_update_linear_factors() {
        let mut m = DeSmack::new();
        m.set_param(0, 20.0);
        m.set_param(1, 10.0);
        m.set_param(3, 20.0);
        assert!((m.threshold_lin - 100.0).abs() < 1e-3);
        assert!((m.pause_lin - 0.1).abs() < 1e-6);
        assert!((m.loud_lin - 0.01).abs() < 1e-6);
        assert_eq!(m.param(3), 20.0);
        assert!((DeSmack::new().bright_lin - db_to_pow(-SALIENT_HF_SHARE_DB)).abs() < 1e-9);
    }

    #[test]
    fn huge_input_does_not_poison_the_detector() {
        let mut m = DeSmack::new();
        m.prepare(48_000.0, 64, 1);
        for _ in 0..100 {
            m.detect(1e30, 1e30);
        }
        assert!(m.speech.is_finite() && m.bg.is_finite() && m.e_hf.is_finite());
    }
}

#[cfg(test)]
mod probe {
    #![allow(clippy::print_stderr)]
    use super::*;
    use hearo_testkit::signals::speech_with_pause_smacks;

    #[test]
    #[ignore = "diagnostic"]
    fn probe_conditions_at_smacks() {
        let sr = 48_000.0;
        let (x, smacks) = speech_with_pause_smacks(sr, 30.0, 21, 15, (-22.0, -10.0), -55.0);
        let mut m = DeSmack::new();
        m.prepare(sr, 512, 1);
        let mut log = vec![(0.0f32, 0.0f32, 0.0f32); x.len()];
        let mut full = vec![[0.0f32; 6]; x.len()];
        for (i, &v) in x.iter().enumerate() {
            m.detect(v, 0.0);
            log[i] = (pow_to_db(m.bg), pow_to_db(m.speech), pow_to_db(m.e_hf));
            let q = if m.quiet_since == u64::MAX { -1.0 } else { (i as u64 - m.quiet_since) as f32 / sr * 1e3 };
            full[i] =
                [pow_to_db(m.e_hf), pow_to_db(m.e_mid), pow_to_db(m.e_bb_mid), pow_to_db(m.bg), pow_to_db(m.speech), q];
        }
        let dump = |label: &str, at: usize| {
            eprintln!("TRACE {label} @{:.3}s  (ms: hf / mid / bb_mid / bg / ev.floor / phase)", at as f32 / sr);
            for k in (at.saturating_sub(480)..(at + 2400).min(x.len())).step_by(96) {
                let f = full[k];
                eprintln!(
                    "  {:+6.1}ms  {:6.1} {:6.1} {:6.1} {:6.1} {:6.1} {:6.1}",
                    (k as f32 - at as f32) / sr * 1e3,
                    f[0],
                    f[1],
                    f[2],
                    f[3],
                    f[4],
                    f[5]
                );
            }
        };
        dump("smack#1", smacks[0].start);
        // Run the real module to get events, then classify them.
        let mut mm = DeSmack::new();
        mm.prepare(sr, 512, 1);
        let mut t = crate::TelemetryWriter::new();
        let mut buf = x.clone();
        let mut evs = Vec::new();
        for (k, chunk) in buf.chunks_mut(512).enumerate() {
            t.set_block_start((k * 512) as u64);
            mm.process(&mut crate::AudioBlock::from_planar(chunk, 1), &mut t);
            evs.extend_from_slice(t.events());
            t.clear_events();
        }
        let hit = |e: &crate::Event| {
            smacks
                .iter()
                .any(|s| (e.start_sample as usize) + 480 >= s.start && (e.start_sample as usize) <= s.end() + 240)
        };
        eprintln!("events {} ({} matched)", evs.len(), evs.iter().filter(|e| hit(e)).count());
        if let Some(fp) = evs.iter().find(|e| !hit(e)) {
            dump("FALSE#1", fp.start_sample as usize);
        }
        for e in evs.iter().filter(|e| !hit(e)).take(8) {
            let k = e.start_sample as usize;
            eprintln!(
                "  FALSE @{:.3}s dur {:.1}ms removed {:.1} dB | bg {:.1} speech {:.1} hf {:.1}",
                k as f32 / sr,
                e.duration_samples as f32 / sr * 1e3,
                e.energy_removed_db,
                log[k].0,
                log[k].1,
                log[k + 48].2
            );
        }
        for s in smacks.iter().filter(|s| {
            !evs.iter()
                .any(|e| (e.start_sample as usize) + 480 >= s.start && (e.start_sample as usize) <= s.end() + 240)
        }) {
            eprintln!("  MISSED @{:.3}s", s.start as f32 / sr);
        }
        let lat = mm.latency_samples();
        for (e, s) in evs.iter().zip(&smacks).take(8) {
            let e_in: f32 = x[s.start..s.end()].iter().map(|v| v * v).sum();
            let e_out: f32 = buf[s.start + lat..s.end() + lat].iter().map(|v| v * v).sum();
            eprintln!(
                "  EV start {:+.1}ms vs smack, dur {:.1}ms (smack {:.1}ms), gain -{:.1} dB → measured -{:.1} dB",
                (e.start_sample as f32 - s.start as f32) / sr * 1e3,
                e.duration_samples as f32 / sr * 1e3,
                s.len as f32 / sr * 1e3,
                e.energy_removed_db,
                10.0 * (e_in / e_out.max(1e-20)).log10()
            );
        }
        for s in &smacks {
            let before = log[s.start.saturating_sub(10)];
            let peak = (s.start..s.end()).map(|i| log[i].2).fold(f32::MIN, f32::max);
            let peak_at = (s.start..s.end()).max_by(|a, b| log[*a].2.total_cmp(&log[*b].2)).unwrap();
            let after = (peak_at..(peak_at + 1920).min(x.len())).map(|i| log[i].2).fold(f32::MAX, f32::min);
            eprintln!(
                "smack @{:.3}s len {:>4.1}ms: bg {:6.1} speech {:6.1} (pause margin {:5.1}) | HF peak {:6.1} = bg+{:5.1} | min within 40ms after peak {:6.1} (drop {:5.1})",
                s.start as f32 / sr,
                s.len as f32 / sr * 1000.0,
                before.0,
                before.1,
                before.1 - before.0,
                peak,
                peak - before.0,
                after,
                peak - after
            );
        }
    }
}

#[cfg(test)]
mod probe_music {
    #![allow(clippy::print_stderr)]
    use super::*;
    use hearo_testkit::signals::{add_at, music_like, pink};

    #[test]
    #[ignore = "diagnostic"]
    fn probe_music_false_positives() {
        let sr = 48_000.0;
        let mut x = music_like(sr, 120.0, 32);
        let floor = pink(&mut hearo_testkit::Rng::new(2), x.len(), 10f32.powf(-55.0 / 20.0));
        add_at(&mut x, &floor, 0);
        let mut m = DeSmack::new();
        m.prepare(sr, 512, 1);
        let mut t = crate::TelemetryWriter::new();
        let mut evs = Vec::new();
        for (k, chunk) in x.chunks_mut(512).enumerate() {
            t.set_block_start((k * 512) as u64);
            m.process(&mut crate::AudioBlock::from_planar(chunk, 1), &mut t);
            evs.extend_from_slice(t.events());
            t.clear_events();
        }
        let beat = sr * 0.5;
        for e in &evs {
            let s = e.start_sample as f32;
            eprintln!(
                "music FP @{:7.3}s  beat phase {:.3}  dur {:.1}ms  removed {:.1} dB",
                s / sr,
                (s % beat) / beat,
                e.duration_samples as f32 / sr * 1e3,
                e.energy_removed_db
            );
        }
    }
}

#[cfg(test)]
mod probe_real {
    #![allow(clippy::print_stderr)]
    use super::*;

    /// HEARO_PROBE_WAV=<file> HEARO_PROBE_AT=<seconds> cargo test -p hearo-dsp --lib probe_real -- --ignored --nocapture
    #[test]
    #[ignore = "diagnostic on local snapshots"]
    fn probe_real_snapshot() {
        let (Ok(path), Ok(at)) = (std::env::var("HEARO_PROBE_WAV"), std::env::var("HEARO_PROBE_AT")) else { return };
        let w = hearo_testkit::wav::read(&path).unwrap();
        let sr = w.sample_rate as f32;
        let at: f32 = at.parse().unwrap();
        let mut m = DeSmack::new();
        m.prepare(sr, 512, 2);
        let (l, r) = (&w.channels[0], &w.channels[w.channels.len() - 1]);
        let from = ((at - 0.15) * sr) as usize;
        let to = ((at + 0.12) * sr) as usize;
        eprintln!("ms      hf    mid  bb_mid  bb_q    bg  speech  quiet  phase");
        for i in 0..to.min(l.len()) {
            m.detect(0.5 * (l[i] + r[i]), 0.5 * (l[i] - r[i]));
            if i >= from && (i - from).is_multiple_of((sr / 1000.0) as usize * 2) {
                let ph = m
                    .events
                    .iter()
                    .flatten()
                    .filter(|e| e.phase != Phase::Closed || e.end + m.release > i as u64)
                    .map(|e| match e.phase {
                        Phase::Open => "open",
                        Phase::Decayed => "decayed",
                        Phase::Closed => "CLOSED",
                    })
                    .next()
                    .unwrap_or("");
                let q = if m.quiet_since == u64::MAX { -1.0 } else { (i as u64 - m.quiet_since) as f32 / sr * 1e3 };
                eprintln!(
                    "{:+5.0} {:6.1} {:6.1} {:6.1} {:6.1} {:6.1} {:6.1} {:6.1}  {}",
                    (i as f32 / sr - at) * 1e3,
                    pow_to_db(m.e_hf),
                    pow_to_db(m.e_mid),
                    pow_to_db(m.e_bb_mid),
                    pow_to_db(m.e_bb_q),
                    pow_to_db(m.bg),
                    pow_to_db(m.speech),
                    q,
                    ph
                );
            }
        }
    }
}
