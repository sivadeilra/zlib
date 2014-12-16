/* inffast.c -- fast decoding
 * Copyright (C) 1995-2008, 2010, 2013 Mark Adler
 * For conditions of distribution and use, see copyright notice in zlib.h
 */

use ZStream;
use super::Code;
use super::InflateState;
use super::InflateMode;
use std::slice::bytes::copy_memory;
use std::char;

pub struct BufPos<'a> {
    pub buf: &'a [u8],
    pub pos: uint
}

impl<'a> BufPos<'a> {
    pub fn read(&mut self) -> u8 {
        let b = self.buf[self.pos];
        self.pos += 1;
        b
    }
}

struct BufPosMut<'a> {
    buf: &'a mut [u8],
    pos: uint
}

impl<'a> BufPosMut<'a> {
    pub fn write(&mut self, b: u8) {
        self.buf[self.pos] = b;
        self.pos += 1;
    }
}

struct InputState<'a> {
    pub buf: &'a [u8],
    pub pos: uint,
    pub hold: u32,
    pub bits: uint,
}

impl<'a> InputState<'a> {
    pub fn load_byte(&mut self) {
        let b = self.buf[self.pos];
        self.pos += 1;
        self.hold += (b as u32) << self.bits;
        self.bits += 8;
        debug!("loaded 0x{:02x}, bits = {:2}, hold = 0x{:08x}", b, self.bits, self.hold);
    }
    pub fn drop_bits(&mut self, n: uint) {
        self.bits -= n;
        self.hold >>= n;
        debug!("dropped {} bits, bits = {:2}, hold = 0x{:08x}", n, self.bits, self.hold);
    }
}

// Decode literal, length, and distance codes and write out the resulting
// literal and match bytes until either not enough input or output is
// available, an end-of-block is encountered, or a data error is encountered.
// When large enough input and output buffers are supplied to inflate(), for
// example, a 16K input buffer and a 64K output buffer, more than 95% of the
// inflate execution time is spent in this routine.
//
// Entry assumptions:
//
//      state.mode == LEN
//      strm.avail_in >= 6
//      strm.avail_out >= 258
//      start >= strm.avail_out
//      state.bits < 8
//
// On return, state.mode is one of:
//
//      LEN -- ran out of enough output space or enough available input
//      TYPE -- reached end of block code, inflate() to interpret next block
//      BAD -- error in block data
//
// Notes:
//
//  - The maximum input bits used by a length/distance pair is 15 bits for the
//    length code, 5 bits for the length extra, 15 bits for the distance code,
//    and 13 bits for the distance extra.  This totals 48 bits, or six bytes.
//    Therefore if strm.avail_in >= 6, then there is enough input to avoid
//    checking for available input while decoding.
//
//  - The maximum bytes that a single length/distance pair can output is 258
//    bytes, which is the maximum length that can be coded.  inflate_fast()
//    requires strm.avail_out >= 258 for each loop to avoid checking for
//    output space.
//

fn copy_within_output_buffer(buf: &mut [u8], dstpos: uint, srcpos: uint, len: uint)
{
    debug!("srcpos = {}, dstpos = {}, len = {}", srcpos, dstpos, len);
    // the source region must be at lower indices than the dest region

    debug_assert!(srcpos <= dstpos);

    let src_end = srcpos + len;

    if src_end <= dstpos {
        // non-overlapping copy -- easy
        debug_assert!(srcpos + len <= dstpos);

        let (src_split, dst_split) = buf.split_at_mut(dstpos);
        debug!("src_split.len() = {}, dst_split.len() = {}", src_split.len(), dst_split.len());
        let src_buf = src_split.slice(srcpos, srcpos + len);
        debug!("src_buf.len() = {}", src_buf.len());
        copy_memory(dst_split, src_buf);
    }
    else {
        // overlapping copy -- do it the hard way
        for i in range(0, len) {
            buf[dstpos + i] = buf[srcpos + i];
        }
    }
}

#[deriving(Show)]
enum InflateFastState {
    Start,
    DoDist,
    DoLen
}

pub fn inflate_fast(
    state: &mut InflateState,
    strm: &mut ZStream,
    input_buffer: &[u8],
    output_buffer: &mut [u8],
    strm_next_in: &mut uint,
    strm_avail_in: &mut uint,
    strm_next_out: &mut uint,
    strm_avail_out: &mut uint,
    start: uint)                 /* inflate()'s starting value for strm.avail_out */
{
    // copy state to local variables
    let mut out = BufPosMut { buf: output_buffer, pos: *strm_next_out };
    let beg :uint = out.pos - (start - *strm_avail_out);     // inflate()'s initial strm.next_out
    let end :uint = out.pos + (*strm_avail_out - 257);       // while out < end, enough space available
// #ifdef INFLATE_STRICT
    let dmax: uint = state.dmax;                    // maximum distance from zlib header
// #endif

    let wsize: uint = state.wsize;                  // window size or zero if not using window
    let whave: uint = state.whave;                  // valid bytes in the window
    let wnext: uint = state.wnext;                  // window write index
    let window = state.window.as_slice();           // allocated sliding window, if wsize != 0

    let codes = &mut state.codes;                   // local strm.codes
    let lcode: uint = state.lencode;                // local strm.lencode; is index into 'codes'
    let dcode: uint = state.distcode;               // local strm.distcode; is index into 'codes'
    let lmask: u32 = (1 << state.lenbits) - 1;      // mask for first level of length codes
    let dmask: u32 = (1 << state.distbits) - 1;     // mask for first level of distance codes

    Tracevv!("total_in: {}", strm.total_in);

    debug!("wsize = {}, window.len = {}", wsize, window.len());
    // assert!(wsize == window.len());

    let mut input = InputState {
        buf: input_buffer,
        pos: *strm_next_in,
        hold: state.hold,
        bits: state.bits,
    };
    let last: uint = input.pos + (*strm_avail_in - 5);     // (index into input_buffer) have enough input while in < last

    debug!("starting inflate_fast");
    debug!("    total_in: {}, next_in: {}", strm.total_in, *strm_next_in);
    debug!("    wsize = {}, whave = {}, wnext = {}, window buffer size = {}", wsize, whave, wnext, window.len());
    debug!("    hold = 0x{:08x}, bits = {}", input.hold, input.bits);
    debug!("    beg = {}, end = {}", beg, end);
    debug!("    input: pos = {}, last = {}", input.pos, last);

    // we use 'st' to simulate gotos
    let mut st = InflateFastState::Start;

    let mut len: uint = 0;

    let mut here: Code;         // retrieved table entry
    here = Code { op: 0, bits: 0, val: 0 }; // cannot prove this is unused yet

    if input.pos >= last {
        debug!("HEY! input is ALREADY at an end!");
    }

    if out.pos >= end {
        debug!("HEY! output is ALREADY at an end");
    }

    Tracevv!("initial: last - in = {}", (last as int) - (input.pos as int));

    // decode literals and length/distances until end-of-block or not enough
    // input data or output space
    loop {
        debug!("main loop - st = {}", st);
        st = match st {
            InflateFastState::Start => {
                if input.bits < 15 {
                    input.load_byte();
                    input.load_byte();
                }
                here = codes[(lcode + (input.hold & lmask) as uint) as uint];
                debug!("(start): here := op: {}, bits: {}, val: {}", here.op, here.bits, here.val);
                InflateFastState::DoLen
            }

            InflateFastState::DoLen => {
              //dolen:
                input.drop_bits(here.bits as uint);
                let op = here.op as uint;
                if op == 0 {
                    // literal
                    // debug!("(dolen): consumed {:2} bits, {:2} bits left, output literal byte: 0x{:2x}", here.bits, input.bits, here.val);

                    if here.val >= 0x20 && here.val < 0x7f {
                        Tracevv!("inflate:         literal '{}'", char::from_u32(here.val as u32).unwrap());
                    }
                    else {
                        Tracevv!("inflate:         literal 0x{:02x}", here.val);
                    }

                    out.write(here.val as u8); // truncate u16 to u8
                }
                else if (op & 16) != 0 {
                    // length base
                    len = here.val as uint; // match length; used in DoDist
                    let extra_bits = op & 15; // number of extra bits
                    if extra_bits != 0 {
                        if input.bits < extra_bits {
                            input.load_byte();
                        }
                        let more_len = (input.hold & ((1 << extra_bits) - 1)) as uint;
                        len += more_len;
                        input.drop_bits(extra_bits);
                        // debug!("    used {} extra bits to decode {} more length", extra_bits, more_len);
                    }
                    Tracevv!("inflate:         length {}", len);
                    // debug!("(dolen): consumed {:2} bits, {:2} bits left, length = {}", here.bits, input.bits, len);
                    if input.bits < 15 {
                        input.load_byte();
                        input.load_byte();
                    }
                    here = codes[dcode + (input.hold & dmask) as uint];
                    // debug!("st := DoDist");
                    st = InflateFastState::DoDist;
                    continue;
                }
                else if (op & 64) == 0 {
                    // 2nd level length code
                    here = codes[lcode + (here.val as uint + (input.hold as uint & ((1 << op) - 1)))];
                    // debug!("second level length code");
                    st = InflateFastState::DoLen;
                    continue;
                }
                else if (op & 32) != 0 {
                    // end-of-block
                    // debug!("inflate: end of block");
                    Tracevv!("inflate:         end of block");
                    state.mode = InflateMode::TYPE;
                    break;
                }
                else {
                    strm.msg = Some("invalid literal/length code".to_string());
                    state.mode = InflateMode::BAD;
                    break;
                }

                if input.pos >= last {
                    debug!("reached end of input; halting");
                    break;
                }
                if out.pos >= end {
                    debug!("reached end of output; halting");
                    break;
                }
                InflateFastState::Start
            }

            InflateFastState::DoDist => {
              //dodist:
                let distbits = here.bits as uint;
                input.drop_bits(distbits);
                let op = here.op as uint;
                // debug!("(dodist): used {} bits ({} bits left), op = 0x{:x}", distbits, input.bits, here.op);
                if (op & 16) != 0 {
                    // distance base
                    let distbase = here.val as uint;
                    // debug!("distance base = {}", distbase);
                    let extra_bits: uint = op & 15; // number of extra bits
                    if input.bits < extra_bits {
                        input.load_byte();
                        if input.bits < extra_bits {
                            input.load_byte();
                        }
                    }
                    let dist = distbase + (input.hold as uint & ((1 << extra_bits) - 1));
    // #ifdef INFLATE_STRICT
                    if dist > dmax {
                        debug!("invalid distance, too far back.  dist {} > dmax {}", dist, dmax);
                        strm.msg = Some("invalid distance too far back".to_string());
                        state.mode = InflateMode::BAD;
                        break;
                    }
    // #endif
                    input.drop_bits(extra_bits);

                    // maxout is the maximum number of bytes that are available in the output buffer
                    // for use as a source operand for window copies.  since the same data is in the
                    // window as in the output buffer (i think?), it is easier to copy within the
                    // output buffer, rather than dealing with wrap-around in the window buffer.
                    let mut maxout = out.pos - beg;     /* max distance in output */
                    debug!("inflate: distance = {}, maxout = {}", dist, maxout);
                    Tracevv!("inflate:         distance {}", dist);
                    Tracevv!("maxout: {}", maxout);
                    if dist > maxout {
                        // The distance exceeds the data that is stored within the output buffer.
                        // We still may be able to copy some data from the output buffer, but we
                        // will need to copy at least some of it from the window.

                        /* see if copy from window */
                        maxout = dist - maxout; /* distance back in window */
                        Tracevv!("maxout = dist - maxout = {}", maxout);
                        if maxout > whave {
                            if state.sane {
                                strm.msg = Some("invalid distance too far back".to_string());
                                state.mode = InflateMode::BAD;
                                break;
                            }
    /*#ifdef INFLATE_ALLOW_INVALID_DISTANCE_TOOFAR_ARRR
                            if (len <= maxout - whave) {
                                while len > 0 {
                                    out.write(0);
                                    len -= 1;
                                }
                                continue;
                            }
                            len -= maxout - whave;
                            loop {
                                out.write(0);
                                maxout -= 1;
                                if maxout <= whave {
                                    break;
                                }
                            }
                            if maxout == 0 {
                                let mut from = BufPos { buf: out.buf, pos: out.pos - dist };
                                while len > 0 {
                                    out.write(from.read());
                                    len -= 1;
                                }
                                continue;
                            }
    #endif*/
                        }

                        // Next, decide what we are going to copy to the output.

                        fn copy_to_output(out: &mut BufPosMut, from: &mut BufPos, len: &mut uint)
                        {
                            while *len > 0 {
                                out.write(from.read());
                                *len -= 1;
                            }
                        }

                        fn copy_to_output_from_history(buf: &mut [u8], pos: uint, dist: uint, len: uint)
                        {
                            assert!(dist <= pos);
                            assert!(dist + len <= pos);

                            // Because we need to copy within a single buffer, we have to split it.
                            let (src_buf, dst_buf) = buf.split_at_mut(pos);
                            let src_buf = src_buf.slice(src_buf.len() - dist, len);
                            let dst_buf = dst_buf.slice_mut(0, len);
                            copy_memory(dst_buf, src_buf);
                        }

                        let mut from = BufPos { buf: window, pos: 0 };
                        if wnext == 0 {
                            // very common case
                            Tracevv!("wnext = 0");
                            debug!("(common) wnext = 0, wsize = {}, maxout = {}, len = {}", wsize, maxout, len);
                            if maxout < len {
                                // some from window
                                Tracevv!("some from window, len = {}", len);
                                debug!("copying some from window, out.pos = {}, window pos = {}, length = {}", out.pos, wsize - maxout, maxout);
                                len -= maxout;
                                copy_memory(
                                    out.buf.slice_mut(out.pos, out.pos + maxout),
                                    window.slice(wsize - maxout, wsize));
                                out.pos += maxout;

                                // copy the rest from the output buffer
                                debug!("copying within output buffer, out.pos (dst) = {}, dist = {}, out.src = {}, len = {}", out.pos, dist, out.pos - dist, len);
                                copy_within_output_buffer(out.buf, out.pos, out.pos - dist, len);
                            }
                            else {
                                let wpos = wsize - maxout;
                                debug!("copying all from window, out.pos = {}, window.len() = {}, window pos = {}, len = {}", out.pos, window.len(), wpos, len);
                                let d = out.buf.slice_mut(out.pos, out.pos + len);
                                let s = window.slice(wpos, wpos + len);
                                copy_memory(d, s);
                                out.pos += len;
                            }
                        }
                        else if wnext < maxout {
                            // wrap around window
                            debug!("wrap around window");
                            Tracevv!("wrap around window, wnext: {}, op: {}, advancing from by {}", wnext, maxout, wsize + wnext - maxout);
                            from.pos += wsize + wnext - maxout;
                            maxout -= wnext;
                            if maxout < len {
                                /* some from end of window */
                                debug!("some from end of window");
                                len -= maxout;
                                while maxout > 0 {
                                    out.write(from.read());
                                    maxout -= 1;
                                }
                                from = BufPos { buf: window, pos: 0 };
                                if wnext < len {
                                    // some from start of window
                                    debug!("some from start of window");
                                    maxout = wnext;
                                    len -= maxout;
                                    copy_memory(
                                        out.buf.slice_mut(out.pos, out.pos + maxout),
                                        from.buf.slice(from.pos, from.pos + maxout));
                                    out.pos += maxout;

                                    /* rest from output */
                                    copy_within_output_buffer(out.buf, out.pos, out.pos - dist, len);
                                }
                                else {
                                    // copy from 'from' to output
                                    copy_memory(out.buf.slice_mut(out.pos, out.pos + len), window.slice(from.pos, from.pos + len));
                                }
                            }
                            else {
                                // copy from window ('from') to output
                                copy_memory(out.buf.slice_mut(out.pos, out.pos + len), window.slice(from.pos, from.pos + len));
                            }
                        }
                        else {
                            // contiguous in window
                            debug!("contiguous in window, advancing {}", wnext - maxout);
                            Tracevv!("contiguous in window, from += {}", wnext - maxout);

                            from.pos += wnext - maxout;
                            if maxout < len {
                                // some from window
                                len -= maxout;
                                while maxout > 0 {
                                    out.write(from.read());
                                    maxout -= 1;
                                }
                                /* rest from output */
                                copy_within_output_buffer(out.buf, out.pos, out.pos - dist, len);
                                out.pos += len;
                            }
                            else {
                                // todo: make faster
                                while len > 0 {
                                    out.write(from.read());
                                    len -= 1;
                                }
                            }
                        }
                    }
                    else {
                        // All of the data that we need to copy can already be found in the output buffer.
                        // Now we just need to locate it and do the copy.
                        //      dist - distance back
                        //      len - length of region to copy
                        //      (out.pos - dist + len) <= out.pos
                        // assert!(out.pos - dist + len <= out.pos);
                        // assert!(len - dist <= 0);
                        debug!("copy direct from output, dist = {}, out.pos = {}, src pos = {}, len = {}", dist, out.pos, out.pos - dist, len);
                        // assert!(dist >= len);

                        // copy direct from output
                        debug_assert!(len >= 3); // minimum length is three

                        Tracevv!("copy direct from output, len: {}", len);

                        // note: this can legally be a self-overlapping copy!
                        while len > 0 {
                            out.buf[out.pos] = out.buf[out.pos - dist];
                            out.pos += 1;
                            len -= 1;
                        }

                        Tracevv!("out.pos: {}, input.pos: {}, bits: {}, hold: 0x{:08x}", out.pos - *strm_next_out, input.pos - *strm_next_in, input.bits, input.hold);
                    }
                }
                else if (op & 64) == 0 {
                    // 2nd level distance code
                    debug!("second-level distance code");
                    here = codes[dcode + (here.val as uint + (input.hold & ((1 << op) - 1)) as uint)];
                    Tracevv!("second level distance code, op {} bits {} val {}", here.op, here.bits, here.val);
                    st = InflateFastState::DoDist;
                    continue;
                }
                else {
                    strm.msg = Some("invalid distance code".to_string());
                    state.mode = InflateMode::BAD;
                    break;
                }
                InflateFastState::Start
            }
        };

        match st {
            InflateFastState::Start => {
                Tracevv!("last - in = {}", (last as int) - (input.pos as int));
                if input.pos >= last {
                    debug!("breaking loop (end of input)");
                    Tracevv!("stopping (no input)");
                    break;
                }

                if out.pos >= end {
                    debug!("breaking loop (end of output)");
                    Tracevv!("stopping (no output)");
                    break;
                }
            }
            _ => {}
        }
    }

    debug!("done.");
    debug!("    input: pos = {}, last = {}", input.pos, last);

    // return unused bytes (on entry, bits < 8, so in won't go too far back)
    let len = input.bits >> 3;
    input.pos -= len;
    input.bits -= len << 3;
    input.hold &= (1 << input.bits) - 1;

    // we expect to have advanced state
    debug!("    input.pos = {}, strm.next_in = {}", input.pos, *strm_next_in);
    assert!(input.pos > *strm_next_in || out.pos > *strm_next_out);

    // update state and return
    *strm_next_in = input.pos;
    *strm_next_out = out.pos;
    *strm_avail_in = if input.pos < last { 5 + (last - input.pos) } else { 5 - (input.pos - last) };
    *strm_avail_out = if out.pos < end { 257 + (end - out.pos) } else { 257 - (out.pos - end) };
    state.hold = input.hold;
    state.bits = input.bits;

    debug!("done.  strm {{ next_in: {}, next_out: {}, avail_in: {}, avail_out: {} }}",
        *strm_next_in,
        *strm_next_out,
        *strm_avail_in,
        *strm_avail_out);

    Tracevv!("done.  avail_in: {}, avail_out: {}", strm.avail_in, strm.avail_out);
}

/*
   inflate_fast() speedups that turned out slower (on a PowerPC G3 750CXe):
   - Using bit fields for code structure
   - Different op definition to avoid & for extra bits (do & for table bits)
   - Three separate decoding do-loops for direct, window, and wnext == 0
   - Special case for distance > 1 copies to do overlapped load and store copy
   - Explicit branch predictions (based on measured branch probabilities)
   - Deferring match copy and interspersed it with decoding subsequent codes
   - Swapping literal/length else
   - Swapping window/direct else
   - Larger unrolled copy loops (three is about right)
   - Moving len -= 3 statement into middle of loop
 */

//#endif /* !ASMINF */
