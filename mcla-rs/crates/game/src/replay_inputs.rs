//! The data side of the original's replay, as pure functions: the player-input quantiser / unpacker, the replay buffer stream
//! (record / playback, delta and raw mode), the 21-byte frame and the position parity that detects a desync.
//! Decoded in `docs/reference/cameras/replay_inputs_rule.md`; nothing calls this yet (the live input path and the game loop
//! are not wired), so the module is allowed to be dead code.
//!
//! Each piece is checked word for word against the original's own code run offline (recompiled functions, harness in
//! `work/replay-inputs/h`, data in `testdata/replay_*_ref.txt`).
#![allow(dead_code)]

// ---------------------------------------------------------------------------------------------------------------------------
// 1. Input quantiser (F-57773) and unpacker (F-61107)
// ---------------------------------------------------------------------------------------------------------------------------

/// The twelve bytes of a frame that hold the local player's input (frame bytes 9..20).
pub const INPUT_BYTES: usize = 12;

/// The fields of the player input object (mcPlayerInput) that the replay records, named by meaning; the comments give the
/// original's field offsets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReplayInput {
    /// +12, 0..1, recorded as one unsigned byte (frame byte 9).
    pub throttle: f32,
    /// +16, 0..1, unsigned byte (byte 10).
    pub brake: f32,
    /// +20, 0..1, unsigned byte (byte 11).
    pub handbrake: f32,
    /// +84, the raw steering input, -1..1, signed byte (byte 12).
    pub steer_raw: f32,
    /// +28, signed byte (byte 13); the original's recorder treats it exactly like the steering input.
    pub axis_28: f32,
    /// +32, stick Y, signed byte (byte 14).
    pub stick_y: f32,
    /// +36, stick X, signed byte (byte 15).
    pub stick_x: f32,
    /// +40, the button / state flag word, recorded big-endian, MSB first (bytes 16..19). Replaced as a whole on playback.
    pub flags: u32,
    /// +80, a word of which only the low byte is recorded (byte 20); on playback the word is the byte, zero-extended.
    pub word_80: u32,
}

/// 1/255 and 1/127 as single-precision constants (the original multiplies the code by them).
const INV_255: f32 = 1.0 / 255.0;
const INV_127: f32 = 1.0 / 127.0;

/// `q = trunc(clamp(fmadds(x, 255, 0.5), 0, 255))`; a NaN gives 0 (the clamps are fsel).
fn quantise_pedal(x: f32) -> u8 {
    let t = x.mul_add(255.0, 0.5);
    let lo = if t >= 0.0 { t } else { 0.0 };
    let v = if lo - 255.0 >= 0.0 { 255.0 } else { lo };
    v as i64 as u8
}

/// `q = trunc_toward_zero(clamp(fmadds(x, 127, 0.5), -127, 127))`; a NaN gives -127. Returns the signed code.
fn quantise_signed(x: f32) -> i8 {
    let t = x.mul_add(127.0, 0.5);
    let lo = if t - (-127.0) >= 0.0 { t } else { -127.0 };
    let v = if lo - 127.0 >= 0.0 { 127.0 } else { lo };
    v as i32 as i8
}

impl ReplayInput {
    /// Record mode: quantise the seven floats into frame bytes 9..15, write the quantised values back into the input (so the
    /// recorded race is driven by what playback will feed) and add the flag word (big-endian) and the low byte of +80.
    pub fn quantise(&mut self) -> [u8; INPUT_BYTES] {
        let mut b = [0u8; INPUT_BYTES];
        let p = [self.throttle, self.brake, self.handbrake].map(quantise_pedal);
        for i in 0..3 {
            b[i] = p[i];
        }
        self.throttle = p[0] as f32 * INV_255;
        self.brake = p[1] as f32 * INV_255;
        self.handbrake = p[2] as f32 * INV_255;
        let s = [self.steer_raw, self.axis_28, self.stick_y, self.stick_x].map(quantise_signed);
        for i in 0..4 {
            b[3 + i] = s[i] as u8;
        }
        self.steer_raw = s[0] as f32 * INV_127;
        self.axis_28 = s[1] as f32 * INV_127;
        self.stick_y = s[2] as f32 * INV_127;
        self.stick_x = s[3] as f32 * INV_127;
        b[7..11].copy_from_slice(&self.flags.to_be_bytes());
        b[11] = self.word_80 as u8;
        b
    }

    /// Playback: replace the twelve values with the recorded bytes.
    pub fn unpack(&mut self, b: &[u8; INPUT_BYTES]) {
        self.throttle = b[0] as f32 * INV_255;
        self.brake = b[1] as f32 * INV_255;
        self.handbrake = b[2] as f32 * INV_255;
        self.steer_raw = (b[3] as i8) as f32 * INV_127;
        self.axis_28 = (b[4] as i8) as f32 * INV_127;
        self.stick_y = (b[5] as i8) as f32 * INV_127;
        self.stick_x = (b[6] as i8) as f32 * INV_127;
        self.flags = u32::from_be_bytes([b[7], b[8], b[9], b[10]]);
        self.word_80 = b[11] as u32;
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// 2. The 21-byte frame
// ---------------------------------------------------------------------------------------------------------------------------

/// Bytes of a frame (the buffer's BeginRecord argument 20, plus the flag byte).
pub const FRAME_SIZE: usize = 21;

/// What one rendered frame stores: byte 0 the flags (bit 0 cine skip button, bit 1 speech manager state, bit 2 position
/// parity), bytes 1..4 the time scale and 5..8 the raw frame time (f32, big-endian), bytes 9..20 the input bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReplayFrame {
    pub cine_skip: bool,
    pub speech: bool,
    pub parity: bool,
    pub time_scale: f32,
    pub frame_time: f32,
    pub input: [u8; INPUT_BYTES],
}

impl ReplayFrame {
    /// The frame as the writers lay it out; the flag byte's other bits stay 0 (nothing in the original sets them).
    pub fn to_bytes(&self) -> [u8; FRAME_SIZE] {
        let mut f = [0u8; FRAME_SIZE];
        f[0] = self.cine_skip as u8 | (self.speech as u8) << 1 | (self.parity as u8) << 2;
        f[1..5].copy_from_slice(&self.time_scale.to_bits().to_be_bytes());
        f[5..9].copy_from_slice(&self.frame_time.to_bits().to_be_bytes());
        f[9..].copy_from_slice(&self.input);
        f
    }

    pub fn from_bytes(f: &[u8; FRAME_SIZE]) -> ReplayFrame {
        let mut input = [0u8; INPUT_BYTES];
        input.copy_from_slice(&f[9..]);
        ReplayFrame {
            cine_skip: f[0] & 1 != 0,
            speech: f[0] & 2 != 0,
            parity: f[0] & 4 != 0,
            time_scale: f32::from_bits(u32::from_be_bytes([f[1], f[2], f[3], f[4]])),
            frame_time: f32::from_bits(u32::from_be_bytes([f[5], f[6], f[7], f[8]])),
            input,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// 3. Position parity (F-59932) and its use at the frame tick (F-99640)
// ---------------------------------------------------------------------------------------------------------------------------

/// One entry of the racer list as the parity reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParityRacer {
    /// false: a null list entry (skipped).
    pub present: bool,
    /// The racer's state word (+660); 1 counts as active without asking the racer.
    pub state: i32,
    /// The result of the racer's virtual "is active" call (vt+40); only its low byte is tested.
    pub active_call: u32,
    /// The raw bits of the matrix position (x, y, z).
    pub pos_bits: [u32; 3],
}

/// Bit 0 of the XOR of the raw position bits (x, y, z) of every active racer. `count` is the list's own count (a count of
/// 0 or less reads nothing); `racers` are the entries in list order.
pub fn position_parity(count: i32, racers: &[ParityRacer]) -> bool {
    let mut acc = 0u32;
    if count > 0 {
        for r in racers.iter().take(count as usize) {
            if !r.present {
                continue;
            }
            if r.state == 1 || (r.active_call & 0xFF) != 0 {
                acc ^= r.pos_bits[0] ^ r.pos_bits[1] ^ r.pos_bits[2];
            }
        }
    }
    acc & 1 != 0
}

/// Flag bits of the buffer's +76 byte.
pub const BUF_RECORDING: u8 = 0x80;
pub const BUF_PLAYING: u8 = 0x40;
/// Bit of the replay manager's flag byte (+2620) that marks a desync.
pub const MGR_DESYNC: u8 = 0x04;

/// The parity step of the frame tick, after the buffer's begin-frame: recording stores the parity into frame byte 0 bit 2;
/// playback compares the recorded bit with the parity now and sets the manager's desync flag when they differ.
/// `parity` is only needed (and in the original only computed) in those two modes.
pub fn frame_parity_step(buffer_flags: u8, byte0: &mut u8, mgr_flags: &mut u8, parity: impl FnOnce() -> bool) {
    if buffer_flags & BUF_RECORDING != 0 {
        if parity() {
            *byte0 |= 4;
        } else {
            *byte0 &= !4;
        }
    } else if buffer_flags & BUF_PLAYING != 0 {
        let recorded = (*byte0 >> 2) & 1 != 0;
        if recorded != parity() {
            *mgr_flags |= MGR_DESYNC;
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// 4. mcReplayBuffer: the frame stream (F-65920 BeginRecord, F-55913 BeginPlayback, F-70520 begin-frame / read,
//    F-17812 write, F-31252 StopRecord, F-44391 StopPlayback)
// ---------------------------------------------------------------------------------------------------------------------------

/// The replay buffer object with its memory stream. `block_size` is the data block (131072, or 8192 with the `shortreplay`
/// parameter), `delta` the mode (true: change-mask bytes; false: raw frames).
#[derive(Clone, Debug)]
pub struct ReplayBuffer {
    data: Vec<u8>,
    block_size: i32,
    recorded_len: i32,
    frame_size: usize,
    /// +20: the cursor into the current frame; 1 after begin-frame (byte 0 is the flag byte).
    cursor: usize,
    delta: bool,
    prev: [u8; 24],
    cur: [u8; 24],
    flags: u8,
    /// The stream: `Some(position)` while open, None once closed (the original's handle 0).
    stream: Option<usize>,
}

impl ReplayBuffer {
    pub fn new(block_size: usize, delta: bool) -> ReplayBuffer {
        ReplayBuffer {
            data: vec![0; block_size],
            block_size: block_size as i32,
            recorded_len: 0,
            frame_size: 0,
            cursor: 0,
            delta,
            prev: [0; 24],
            cur: [0; 24],
            flags: 0,
            stream: None,
        }
    }

    pub fn flags(&self) -> u8 {
        self.flags
    }
    pub fn stream_open(&self) -> bool {
        self.stream.is_some()
    }
    pub fn recorded_len(&self) -> usize {
        self.recorded_len as usize
    }
    /// The recorded bytes (the stream's contents up to the recorded length).
    pub fn recorded(&self) -> &[u8] {
        &self.data[..self.recorded_len as usize]
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    /// The current frame (24 bytes; only the first `frame size` are used).
    pub fn cur(&self) -> &[u8; 24] {
        &self.cur
    }
    pub fn cur_mut(&mut self) -> &mut [u8; 24] {
        &mut self.cur
    }
    /// A frame writer appends one byte at the cursor.
    pub fn put(&mut self, b: u8) {
        self.cur[self.cursor] = b;
        self.cursor += 1;
    }
    /// A frame reader takes the byte at the cursor.
    pub fn get(&mut self) -> u8 {
        let b = self.cur[self.cursor];
        self.cursor += 1;
        b
    }
    /// Put a whole frame into the current frame as the writers do (byte 0 directly, the rest through the cursor).
    pub fn put_frame(&mut self, f: &ReplayFrame) {
        let b = f.to_bytes();
        self.cur[0] = b[0];
        for &x in &b[1..] {
            self.put(x);
        }
    }
    /// The current frame as the readers see it (after a successful begin-frame in playback).
    pub fn frame(&self) -> ReplayFrame {
        let mut b = [0u8; FRAME_SIZE];
        b.copy_from_slice(&self.cur[..FRAME_SIZE]);
        ReplayFrame::from_bytes(&b)
    }

    /// BeginRecord(n): the frame is n + 1 bytes (the original always passes 20); both frame copies are cleared and a fresh
    /// stream opens at position 0. The data block is not cleared.
    pub fn begin_record(&mut self, n: usize) {
        assert!(n + 1 <= 24);
        self.cursor = 1;
        self.frame_size = n + 1;
        self.flags |= BUF_RECORDING;
        self.cur = [0; 24];
        self.prev = [0; 24];
        self.stream = Some(0);
    }

    /// BeginPlayback: the frame size and recorded length stay from the recording.
    pub fn begin_playback(&mut self) {
        self.cursor = 1;
        self.cur = [0; 24];
        self.flags |= BUF_PLAYING;
        self.stream = Some(0);
    }

    fn read_stream_byte(&mut self) -> u8 {
        // Inside the recorded length the original reads straight from the block; past it the stream's underflow asks the
        // (absent) device and the byte becomes 0xFF - not reachable with data the recorder wrote [not exercised].
        let pos = self.stream.as_mut().expect("replay stream is closed");
        let b = if *pos < self.recorded_len as usize { self.data[*pos] } else { 0xFF };
        *pos += 1;
        b
    }

    /// Begin-frame, called every unpaused frame. Playing: false at the end of the recorded data, otherwise reads the next
    /// frame into the current frame (raw: the whole frame; delta: the mask bytes, then one byte per set bit, LSB first;
    /// the other bytes keep their previous value). In every case the cursor goes back to 1 and the result is true.
    pub fn begin_frame(&mut self) -> bool {
        if self.flags & BUF_PLAYING != 0 {
            let pos = self.stream.expect("replay stream is closed");
            if pos as i32 >= self.recorded_len {
                return false;
            }
            let fs = self.frame_size;
            if !self.delta {
                for i in 0..fs {
                    self.cur[i] = self.read_stream_byte();
                }
                self.cursor = 1;
                return true;
            }
            let nmask = (fs + 7) >> 3;
            let mut mask = [0u8; 4];
            for m in mask.iter_mut().take(nmask) {
                *m = self.read_stream_byte();
            }
            for g in 0..nmask {
                for k in 0..8 {
                    if mask[g] & (1 << k) != 0 {
                        self.cur[8 * g + k] = self.read_stream_byte();
                    }
                }
            }
        }
        self.cursor = 1;
        true
    }

    /// Write the current frame at the end of the frame. True when there was nothing to write (cursor still 1) or the buffer
    /// is not recording; false (refused) when the frame would not fit (`position + frame size >= block size`), which ends
    /// the recording. The previous-frame copy is refreshed only after a real write; the cursor is not reset.
    pub fn write_frame(&mut self) -> bool {
        if self.cursor == 1 || self.flags & BUF_RECORDING == 0 {
            return true;
        }
        let fs = self.frame_size;
        let pos = self.stream.expect("replay stream is closed");
        if pos as i32 + fs as i32 >= self.block_size {
            return false;
        }
        let mut at = pos;
        let mut out = |b: u8, data: &mut Vec<u8>| {
            data[at] = b;
            at += 1;
        };
        if !self.delta {
            for i in 0..fs {
                out(self.cur[i], &mut self.data);
            }
        } else {
            let nmask = (fs + 7) >> 3;
            for g in 0..nmask {
                let mut m = 0u8;
                for k in 0..8 {
                    if self.cur[8 * g + k] != self.prev[8 * g + k] {
                        m |= 1 << k;
                    }
                }
                out(m, &mut self.data);
            }
            for i in 0..fs {
                if self.cur[i] != self.prev[i] {
                    out(self.cur[i], &mut self.data);
                }
            }
        }
        self.stream = Some(at);
        self.prev = self.cur;
        true
    }

    /// StopRecord: clear the recording bit, flush (a no-op now), remember the length and close the stream.
    pub fn stop_record(&mut self) {
        self.flags &= 0x7F;
        if let Some(pos) = self.stream {
            self.recorded_len = pos as i32;
            self.stream = None;
        }
    }

    /// StopPlayback: clear the playing bit and close the stream.
    pub fn stop_playback(&mut self) {
        self.flags &= !BUF_PLAYING;
        self.stream = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }
    /// A hex word of the test data; 8-digit words carry an `h` prefix so that they never look like addresses.
    fn w(s: &str) -> u32 {
        u32::from_str_radix(s.strip_prefix('h').unwrap_or(s), 16).unwrap()
    }

    fn input_words(i: &ReplayInput) -> [u32; 9] {
        [i.throttle.to_bits(), i.brake.to_bits(), i.handbrake.to_bits(), i.steer_raw.to_bits(), i.axis_28.to_bits(), i.stick_y.to_bits(),
         i.stick_x.to_bits(), i.flags, i.word_80]
    }

    /// 9,012 cases of the original's quantiser and unpacker (random bit patterns, quantisation boundaries, NaN / inf / huge,
    /// every code): the 12 bytes, the cursor, the seven written-back floats and the two words, all bit for bit.
    #[test]
    fn input_quantiser_matches_the_original() {
        let mut nq = 0;
        let mut nu = 0;
        for line in include_str!("testdata/replay_quant_ref.txt").lines() {
            let (inp, out) = line.split_once(" => ").unwrap();
            let t: Vec<&str> = inp.split_whitespace().collect();
            let o: Vec<&str> = out.split_whitespace().collect();
            // o: kind, bytes, cursor, 7 words, +40, +80
            let mut v = ReplayInput::default();
            let bytes;
            if t[0] == "Q" {
                let f = |i: usize| f32::from_bits(w(t[i]));
                v = ReplayInput { throttle: f(1), brake: f(2), handbrake: f(3), steer_raw: f(4), axis_28: f(5), stick_y: f(6), stick_x: f(7),
                                  flags: w(t[8]), word_80: w(t[9]) };
                bytes = v.quantise();
                nq += 1;
            } else {
                let mut b = [0u8; INPUT_BYTES];
                for (k, tok) in t[1..4].iter().enumerate() {
                    b[4 * k..4 * k + 4].copy_from_slice(&w(tok).to_be_bytes());
                }
                v.unpack(&b);
                bytes = b;
                nu += 1;
            }
            assert_eq!(hex(o[1]), bytes.to_vec(), "{line}");
            assert_eq!(o[2], "21", "the cursor ends at the frame end");
            let got = input_words(&v);
            for k in 0..9 {
                assert_eq!(w(o[3 + k]), got[k], "field {k}: {line}");
            }
        }
        assert_eq!((nq, nu), (5256, 3756));
    }

    fn parse_racers(toks: &[&str]) -> Vec<ParityRacer> {
        toks.iter()
            .map(|e| {
                let p: Vec<u32> = e.split(':').map(w).collect();
                ParityRacer { present: p[0] != 0, state: p[1] as i32, active_call: p[2], pos_bits: [p[3], p[4], p[5]] }
            })
            .collect()
    }

    /// 6,000 racer lists (either list layout, null entries, active by state word or by the virtual call's low byte, random
    /// position bits, counts <= 0): the parity bit equals the original's.
    #[test]
    fn position_parity_matches_the_original() {
        let mut n = 0;
        let mut ones = 0;
        for line in include_str!("testdata/replay_parity_ref.txt").lines() {
            let (inp, out) = line.split_once(" => ").unwrap();
            let t: Vec<&str> = inp.split_whitespace().collect();
            let count: i32 = t[1].parse().unwrap();
            let racers = parse_racers(&t[3..]);
            let got = position_parity(count, &racers);
            assert_eq!(got as u32, out.trim().parse::<u32>().unwrap(), "{line}");
            ones += got as u32;
            n += 1;
        }
        assert_eq!(n, 6000);
        assert!(ones > 1000);
    }

    /// 3,000 frame ticks (record, playback, neither, both flags; random byte 0 and manager flags): frame byte 0 and the
    /// manager's flag byte after the parity step equal the original's.
    #[test]
    fn frame_parity_step_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/replay_tick_ref.txt").lines() {
            let (inp, out) = line.split_once(" => ").unwrap();
            let t: Vec<&str> = inp.split_whitespace().collect();
            let flags = w(t[0]) as u8;
            let mut b0 = w(t[1]) as u8;
            let mut mgr = w(t[2]) as u8;
            let count: i32 = t[4].parse().unwrap();
            let racers = parse_racers(&t[5..]);
            frame_parity_step(flags, &mut b0, &mut mgr, || position_parity(count, &racers));
            let o: Vec<&str> = out.split_whitespace().collect();
            assert_eq!((w(o[0]) as u8, w(o[1]) as u8), (b0, mgr), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// 433 record / playback sequences on the original's buffer functions (raw and delta mode, frame sizes 2..24, block sizes
    /// 21..8192, "nothing written" frames, runs that fill the block and are refused): the write results, the recorded length,
    /// every stream byte, the flags and stream state after stop, and every frame read back, bit for bit.
    #[test]
    fn replay_buffer_matches_the_original() {
        let mut n = 0;
        let mut refused = 0;
        for line in include_str!("testdata/replay_buffer_ref.txt").lines() {
            let (inp, out) = line.split_once(" => ").unwrap();
            let t: Vec<&str> = inp.split_whitespace().collect();
            let mode: u32 = t[0].parse().unwrap();
            let block: usize = t[1].parse().unwrap();
            let fs: usize = t[2].parse().unwrap();
            let keep: usize = t[3].parse().unwrap();
            let mut buf = ReplayBuffer::new(block, mode == 1);
            buf.begin_record(fs - 1);
            let mut res = String::new();
            for tok in &t[4..4 + keep] {
                assert!(buf.begin_frame());
                if let Some(h) = tok.strip_prefix('w') {
                    let b = hex(h);
                    buf.cur_mut()[0] = b[0];
                    for &x in &b[1..] {
                        buf.put(x);
                    }
                    assert_eq!(buf.cursor(), fs);
                }
                let ok = buf.write_frame();
                res.push(if ok { '1' } else { '0' });
                if !ok {
                    break;
                }
            }
            buf.stop_record();
            let o: Vec<&str> = out.split_whitespace().collect();
            // o: res= len= data= flags= str= reads=... flags2= str=
            assert_eq!(o[0], format!("res={res}"), "{line}");
            assert_eq!(o[1], format!("len={}", buf.recorded_len()), "{line}");
            let want: String = buf.recorded().iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(o[2], format!("data={want}"), "{line}");
            assert_eq!(o[3], format!("flags={:02x}", buf.flags()), "{line}");
            assert_eq!(o[4], format!("str={}", if buf.stream_open() { "1" } else { "0" }));
            refused += res.ends_with('0') as u32;
            buf.begin_playback();
            let reads_at = o.iter().position(|s| s.starts_with("reads=")).unwrap();
            let flags2_at = o.iter().position(|s| s.starts_with("flags2=")).unwrap();
            let mut exp: Vec<String> = o[reads_at..flags2_at].iter().map(|s| s.to_string()).collect();
            exp[0] = exp[0].trim_start_matches("reads=").to_string();
            let mut got: Vec<String> = Vec::new();
            for _ in 0..res.len() + 3 {
                let ok = buf.begin_frame();
                got.push(format!("{}:{}", ok as u32, buf.cur()[..fs].iter().map(|b| format!("{b:02x}")).collect::<String>()));
                if !ok {
                    break;
                }
            }
            assert_eq!(got, exp, "{line}");
            buf.stop_playback();
            assert_eq!(o[flags2_at], format!("flags2={:02x}", buf.flags()));
            n += 1;
        }
        assert_eq!(n, 433);
        assert_eq!(refused, 193);
    }

    #[test]
    fn frame_layout() {
        let mut inp = [0u8; INPUT_BYTES];
        for (i, b) in inp.iter_mut().enumerate() {
            *b = 0x90 + i as u8;
        }
        let f = ReplayFrame { cine_skip: true, speech: false, parity: true, time_scale: 1.0, frame_time: 0.5, input: inp };
        let b = f.to_bytes();
        assert_eq!(&b[..9], &[0x05, 0x3f, 0x80, 0, 0, 0x3f, 0x00, 0, 0]);
        assert_eq!(&b[9..], &inp[..]);
        assert_eq!(ReplayFrame::from_bytes(&b), f);
        // the frame goes through the buffer the way the writers fill it: byte 0, then bytes 1..20 through the cursor
        let mut buf = ReplayBuffer::new(8192, true);
        buf.begin_record(20);
        assert!(buf.begin_frame());
        buf.put_frame(&f);
        assert_eq!(buf.cursor(), FRAME_SIZE);
        assert!(buf.write_frame());
        buf.stop_record();
        buf.begin_playback();
        assert!(buf.begin_frame());
        assert_eq!(buf.frame(), f);
        assert!(!buf.begin_frame());
    }
}
