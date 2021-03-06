
// Copyright (C) 1995-2009 Mark Adler
// For conditions of distribution and use, see copyright notice in zlib.h

// http://www.gzip.org/zlib/rfc-gzip.html

use std::slice::bytes::copy_memory;
use std::iter::repeat;

use crc32::crc32;
use adler32::adler32;
use self::inffast::inflate_fast;
use self::inffast::BufPos;
use self::inftrees::{Code, ENOUGH, CODES, LENS, DISTS, inflate_table};
use std::default::Default;
use GZipHeader;
use ZStream;
use swap32;
use Flush;
use Z_DEFLATED;
use WINDOW_BITS_DEFAULT;
use WINDOW_BITS_MIN;
use WINDOW_BITS_MAX;

pub use self::reader::InflateReader;

const DEFAULT_DMAX: usize = 32768;

mod inffast;
mod inftrees;
mod reader;
mod inffixed;

macro_rules! BADINPUT {
    ($loc:expr, $msg:expr) => {
        {
            warn!("bad input, total_in={}: {}", $loc.state.strm.total_in, $msg);
            $loc.state.strm.msg = Some($msg);
            $loc.state.mode = InflateMode::BAD;
            return;
        }
    }
}

// Get a byte of input into the bit accumulator, or return from inflate
// if there is no input available.
macro_rules! PULLBYTE {
    ($loc:expr) => {
        {
            if $loc.have() == 0 {
                debug!("PULLBYTE: have=0, inf_leave");
                return;
            }
            let b = $loc.input_buffer[$loc.next];
            $loc.hold += (b as u32) << $loc.bits;
            $loc.next += 1;
            $loc.bits += 8;
        }
    }
}

// Assure that there are at least n bits in the bit accumulator.  If there is
// not enough available input to do that, then return from inflate(). */
macro_rules! NEEDBITS {
    ($loc:expr, $n:expr) => {
        {
            let n :usize = $n;
            while $loc.bits < n {
                PULLBYTE!($loc);
            }
        }
    }
}

/// Describes the results of calling `inflate()`.
#[derive(Copy)]
pub enum InflateResult
{
    Eof(u32),               // input data stream has reached its end; value is crc32 of stream
    NeedInput,              // could decode more, but need more input buffer space
    Decoded(usize, usize),    // decoded N bytes of input, wrote N bytes of output
    InvalidData,            // input data is malformed, decoding has halted
}

// /* define NO_GZIP when compiling if you want to disable gzip header and
//    trailer decoding by inflate().  NO_GZIP would be used to avoid linking in
//    the crc code when it is not needed.  For shared libraries, gzip decoding
//    should be left enabled. */
// #ifndef NO_GZIP
// #  define GUNZIP
// #endif
// */

/* Possible inflate modes between inflate() calls */
#[derive(Show,Copy,PartialEq,Eq)]
enum InflateMode {
    HEAD,       // i: waiting for magic header
    FLAGS,      // i: waiting for method and flags (gzip)
    TIME,       // i: waiting for modification time (gzip)
    OS,         // i: waiting for extra flags and operating system (gzip)
    EXLEN,      // i: waiting for extra length (gzip)
    EXTRA,      // i: waiting for extra bytes (gzip)
    NAME,       // i: waiting for end of file name (gzip)
    COMMENT,    // i: waiting for end of comment (gzip)
    HCRC,       // i: waiting for header crc (gzip)
    DICTID,     // i: waiting for dictionary check value
    DICT,       // waiting for inflateSetDictionary() call
        TYPE,       // i: waiting for type bits, including last-flag bit
        TYPEDO,     // i: same, but skip check to exit inflate on new block
        STORED,     // i: waiting for stored size (length and complement)
        COPY_,      // i/o: same as COPY below, but only first time in
        COPY,       // i/o: waiting for input or output to copy stored block
        TABLE,      // i: waiting for dynamic block table lengths
        LENLENS,    // i: waiting for code length code lengths
        CODELENS,   // i: waiting for length/lit and distance code lengths
            LEN_,       // i: same as LEN below, but only first time in
            LEN,        // i: waiting for length/lit/eob code
            LENEXT,     // i: waiting for length extra bits
            DIST,       // i: waiting for distance code
            DISTEXT,    // i: waiting for distance extra bits
            MATCH,      // o: waiting for output space to copy string
            LIT,        // o: waiting for output space to write literal
    CHECK,      // i: waiting for 32-bit check value
    LENGTH,     // i: waiting for 32-bit length (gzip)
    DONE,       // finished check, done -- remain here until reset
    BAD,        // got a data error -- remain here until reset
    MEM,        // got an inflate() memory error -- remain here until reset
    SYNC        // looking for synchronization bytes to restart inflate()
}

/*
    State transitions between above modes -

    (most modes can go to BAD or MEM on error -- not shown for clarity)

    Process header:
        HEAD -> (gzip) or (zlib) or (raw)
        (gzip) -> FLAGS -> TIME -> OS -> EXLEN -> EXTRA -> NAME -> COMMENT ->
                  HCRC -> TYPE
        (zlib) -> DICTID or TYPE
        DICTID -> DICT -> TYPE
        (raw) -> TYPEDO
    Read deflate blocks:
            TYPE -> TYPEDO -> STORED or TABLE or LEN_ or CHECK
            STORED -> COPY_ -> COPY -> TYPE
            TABLE -> LENLENS -> CODELENS -> LEN_
            LEN_ -> LEN
    Read deflate codes in fixed or dynamic block:
                LEN -> LENEXT or LIT or TYPE
                LENEXT -> DIST -> DISTEXT -> MATCH -> LEN
                LIT -> LEN
    Process trailer:
        CHECK -> LENGTH -> DONE
 */

macro_rules! goto_mode {
    ($loc:expr, $mode:ident) => {
        {
            $loc.is_goto = true;
            $loc.state.mode = InflateMode::$mode;
            continue;
        }
    }
}

/// Decompresses ("inflates") a stream of data.  Supports both `GZIP` and raw `DEFLATE` streams.
/// Use `Inflater::new()` to create a stream.
pub struct Inflater // was inflate_state
{
    mode: InflateMode,          // current inflate mode
    last: bool,                 // true if processing last block
    wrap: u32,                  // bit 0 true for zlib, bit 1 true for gzip
    havedict: bool,             // true if dictionary provided
    flags: u32,                 // gzip header method and flags (0 if zlib)
    dmax: usize,                 // zlib header max distance (INFLATE_STRICT)
    check: u32,                 // protected copy of check value
    total: usize,                // protected copy of output count
    head: Option<GZipHeader>,   // where to save gzip header information

    // sliding window
    wbits: usize,                // log base 2 of requested window size
    wsize: usize,                // window size or zero if not using window
    whave: usize,                // valid bytes in the window
    wnext: usize,                // window write index
    window: Vec<u8>,            // allocated sliding window, if needed

    // bit accumulator
    hold: u32,                  // input bit accumulator
    bits: usize,                 // number of bits in "hold"

    // for string and stored block copying
    length: usize,               // literal or length of data to copy
    offset: usize,               // distance back to copy string from

    // for table and code decoding
    extra: usize,                // extra bits needed

    // fixed and dynamic code tables
    lencode: usize,              // starting table for length/literal codes; is an index into 'codes'
    distcode: usize,             // starting table for distance codes; is an index into 'codes'
    lenbits: usize,              // index bits for lencode
    distbits: usize,             // index bits for distcode

    // dynamic table building
    ncode: usize,                // number of code length code lengths
    nlen: usize,                 // number of length code lengths
    ndist: usize,                // number of distance code lengths
    have: usize,                 // number of code lengths in lens[]
    next: usize,                 // next available space in codes[]
    lens: [u16; 320],           // temporary storage for code lengths
    work: [u16; 288],           // work area for code table building
    codes: [Code; ENOUGH],      // space for code tables
    sane: bool,                 // if false, allow invalid distance too far
    back: usize,                 // bits back of last unprocessed length/lit
    was: usize,                  // initial length of match

    strm: ZStream,

    pub counter_inffast: u32,
    pub counter_mainloop: u32,
}

impl Inflater {
    /// Creates a new Inflater for decoding a GZIP stream.
    /// 
    /// A GZIP stream starts with a GZIP header, which is followed by a DEFLATE
    /// stream, and ends with a GZIP trailer.  The header can specify the name
    /// of the original file, the timestamp, the operating system used to generate
    /// the file, etc.
    pub fn new_gzip() -> Inflater {
        Inflater::internal_new(WINDOW_BITS_DEFAULT, 2)
    }

    /// Creates a new Inflater for decoding a raw DEFLATE stream.  This should not
    /// be used for decoding GZIP streams.
    pub fn new_inflate(window_bits: usize) -> Inflater {
        Inflater::internal_new(window_bits, 0)
    }

    fn internal_new(window_bits: usize, wrap: u32) -> Inflater {
        assert!(window_bits >= WINDOW_BITS_MIN && window_bits <= WINDOW_BITS_MAX);

        let wsize: usize = 1 << window_bits;

        Inflater {
            mode: InflateMode::HEAD,
            last: false,
            wrap: wrap,                 // bit 0 true for zlib, bit 1 true for gzip
            havedict: false,            // true if dictionary provided
            flags: 0,                   // gzip header method and flags (0 if zlib)
            dmax: DEFAULT_DMAX,         // zlib header max distance (INFLATE_STRICT)
            check: 0,                   // protected copy of check value
            total: 0,                   // protected copy of output count
            head: None,                 // where to save gzip header information

            // sliding window
            wbits: window_bits,         // log base 2 of requested window size
            wsize: wsize,               // window size or zero if not using window
            whave: 0,                   // valid bytes in the window
            wnext: 0,                   // window write index
            window: {
                let mut w = Vec::with_capacity(wsize);         // allocated sliding window, if needed
                w.extend(repeat(0u8).take(wsize));
                w
            },

            // bit accumulator
            hold: 0,                    // input bit accumulator
            bits: 0,                    // number of bits in "in"

            // for string and stored block copying
            length: 0,                  // literal or length of data to copy
            offset: 0,                  // distance back to copy string from

            // for table and code decoding
            extra: 0,                   // extra bits needed

            // fixed and dynamic code tables
            lencode: 0,                 // starting table for length/literal codes       // index into 'codes'
            distcode: 0,                // starting table for distance codes        // index into 'codes'
            lenbits: 0,                 // index bits for lencode
            distbits: 0,                // index bits for distcode

            // dynamic table building
            ncode: 0,                   // number of code length code lengths
            nlen: 0,                    // number of length code lengths
            ndist: 0,                   // number of distance code lengths
            have: 0,                    // number of code lengths in lens[]
            next: 0,                    // next available space in codes[]   // index into codes[]
            lens: [0u16; 320],          // temporary storage for code lengths
            work: [0u16; 288],          // work area for code table building
            codes: [Default::default(); ENOUGH],    // space for code tables
            sane: false,                // if false, allow invalid distance too far
            back: 0,                    // bits back of last unprocessed length/lit
            was: 0,                     // initial length of match
            strm: ZStream::new(),

            counter_mainloop: 0,
            counter_inffast: 0
        }
    }

    // Resets the state of the decoder, without changing the contents of the window.
    pub fn reset_keep(&mut self) {
        self.strm.total_in = 0;
        self.strm.total_out = 0;
        self.total = 0;
        self.strm.msg = None;
        if self.wrap != 0 {
            // to support ill-conceived Java test suite
            self.strm.adler = self.wrap as u32 & 1;
        }
        self.mode = InflateMode::HEAD;
        self.last = false;
        self.havedict = false;
        self.dmax = DEFAULT_DMAX;
        self.head = None;
        self.hold = 0;
        self.bits = 0;

        self.lencode = 0;      // index into self.codes
        self.distcode = 0;     // index into self.codes
        self.next = 0;         // index into self.codes

        self.sane = true;
        self.back = -1;
        // debug!("inflate: reset");

        self.counter_mainloop = 0;
        self.counter_inffast = 0;
    }

    /// Resets the state of the decoder.  This is equivalent to allocating a new Inflater, with
    /// the same arguments that were used to construct this Inflater.
    pub fn reset(&mut self) {
        self.wsize = 0;
        self.whave = 0;
        self.wnext = 0;
        self.reset_keep();
    }

    pub fn prime(&mut self, bits: isize, value: u32) {
        if bits < 0 {
            self.hold = 0;
            self.bits = 0;
            return;
        }

        assert!(bits <= 16);
        assert!(self.bits as isize + bits <= 32);

        let val = value & (1 << bits as usize) - 1;
        self.hold += val << self.bits;
        self.bits += bits as usize;
    }

    pub fn inflate(
        &mut self,
        flush: Option<Flush>,
        input_buffer: &[u8],
        output_buffer: &mut[u8]) -> InflateResult
    {
        debug!("inflate: avail_in={} avail_out={}", input_buffer.len(), output_buffer.len());

        match self.mode {
            InflateMode::BAD => {
                return InflateResult::InvalidData
            }
            InflateMode::DONE => {
                return InflateResult::Eof(self.check)
            }
            _ => ()
        }

        let flush = match flush {
            Some(f) => f,
            None => Flush::None
        };

        let mut loc = InflateLocals {
            state: self,
            input_buffer: input_buffer,
            output_buffer: output_buffer,
            hold: 0,
            bits: 0,
            next: 0,
            put: 0,
            flush: flush,
            is_goto: false,
        };

        Inflater::inflate_main_loop(&mut loc, flush);

        // Return from inflate(), updating the total counts and the check value.
        // If there was no progress during the inflate() call, return a buffer
        // error.  Call updatewindow() to create and/or update the window state.
        // Note: a memory error from inflate() is non-recoverable.

        debug!("inf_leave");
        restore_locals(&mut loc);

        debug!("left={}", loc.left());

        if loc.state.wsize != 0 || (loc.put != 0 && (loc.state.mode as u32) < (InflateMode::BAD as u32) &&
                ((loc.state.mode as u32) < (InflateMode::CHECK as u32) || (loc.flush != Flush::Finish))) {

            debug!("calling updatewindow()");
            let mut put = loc.put;
            if loc.state.mode as u32 >= InflateMode::CHECK as u32 {
                put = 0; // don't ask
            }
            updatewindow(&mut loc, put, put);
        }

        debug!("avail_in={} avail_out={}", loc.avail_in(), loc.avail_out());

        let in_inflated = loc.next;
        let out_inflated = loc.put;

        loc.state.strm.total_in += in_inflated as u64;
        loc.state.strm.total_out += out_inflated as u64;
        loc.state.total += out_inflated;

        if loc.state.wrap != 0 && out_inflated != 0 {
            let updated_check = update(loc.state.flags, loc.state.check, loc.output_buffer.slice_to(out_inflated));
            loc.state.strm.adler = updated_check;
            loc.state.check = updated_check;
        }
        loc.state.strm.data_type = (loc.state.bits as u32)
            | (if loc.state.last { 64 } else { 0 })
            | (if loc.state.mode == InflateMode::TYPE { 128 } else { 0 })
            | (if loc.state.mode == InflateMode::LEN_ || loc.state.mode == InflateMode::COPY_ { 256 } else { 0 });

    //    if (((loc.in_ == 0 && loc.out == 0) || loc.flush == Z_FINISH) && ret == Z_OK) {
    //        ret = Z_BUF_ERROR;
    //    }
    //

        if in_inflated != 0 || out_inflated != 0 {
            InflateResult::Decoded(in_inflated, out_inflated)
        }
        else if loc.state.mode == InflateMode::DONE {
            InflateResult::Eof(loc.state.check)
        }
        else {
            warn!("need input, mode = {:?}", loc.state.mode);
            InflateResult::NeedInput
        }
    }

    #[inline(always)] // yes, i'm serious
    fn inflate_main_loop(loc: &mut InflateLocals, flush: Flush) {

        let mut copy: usize;         // number of stored or match bytes to copy
        let mut last: Code;         // parent table entry
        let mut len: usize;          // length to copy for repeats, bits to drop
        let mut ret: usize;          // return code

        static ORDER: [u16; 19] = /* permutation of code lengths */
            [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

        match loc.state.mode {
            InflateMode::TYPE => {
                loc.state.mode = InflateMode::TYPEDO;      /* skip check */
            }
            _ => ()
        }
        load_locals(loc);

        // ret = Z_OK;
        'mainloop: loop {
            loc.state.counter_mainloop += 1;

            if !cfg!(ndebug) {
                if loc.is_goto {
                    loc.is_goto = false;
                }
                else {
                    debug!("inflate: mode={}", loc.state.mode as u32);
                }
            }

            match loc.state.mode {
            InflateMode::HEAD => {
                if loc.state.wrap == 0 {
                    debug!("HEAD - wrap = 0, switching to TYPEDO");
                    loc.state.mode = InflateMode::TYPEDO;
                    continue;
                }
                // debug!("HEAD: wrap: 0x{:x}", loc.state.wrap);
                NEEDBITS!(loc, 16);
    // #ifdef GUNZIP
                if (loc.state.wrap & 2) != 0 && loc.hold == 0x8b1f {  /* gzip header */
                    // debug!("found GZIP header");
                    loc.state.check = crc32(0, &[]);
                    loc.state.check = crc2(loc.state.check, loc.hold);
                    initbits(loc);
                    loc.state.mode = InflateMode::FLAGS;
                    continue;
                }

                debug!("hold = 0x{:08x}, wrap = {}", loc.hold, loc.state.wrap);

                loc.state.flags = 0;           /* expect zlib header */

                /*
                match &mut loc.state.head {
                    &Some(ref h) => {
                        debug!("already have header, setting done = true");
                        h.done = true;
                    },
                    &None => ()
                }
                */

                if (loc.state.wrap & 1) == 0 ||   /* check if zlib header allowed */
    // #else
    //             if (
    // #endif
                    ((((bits(loc, 8) as u32) << 8) + (loc.hold >> 8)) % 31) != 0 {
                    warn!("incorrect header check.  bits(8) = 0x{:2x}", bits(loc, 8));
                    BADINPUT!(loc, "incorrect header check");
                }
                if bits(loc, 4) != Z_DEFLATED as u32 {
                    BADINPUT!(loc, "unknown compression method");
                }
                dropbits(loc, 4);
                len = (bits(loc, 4) + 8) as usize;
                if loc.state.wbits == 0 {
                    loc.state.wbits = len;
                }
                else if len > loc.state.wbits {
                    BADINPUT!(loc, "invalid window size");
                }
                loc.state.dmax = 1 << len;
                // debug!("max distance (dmax) = {} 0x{:x}", loc.state.dmax, loc.state.dmax);
                // debug!("inflate:   zlib header ok");
                let adler_value = adler32(0, &[]);
                loc.state.strm.adler = adler_value;
                loc.state.check = adler_value;
                loc.state.mode = if (loc.hold & 0x200) != 0 { InflateMode::DICTID } else { InflateMode::TYPE };
                initbits(loc);
            }

    // #ifdef GUNZIP
            InflateMode::FLAGS => {
                NEEDBITS!(loc, 16);

                let flags = loc.hold;
                let is_text = ((loc.hold >> 8) & 1) != 0;
                let method = flags & 0xff;
                // debug!("FLAGS: flags: 0x{:8x} is_text: {}", flags, is_text);

                loc.state.flags = loc.hold;
                if method != Z_DEFLATED as u32 {
                    BADINPUT!(loc, "unknown compression method");
                }
                if (loc.state.flags & 0xe000) != 0 {
                    BADINPUT!(loc, "unknown header flags set");
                }

                match loc.state.head {
                    Some(ref mut h) => {
                        h.text = is_text;
                    }
                    None => ()
                }
                if (loc.state.flags & 0x0200) != 0 {
                    // debug!("FLAGS contains time");
                    loc.state.check = crc2(loc.state.check, loc.hold);
                }
                else {
                    // debug!("FLAGS did not have a time");
                }
                initbits(loc);
                goto_mode!(loc, TIME);
            }

            InflateMode::TIME => {
                NEEDBITS!(loc, 32);
                let time :u32 = loc.state.hold;
                // debug!("TIME: t: {}", time);

                /*if (state.head != Z_NULL)
                    state.head.time = time;
                */

                if (loc.state.flags & 0x0200) != 0 {
                    loc.state.check = crc4(loc.state.check, time);
                }
                initbits(loc);
                goto_mode!(loc, OS);
            }

            InflateMode::OS => {
                NEEDBITS!(loc, 16);
                let ostype = loc.state.hold;
                let xflags = ostype & 0xff;
                let os = ostype >> 8;
                // debug!("OS: os 0x{:x} xflags 0x{:x}", os, xflags);
                match loc.state.head {
                    Some(ref mut h) => {
                        h.xflags = xflags;
                        h.os = os;
                    }
                    None => ()
                }
                if (loc.state.flags & 0x0200) != 0 {
                    loc.state.check = crc2(loc.state.check, ostype);
                }
                initbits(loc);
                goto_mode!(loc, EXLEN);
            }

            InflateMode::EXLEN => {
                if (loc.state.flags & 0x0400) != 0 {
                    NEEDBITS!(loc, 16);
                    let extra_len = loc.state.hold & 0xffff;
                    loc.state.length = extra_len as usize;

                    // debug!("EXTRALEN: extra_len = {}", extra_len);

                    match loc.state.head {
                        Some(ref mut h) => {
                            h.extra_len = extra_len as usize;
                        }
                        _ => ()
                    }

                    if (loc.state.flags & 0x0200) != 0 {
                        loc.state.check = crc2(loc.state.check, extra_len);
                    }
                    initbits(loc);
                }
                else {
                    // debug!("no EXTRALEN");
                    match loc.state.head {
                        Some(ref mut h) => {
                            h.extra_len = 0;
                        }
                        None => ()
                    }
                }
                goto_mode!(loc, EXTRA);
            }

            InflateMode::EXTRA => {
                if (loc.state.flags & 0x0400) != 0 {
                    let mut copy = loc.state.length;
                    if copy > loc.have() {
                        copy = loc.have();
                    }
                    if copy != 0 {
                        /* TODO
                        if (state.head != Z_NULL && state.head.extra != Z_NULL) {
                            len = state.head.extra_len - state.length;
                            copy_memory(state.head.extra + len, next,
                                    if len + copy > state.head.extra_max
                                    { state.head.extra_max - len } else { copy });
                        }
                        */
                        if (loc.state.flags & 0x0200) != 0 {
                            loc.state.check = crc32(loc.state.check, loc.input_buffer.slice(loc.next, loc.next + copy));
                        }
                        loc.next += copy;
                        loc.state.length -= copy;
                    }
                    if loc.state.length != 0 {
                        break;
                    }
                }
                else {
                    // debug!("no EXTRA");
                }
                loc.state.length = 0;
                goto_mode!(loc, NAME);
            }

            InflateMode::NAME => {
                if (loc.state.flags & 0x0800) != 0 {
                    // debug!("NAME: header flags indicate that stream contains a NAME record");
                    if loc.have() == 0 {
                        break;
                    }
                    let mut copy = 0;
                    loop {
                        len = loc.input_buffer[loc.next + copy] as usize;
                        copy += 1;

                        /* TODO
                        if (state.head != Z_NULL && state.head.name != Z_NULL && state.length < state.head.name_max) {
                            state.head.name[state.length++] = len;
                        } */

                        if !(len != 0 && copy < loc.have()) {
                             break;
                        }
                    }
                    if (loc.state.flags & 0x0200) != 0 {
                        loc.state.check = crc32(loc.state.check, loc.input_buffer.slice(loc.next, loc.next + copy));
                    }
                    loc.next += copy;
                    if len != 0 { break; }
                }
                else {
                    // debug!("NAME: header does not contain a NAME record");
                    /*TODO if (state.head != Z_NULL) {
                        state.head.name = Z_NULL;
                    }*/
                }
                loc.state.length = 0;
                goto_mode!(loc, COMMENT);
            }

            InflateMode::COMMENT => {
                if (loc.state.flags & 0x1000) != 0 {
                    // debug!("COMMENT: header contains a COMMENT record");
                    if loc.have() == 0 {
                        // debug!("have no data, returning");
                        break;
                    }
                    let mut copy = 0;
                    let mut len;
                    loop {
                        len = loc.input_buffer[loc.next + copy];
                        copy += 1;

                        /* TODO
                        if (state.head != Z_NULL &&
                                state.head.comment != Z_NULL &&
                                state.length < state.head.comm_max) {
                            state.head.comment[state.length] = len;
                            state.length += 1;
                        }
                        */

                        if !(len != 0 && copy < loc.have()) {
                            break;
                        }
                    }
                    if (loc.state.flags & 0x0200) != 0 {
                        loc.state.check = crc32(loc.state.check, loc.input_buffer.slice(loc.next, loc.next + copy));
                    }
                    loc.next += copy;
                    if len != 0 {
                        // We have not received all of the bytes for the comment, so bail.
                        break;
                    }
                }
                else {
                    // debug!("COMMENT: header does not contain a COMMENT record");
                    // TODO
                    // if (state.head != Z_NULL)
                    //     state.head.comment = Z_NULL;
                }
                goto_mode!(loc, HCRC);
            }

            InflateMode::HCRC => {
                if (loc.state.flags & 0x0200) != 0 {
                    NEEDBITS!(loc, 16);
                    let expected_crc = loc.hold;
                    // debug!("HCRC: header says expected CRC = 0x{:x}", expected_crc);
                    /* TODO - CRC
                    if expected_crc != (loc.state.check & 0xffff) {
                        BADINPUT!(loc, "header crc mismatch");
                    }
                    */
                    initbits(loc);
                }
                /* TODO if (state.head != Z_NULL) {
                    loc.state.head.hcrc = (int)((state.flags >> 9) & 1);
                    loc.state.head.done = 1;
                }*/
                let initial_crc = crc32(0, &[]);
                loc.state.strm.adler = initial_crc;
                loc.state.check = initial_crc;
                loc.state.mode = InflateMode::TYPE;
            }
    // #endif
            InflateMode::DICTID => {
                NEEDBITS!(loc, 32);
                let check = swap32(loc.hold);
                // debug!("check = 0x{:x}", check);
                loc.state.strm.adler = check;
                loc.state.check = check;
                initbits(loc);
                goto_mode!(loc, DICT);
            }

            InflateMode::DICT => {
                if !loc.state.havedict {
                    debug!("do not have dictionary, returning Z_NEED_DICT");
                    restore_locals(loc);
                    unimplemented!(); // return Z_NEED_DICT;
                }
                let check = adler32(0, &[]);
                loc.state.strm.adler = check;
                loc.state.check = check;
                goto_mode!(loc, TYPE);
            }

            InflateMode::TYPE => {
                if flush == Flush::Block || flush == Flush::Trees {
                    debug!("TYPE: flush is Z_BLOCK or Z_TREES, returning");
                    break;
                }
                goto_mode!(loc, TYPEDO);
            }

            InflateMode::TYPEDO => {
                if loc.state.last {
                    // debug!("TYPEDO: is last block, --> CHECK");
                    bytebits(loc);
                    loc.state.mode = InflateMode::CHECK;
                }
                else {
                    NEEDBITS!(loc, 3);
                    loc.state.last = bitbool(loc);
                    dropbits(loc, 1);
                    // debug!("TYPEDO: last = {}, kind = {}", loc.state.last, bits(loc, 2));
                    match bits(loc, 2) {
                        0 => { // stored block 
                            if loc.state.last {
                                debug!("inflate:     stored block (last)");
                            }
                            else {
                                debug!("inflate:     stored block");
                            }
                            loc.state.mode = InflateMode::STORED;
                        }

                        1 => { // fixed block
                            loc.state.fixedtables();
                            if loc.state.last {
                                debug!("inflate:     fixed codes block (last)");
                            }
                            else {
                                debug!("inflate:     fixed codes block");
                            }
                            loc.state.mode = InflateMode::LEN_; // decode codes
                            if flush == Flush::Trees {
                                dropbits(loc, 2);
                                break;
                            }
                        }

                        2 => { // dynamic block
                            if loc.state.last {
                                debug!("inflate:     dynamic codes block (last)");
                            }
                            else {
                                debug!("inflate:     dynamic codes block");
                            }
                            loc.state.mode = InflateMode::TABLE;
                        }
                        3 => {
                            BADINPUT!(loc, "invalid block type");
                        }
                        _ => { unreachable!(); }
                    }
                    dropbits(loc, 2);
                    debug!("hold={:08x} bits={}", loc.hold, loc.bits);
                }
            }

            InflateMode::STORED => {
                bytebits(loc);                         /* go to byte boundary */
                NEEDBITS!(loc, 32);
                let len = loc.hold & 0xffff;
                let invlen = (loc.hold >> 16) ^ 0xffff;
                if len != invlen {
                    warn!("invalid stored block lengths;  hold=0x{:08x}  len=0x{:04x}  invlen=0x{:04x}", loc.hold, len, invlen);
                    BADINPUT!(loc, "invalid stored block lengths");
                }
                debug!("inflate:       stored length {}", len);
                loc.state.length = len as usize;
                initbits(loc);
                loc.state.mode = InflateMode::COPY_;
                if flush == Flush::Trees {
                    debug!("flush = Z_TREES, so returning");
                    break;
                }
                goto_mode!(loc, COPY_);
            }
            InflateMode::COPY_ => {
                goto_mode!(loc, COPY);
            }
            InflateMode::COPY => {
                copy = loc.state.length;
                if copy != 0 {
                    // debug!("copy length = {}", copy);
                    if copy > loc.have() { copy = loc.have(); }
                    if copy > loc.left() { copy = loc.left(); }
                    if copy == 0 {
                        // debug!("cannot copy data right now (no buffer space) -- exiting");
                        break;
                    }
                    copy_memory(loc.output_buffer.slice_mut(loc.put, loc.put + copy), loc.input_buffer.slice(loc.next, loc.next + copy));
                    loc.next += copy;
                    loc.put += copy;
                    loc.state.length -= copy;
                    // stay in state COPY
                }
                else {
                    debug!("inflate:       stored end");
                    loc.state.mode = InflateMode::TYPE;
                }
            }

            InflateMode::TABLE => {
                NEEDBITS!(loc, 14);
                loc.state.nlen = bits_and_drop(loc, 5) as usize + 257;
                loc.state.ndist = bits_and_drop(loc, 5) as usize + 1;
                loc.state.ncode = bits_and_drop(loc, 4) as usize + 4;
                // debug!("TABLE: nlen {} ndist {} ncode {}", loc.state.nlen, loc.state.ndist, loc.state.ncode);
    // #ifndef PKZIP_BUG_WORKAROUND
                if loc.state.nlen > 286 || loc.state.ndist > 30 {
                    BADINPUT!(loc, "too many length or distance symbols");
                }
    // #endif
                debug!("inflate:       table sizes ok");
                loc.state.have = 0;
                goto_mode!(loc, LENLENS);
            }

            InflateMode::LENLENS => {
                // debug!("have = {}, ncode = {}, reading {} lengths", loc.state.have, loc.state.ncode, loc.state.ncode - loc.state.have);
                while loc.state.have < loc.state.ncode {
                    NEEDBITS!(loc, 3);
                    let lenlen = bits(loc, 3);
                    let lenindex = ORDER[loc.state.have] as usize;
                    // debug!("    lens[{}] := {}", lenindex , lenlen);
                    loc.state.lens[lenindex ] = lenlen as u16;
                    loc.state.have += 1;
                    dropbits(loc, 3);
                }
                while loc.state.have < 19 {
                    let lenindex = ORDER[loc.state.have] as usize;
                    // debug!("clearing {}", lenindex);
                    loc.state.lens[lenindex] = 0;
                    loc.state.have += 1;
                }
                // debug!("inflating code lengths");
                loc.state.next = 0;
                loc.state.lencode = loc.state.next;
                loc.state.lenbits = 7;
                let (inflate_ret, inflate_bits) = inflate_table(CODES, &loc.state.lens, 19, &mut loc.state.codes, &mut loc.state.next,
                    loc.state.lenbits, loc.state.work.as_mut_slice());
                ret = inflate_ret as usize;
                loc.state.lenbits = inflate_bits;
                if ret != 0 {
                    BADINPUT!(loc, "invalid code lengths set");
                }
                debug!("inflate:       code lengths ok");
                loc.state.have = 0;
                goto_mode!(loc, CODELENS);
            }
            InflateMode::CODELENS => {
                while loc.state.have < loc.state.nlen + loc.state.ndist {
                    let mut here: Code; // current decoding table entry
                    while { here = loc.state.codes[loc.state.lencode + bits(loc, loc.state.lenbits) as usize]; here.bits as usize > loc.bits } {
                        PULLBYTE!(loc);
                    }
                    if here.val < 16 {
                        dropbits(loc, here.bits as usize);
                        loc.state.lens[loc.state.have] = here.val;
                        loc.state.have += 1;
                    }
                    else {
                        let (len, copy) = if here.val == 16 {
                            NEEDBITS!(loc, here.bits as usize + 2);
                            dropbits(loc, here.bits as usize);
                            if loc.state.have == 0 {
                                BADINPUT!(loc, "invalid bit length repeat");
                            }
                            (loc.state.lens[loc.state.have as usize - 1], 3 + bits_and_drop(loc, 2) as usize)
                        }
                        else if here.val == 17 {
                            NEEDBITS!(loc, here.bits as usize + 3);
                            dropbits(loc, here.bits as usize);
                            (0, 3 + bits_and_drop(loc, 3) as usize)
                        }
                        else {
                            NEEDBITS!(loc, here.bits as usize + 7);
                            dropbits(loc, here.bits as usize);
                            (0, 11 + bits_and_drop(loc, 7) as usize)
                        };
                        if loc.state.have + copy > loc.state.nlen + loc.state.ndist {
                            BADINPUT!(loc, "invalid bit length repeat");
                        }
                        for _ in range(0, copy) {
                            loc.state.lens[loc.state.have] = len as u16;
                            loc.state.have += 1;
                        }
                    }
                }

                // check for end-of-block code (better have one)
                if loc.state.lens[256] == 0 {
                    BADINPUT!(loc, "(CODELENS) invalid code -- missing end-of-block");
                }

                // build code tables -- note: do not change the lenbits or distbits
                // values here (9 and 6) without reading the comments in inftrees.h
                // concerning the ENOUGH constants, which depend on those values
                loc.state.next = 0;
                loc.state.lencode = 0;
                loc.state.lenbits = 9;
                // debug!("calling inflate_table for lengths");
                let (inflate_result, inflate_bits) = inflate_table(
                    LENS, loc.state.lens.as_slice(), loc.state.nlen, &mut loc.state.codes, &mut loc.state.next,
                                    loc.state.lenbits, loc.state.work.as_mut_slice());
                ret = inflate_result as usize;
                loc.state.lenbits = inflate_bits;
                if ret != 0 {
                    BADINPUT!(loc, "invalid literal/lengths set");
                }
                loc.state.distcode = loc.state.next;
                loc.state.distbits = 6;
                // debug!("calling inflate_table for codes");
                // debug!("loc.state.lens = {}, loc.state.nlen = {}, loc.state.ndist = {}", loc.state.lens.len(), loc.state.nlen, loc.state.ndist);

                let (inflate_ret, inflate_bits) = {
                    let codes_lens = loc.state.lens.slice(loc.state.nlen, loc.state.nlen + loc.state.ndist);
                    inflate_table(DISTS, codes_lens, loc.state.ndist,
                                &mut loc.state.codes, &mut loc.state.next, loc.state.distbits, loc.state.work.as_mut_slice()) };
                if inflate_ret != 0 {
                    BADINPUT!(loc, "invalid distances set");
                }
                loc.state.distbits = inflate_bits;
                debug!("inflate:       codes ok");
                loc.state.mode = InflateMode::LEN_;
                if flush == Flush::Trees {
                    debug!("flush = Z_TREES, returning");
                    break;
                }
                goto_mode!(loc, LEN_);
            }
            InflateMode::LEN_ => {
                goto_mode!(loc, LEN);
            }
            InflateMode::LEN => {
                debug!("LEN: left={}", loc.left());
                if loc.have() >= 6 && loc.left() >= 258 {
                    debug!("LEN: fast path");
                    restore_locals(loc);
                    loc.state.counter_inffast += 1;
                    let iffr = inflate_fast(
                        loc.state,
                        loc.input_buffer,
                        loc.output_buffer, 
                        loc.next,
                        loc.put);
                    loc.next = iffr.strm_next_in;
                    loc.put = iffr.strm_next_out;
                    load_locals(loc);
                    debug!("left={}", loc.left());
                    if loc.state.mode == InflateMode::TYPE {
                        loc.state.back = -1;
                    }
                }
                else {
                    debug!("LEN: slow path");
                    loc.state.back = 0;
                    let mut here: Code;         // current decoding table entry
                    loop {
                        here = loc.state.codes[loc.state.lencode + bits(loc, loc.state.lenbits) as usize];
                        if here.bits as usize <= loc.bits as usize {
                            break;
                        }
                        PULLBYTE!(loc);
                    }
                    if here.op != 0 && (here.op & 0xf0) == 0 {
                        last = here;
                        loop {
                            here = loc.state.codes[loc.state.lencode + last.val as usize + (bits(loc, last.bits as usize + last.op as usize) as usize >> last.bits as usize)];
                            if (last.bits as usize + here.bits as usize) <= loc.bits {
                                break;
                            }
                            PULLBYTE!(loc);
                        }
                        dropbits(loc, last.bits as usize);
                        loc.state.back += last.bits as usize;
                    }
                    dropbits(loc, here.bits as usize);
                    loc.state.back += here.bits as usize;
                    loc.state.length = here.val as usize;
                    if here.op == 0 {
                        if here.val >= 0x20 && here.val < 0x7f {
                            debug!("inflate:         literal '{}'", here.val as u8 as char);
                        }
                        else {
                            debug!("inflate:         literal 0x{:02x}", here.val);
                        }
                        loc.state.mode = InflateMode::LIT;
                        continue;
                    }
                    if (here.op & 32) != 0 {
                        debug!("inflate:         end of block");
                        loc.state.back = -1;
                        loc.state.mode = InflateMode::TYPE;
                        continue;
                    }
                    if (here.op & 64) != 0 {
                        BADINPUT!(loc, "invalid literal/length code");
                    }
                    loc.state.extra = (here.op & 15) as usize;
                    goto_mode!(loc, LENEXT);
                }
            }

            InflateMode::LENEXT => {
                debug!("LENEXT: extra={}", loc.state.extra);
                if loc.state.extra != 0 {
                    NEEDBITS!(loc, loc.state.extra);
                    loc.state.length += bits(loc, loc.state.extra as usize) as usize;
                    let extra = loc.state.extra;
                    dropbits(loc, extra);
                    loc.state.back += loc.state.extra;
                }
                debug!("inflate:         length {}", loc.state.length);
                loc.state.was = loc.state.length;
                goto_mode!(loc, DIST);
            }

            InflateMode::DIST => {
                let mut here: Code;         // current decoding table entry
                loop {
                    here = loc.state.codes[loc.state.distcode as usize + bits(loc, loc.state.distbits) as usize];
                    if here.bits as usize <= loc.bits {
                        break;
                    }
                    PULLBYTE!(loc);
                }
                if (here.op & 0xf0) == 0 {
                    last = here;
                    loop {
                        here = loc.state.codes[loc.state.distcode + last.val as usize + (bits(loc, last.bits as usize + last.op as usize) >> last.bits as usize) as usize];
                        if (last.bits as usize + here.bits as usize) <= loc.bits as usize {
                            break;
                        }
                        PULLBYTE!(loc);
                    }
                    dropbits(loc, last.bits as usize);
                    loc.state.back += last.bits as usize;
                }
                dropbits(loc, here.bits as usize);
                loc.state.back += here.bits as usize;
                if (here.op & 64) != 0 {
                    BADINPUT!(loc, "invalid distance code");
                }
                loc.state.offset = here.val as usize;
                loc.state.extra = (here.op & 15) as usize;
                goto_mode!(loc, DISTEXT);
            }

            InflateMode::DISTEXT => {
                if loc.state.extra != 0 {
                    NEEDBITS!(loc, loc.state.extra);
                    loc.state.offset += bits(loc, loc.state.extra) as usize;
                    let extra = loc.state.extra;
                    dropbits(loc, extra);
                    loc.state.back += loc.state.extra;
                }
    // #ifdef INFLATE_STRICT
                if loc.state.offset > loc.state.dmax {
                    BADINPUT!(loc, "invalid distance too far back");
                }
    // #endif
                debug!("inflate:         distance {}", loc.state.offset);
                goto_mode!(loc, MATCH);
            }

            InflateMode::MATCH => {
                let mut from :BufPos; // index into loc.input_buffer (actually, several different buffers)
                if loc.left() == 0 {
                    debug!("MATCH: inf_leave");
                    break;
                }
                let mut copy = loc.put;
                debug!("copy={} state.offset={}", copy, loc.state.offset);
                if loc.state.offset > copy {         /* copy from window */
                    copy = loc.state.offset - copy;
                    debug!("copy from window, copy={}", copy);
                    if copy > loc.state.whave {
                        if loc.state.sane {
                            BADINPUT!(loc, "invalid distance too far back");
                        }
    // #ifdef INFLATE_ALLOW_INVALID_DISTANCE_TOOFAR_ARRR
                        debug!("inflate.c too far");
                        copy -= loc.state.whave;
                        if copy > loc.state.length { copy = loc.state.length; }
                        if copy > loc.left() { copy = loc.left(); }
                        loc.state.length -= copy;
                        loop {
                            loc.output_buffer[loc.put] = 0;
                            loc.put += 1;
                            copy -= 1;
                            if copy == 0 { break; }
                        }
                        if loc.state.length == 0 { loc.state.mode = InflateMode::LEN; }
                        continue;
    // #endif
                    }
                    if copy > loc.state.wnext {
                        copy -= loc.state.wnext;
                        from = BufPos { buf: loc.state.window.as_slice(), pos: (loc.state.wsize - copy) };
                    }
                    else {
                        from = BufPos { buf: loc.state.window.as_slice(), pos: (loc.state.wnext - copy) };
                    }
                    if copy > loc.state.length {
                        copy = loc.state.length;
                    }

                    if copy > loc.left() {
                        debug!("copy={} > left={}, setting copy={}", copy, loc.left(), copy);
                        copy = loc.left();
                    }
                    loc.state.length -= copy;
                    while copy > 0 {
                        let b = from.read();
                        // debug!("MATCH: write {}", b);
                        loc.output_buffer[loc.put] = b;
                        loc.put += 1;
                        copy -= 1;
                    }
                }
                else {
                    // Copy data from the output buffer to the output buffer.  Because this requires copying
                    // data within a single buffer, the compiler (rightly!) points out that we cannot alias
                    // the output buffer for a call to copy_memory or something similar.  Also, it is legal
                    // (and very common) for the copy to be self-overlapping, meaning the copy will repeat a
                    // pattern into the output buffer, because we will read data that we have just written.
                    // To do this correctly, we simply fall back to manually indexing into output_buffer.
                    let mut from_pos = loc.put - loc.state.offset;
                    // from = BufPos { buf: loc.input_buffer, pos: loc.put - loc.state.offset };
                    copy = loc.state.length;
                    debug!("copy from output, copy={}", copy);

                    if copy > loc.left() {
                        debug!("copy={} > left={}, setting copy={}", copy, loc.left(), copy);
                        copy = loc.left();
                    }
                    loc.state.length -= copy;
                    while copy > 0 {
                        let b = loc.output_buffer[from_pos];
                        from_pos += 1;
                        // debug!("MATCH: write {}", b);
                        loc.output_buffer[loc.put] = b;
                        loc.put += 1;
                        copy -= 1;
                    }
                }

                if loc.state.length == 0 {
                    loc.state.mode = InflateMode::LEN;
                }
            }

            InflateMode::LIT => {
                if loc.left() == 0 {
                    break;
                }
                // debug!("LIT: write {}", loc.state.length as u8);
                loc.output_buffer[loc.put] = loc.state.length as u8;
                loc.put += 1;
                loc.state.mode = InflateMode::LEN;
            }

            InflateMode::CHECK => {
                // let mut from: usize; // index into loc.input_buffer
                if loc.state.wrap != 0 {
                    NEEDBITS!(loc, 32);
                    loc.state.strm.total_out += loc.put as u64;
                    loc.state.total += loc.put;
                    if loc.put != 0 {
                        let check = update(loc.state.flags, loc.state.check, loc.output_buffer.slice_to(loc.put));
                        loc.state.strm.adler = check;
                        loc.state.check = check;
                    }
    // #ifdef GUNZIP
                    let ch = if loc.state.flags != 0 { loc.hold } else { swap32(loc.hold) };
                    if ch != loc.state.check {
                        // not implemented yet
                        // BADINPUT!(loc, "incorrect data check");
                    }
    // #else
    //              if ((ZSWAP32(hold)) != state.check) {
    //                  BADINPUT!(loc, "incorrect data check");
    //              }
    // #endif
                    initbits(loc);
                    debug!("inflate:   check matches trailer");
                }
    // #ifdef GUNZIP
                goto_mode!(loc, LENGTH);
            }

            InflateMode::LENGTH => {
                if loc.state.wrap != 0 && loc.state.flags != 0 {
                    NEEDBITS!(loc, 32);
                    /* -- need to fix
                    if loc.hold != (loc.state.total & 0xffffffff) as u32 {
                        warn!("LENGTH: expected 0x{:08x}, instead got 0x{:08x}", loc.hold, (loc.state.total & 0xffffffff) as u32);
                        BADINPUT!(loc, "incorrect length check");
                    }
                    */
                    initbits(loc);
                    debug!("inflate:   length matches trailer");
                }
                else {
                    debug!("LENGTH: no length in stream");
                }
    // #endif
                goto_mode!(loc, DONE);
            }

            InflateMode::DONE => {
                break;
            }

            InflateMode::BAD => {
                debug!("BAD state -- input data is invalid");
                match loc.state.strm.msg {
                    Some(ref errmsg) => {
                        panic!("InflateMode::BAD: total_in = {}, message: {}", loc.state.strm.total_in, errmsg);
                    }
                    _ => {}
                }
                panic!();
                // ret = Z_DATA_ERROR;
                // return inf_leave(loc);
            }

            /*
            case SYNC:
            default:
                return Z_STREAM_ERROR;
            */

                _ => {
                    warn!("unimplemented mode: {:?}", loc.state.mode);
                    unimplemented!();
                }
            }
        }
    }

    // Return state with length and distance decoding tables and index sizes set to
    // fixed code decoding.  Normally this returns fixed tables from inffixed.h.
    // If BUILDFIXED is defined, then instead this routine builds the tables the
    // first time it's called, and returns those tables the first time and
    // thereafter.  This reduces the size of the code by about 2K bytes, in
    // exchange for a little execution time.  However, BUILDFIXED should not be
    // used for threaded applications, since the rewriting of the tables and virgin
    // may not be thread-safe.
    fn fixedtables(&mut self) {
/* // This code works, but we prefer to use pre-built tables.
        // debug!("fixedtables");

        let mut fixed: [Code, ..544] = [Default::default(), ..544];

        // build fixed huffman tables

        /* literal/length table */
        {
            let mut sym :usize = 0;
            while sym < 144 { self.lens[sym] = 8; sym += 1; }
            while sym < 256 { self.lens[sym] = 9; sym += 1; }
            while sym < 280 { self.lens[sym] = 7; sym += 1; }
            while sym < 288 { self.lens[sym] = 8; sym += 1; }
        }

        let mut next :usize = 0;     // index into 'fixed' table
        let lenfix: usize = 0;       // index into 'fixed' table
        let (err, bits) = inflate_table(LENS, &self.lens, 288, &mut fixed, &mut next, 9, self.work.as_mut_slice());
        assert!(err == 0);

        /* distance table */
        {
            let mut sym :usize = 0;
            while sym < 32 { self.lens[sym] = 5; sym += 1; }
        }
        let distfix: usize = next;      // index into 'fixed' table

        let (err, bits) = inflate_table(DISTS, &self.lens, 32, &mut fixed, &mut next, 5, self.work.as_mut_slice());
        assert!(err == 0);

    // #else /* !BUILDFIXED */
    // #   include "inffixed.h"
    // #endif /* BUILDFIXED */
        // ::std::slice::bytes::copy_memory(self.codes.as_mut_slice(), fixed.as_slice());
        for i in range(0, 544) {
            self.codes[i] = fixed[i];
        }
        self.lencode = lenfix;
        self.lenbits = 9;
        self.distcode = distfix;
        self.distbits = 5;
*/
        // Copy fixed tables

        for i in range(0, inffixed::LENFIX.len()) {
            self.codes[i] = inffixed::LENFIX[i];
        }

        for i in range(0, inffixed::DISTFIX.len()) {
            self.codes[i + inffixed::LENFIX.len()] = inffixed::DISTFIX[i];
        }

        self.lencode = 0;
        self.distcode = inffixed::LENFIX.len();
        self.lenbits = 9;
        self.distbits = 5;
    }
}

// Update the window with the last wsize (normally 32K) bytes written before
// returning.  If window does not exist yet, create it.  This is only called
// when a window is already in use, or when output has been written during this
// inflate call, but the end of the deflate stream has not been reached yet.
// It is also called to create a window for dictionary data when a dictionary
// is loaded.
//
// Providing output buffers larger than 32K to inflate() should provide a speed
// advantage, since only the last 32K of output is copied to the sliding window
// upon return from inflate(), and since all distances after the first 32K of
// output will fall in the output data, making match copies simpler and faster.
// The advantage may be dependent on the size of the processor's data caches.
//
//      end - the index within loc.output_buffer where the source data ENDS
//      copy - the length of the data that was just written to loc.output_buffer,
//          and so which is now available to copy into the window
//
#[inline]
fn updatewindow(loc: &mut InflateLocals, end: usize, copy: usize) {
    debug!("updatewindow: copy={}", copy);

    let mut copy = copy;
    let mut dist: usize;

    /* if it hasn't been done already, allocate space for the window */

    // loc.state.window.clear();
    // loc.state.window.grow(1 << loc.state.wbits, 0);

    /* if window not in use yet, initialize */
    if loc.state.wsize == 0 {
        // debug!("wsize=0, initializing window, wbits={}", loc.state.wbits);
        loc.state.wsize = 1 << loc.state.wbits;
        loc.state.wnext = 0;
        loc.state.whave = 0;
    }

    /* copy state.wsize or less output bytes into the circular window */
    if copy >= loc.state.wsize {
        // debug!("copy >= wsize, copy = {}, wsize = {}", copy, loc.state.wsize);
        debug!("filling entire window");
        copy_memory(loc.state.window.as_mut_slice(), loc.output_buffer.slice(end - loc.state.wsize, end));
        loc.state.wnext = 0;
        loc.state.whave = loc.state.wsize;
    }
    else {
        // debug!("copy < wsize, copy = {}, wsize = {}", copy, loc.state.wsize);
        debug!("partial window fill");
        dist = loc.state.wsize - loc.state.wnext;
        if dist > copy {
            dist = copy;
        }
        debug!("copying from output_buffer to window[{}] length: {}", loc.state.wnext, dist);
        copy_memory(
            loc.state.window.slice_mut(loc.state.wnext, loc.state.wnext + dist),
            loc.output_buffer.slice(end - copy, end - copy + dist));
        copy -= dist;
        if copy != 0 {
            debug!("copying second chunk, to window start, length: {}", copy);
            copy_memory(loc.state.window.as_mut_slice(), loc.output_buffer.slice(end - copy, end));
            loc.state.wnext = copy;
            loc.state.whave = loc.state.wsize;
        }
        else {
            debug!("no second chunk, advancing by dist={}", dist);
            loc.state.wnext += dist;
            if loc.state.wnext == loc.state.wsize {
                debug!("wnext=wsize, so resetting wnext to 0");
                loc.state.wnext = 0;
            }
            if loc.state.whave < loc.state.wsize {
                debug!("whave < wsize, so advancing whave");
                loc.state.whave += dist;
            }
        }
    }

    debug!("whave={} wnext={}", loc.state.whave, loc.state.wnext);
}

/* Macros for inflate(): */

/* check function to use adler32() for zlib or crc32() for gzip */
// was UPDATE
fn update(flags: u32, check: u32, data: &[u8]) -> u32
{
// #ifdef GUNZIP
    if flags != 0 {
        crc32(check, data)
    }
    else {
        adler32(check, data)
    }
// #else
// #  define UPDATE(check, buf, len) adler32(check, buf, len)
// #endif
}


/* check macros for header crc */
// #ifdef GUNZIP
// #  define CRC2(check, word) \

// Computes a CRC over two bytes.  The bytes are stored in a u32 value.
// The bits are packed in "little-endian" form; byte[0] is in bits [0..7],
// while byte[1] is in bits [8..15].
fn crc2(check: u32, word: u32) -> u32 {
    let mut hbuf :[u8; 2] = [0; 2];
    hbuf[0] = (word & 0xff) as u8;
    hbuf[1] = ((word >> 8) & 0xff) as u8;
    return crc32(check, &hbuf);
}

// Computes a CRC over four bytes.  The bytes are stored in a u32 value.
// The bits are packed in "little-endian" form; byte[0] is in bits [0..7],
// while byte[1] is in bits [8..15], etc.
fn crc4(check: u32, word: u32) -> u32 {
    let mut hbuf = [0u8; 4];
    hbuf[0] = (word & 0xff) as u8;
    hbuf[1] = ((word >> 8) & 0xff) as u8;
    hbuf[2] = ((word >> 16) & 0xff) as u8;
    hbuf[3] = ((word >> 24) & 0xff) as u8;
    return crc32(check, &hbuf);
}

/* Load registers with state in inflate() for speed */
// was LOAD
#[inline]
fn load_locals(loc: &mut InflateLocals) {
    loc.hold = loc.state.hold;
    loc.bits = loc.state.bits;
}

/* Restore state from registers in inflate() */
// was RESTORE
#[inline]
fn restore_locals(loc: &mut InflateLocals) {
    loc.state.hold = loc.hold;
    loc.state.bits = loc.bits;
}

// Clear the input bit accumulator
#[inline]
fn initbits(loc: &mut InflateLocals) {
    loc.hold = 0;
    loc.bits = 0;
}


/* Return the low n bits of the bit accumulator (n < 16) */
// was 'BITS'
#[inline]
fn bits(loc: &InflateLocals, n: usize) -> u32
{
    loc.hold & ((1 << n) - 1)
}

// was BITBOOL
#[inline]
fn bitbool(loc: &InflateLocals) -> bool
{
    bits(loc, 1) != 0
}

/* Remove n bits from the bit accumulator */
// was 'DROPBITS'
#[inline]
fn dropbits(loc: &mut InflateLocals, n: usize)
{
    loc.hold >>= n;
    loc.bits -= n;
}

#[inline]
fn bits_and_drop(loc: &mut InflateLocals, n: usize) -> u32
{
    let v = bits(loc, n);
    dropbits(loc, n);
    v
}

/* Remove zero to seven bits as needed to go to a byte boundary */
#[inline]
fn bytebits(loc: &mut InflateLocals) {
    loc.hold >>= loc.bits & 7;
    loc.bits -= loc.bits & 7;
}

/*
   inflate() uses a state machine to process as much input data and generate as
   much output data as possible before returning.  The state machine is
   structured roughly as follows:

    for (;;) switch (state) {
    ...
    case STATEn:
        if (not enough input data or output space to make progress)
            return;
        ... make progress ...
        state = STATEm;
        break;
    ...
    }

   so when inflate() is called again, the same case is attempted again, and
   if the appropriate resources are provided, the machine proceeds to the
   next state.  The NEEDBITS() macro is usually the way the state evaluates
   whether it can proceed or should return.  NEEDBITS() does the return if
   the requested bits are not available.  The typical use of the BITS macros
   is:

        NEEDBITS(n);
        ... do something with BITS(n) ...
        DROPBITS(n);

   where NEEDBITS(n) either returns from inflate() if there isn't enough
   input left to load n bits into the accumulator, or it continues.  BITS(n)
   gives the low n bits in the accumulator.  When done, DROPBITS(n) drops
   the low n bits off the accumulator.  initbits() clears the accumulator
   and sets the number of available bits to zero.  BYTEBITS() discards just
   enough bits to put the accumulator on a byte boundary.  After BYTEBITS()
   and a NEEDBITS(8), then BITS(8) would return the next byte in the stream.

   NEEDBITS(n) uses PULLBYTE() to get an available byte of input, or to return
   if there is no input available.  The decoding of variable length codes uses
   PULLBYTE() directly in order to pull just enough bytes to decode the next
   code, and no more.

   Some states loop until they get enough input, making sure that enough
   state information is maintained to continue the loop where it left off
   if NEEDBITS() returns in the loop.  For example, want, need, and keep
   would all have to actually be part of the saved state in case NEEDBITS()
   returns:

    case STATEw:
        while (want < need) {
            NEEDBITS(n);
            keep[want++] = BITS(n);
            DROPBITS(n);
        }
        state = STATEx;
    case STATEx:

   As shown above, if the next state is also the next case, then the break
   is omitted.

   A state may also return if there is not enough output space available to
   complete that state.  Those states are copying stored data, writing a
   literal byte, and copying a matching string.

   When returning, a "goto inf_leave" is used to update the total counters,
   update the check value, and determine whether any progress has been made
   during that inflate() call in order to return the proper return code.
   Progress is defined as a change in either strm.avail_in or strm.avail_out.
   When there is a window, goto inf_leave will update the window with the last
   output written.  If a goto inf_leave occurs in the middle of decompression
   and there is no window currently, goto inf_leave will create one and copy
   output to the window for the next call of inflate().

   In this implementation, the flush parameter of inflate() only affects the
   return code (per zlib.h).  inflate() always writes as much as possible to
   strm.next_out, given the space available and the provided input--the effect
   documented in zlib.h of Z_SYNC_FLUSH.  Furthermore, inflate() always defers
   the allocation of and copying into a sliding window until necessary, which
   provides the effect documented in zlib.h for Z_FINISH when the entire input
   stream available.  So the only thing the flush parameter actually does is:
   when flush is set to Z_FINISH, inflate() cannot return Z_OK.  Instead it
   will return Z_BUF_ERROR if it has not reached the end of the stream.
 */

struct InflateLocals<'a>
{
    state: &'a mut Inflater,

    input_buffer: &'a [u8],
    output_buffer: &'a mut[u8],

    hold: u32,          // bit buffer
    bits: usize,         // bits in bit buffer
    next: usize,         // next input; is an index into input_buffer
    put: usize,          // next output; is an index into output_buffer

    flush: Flush,

    is_goto: bool,
}

impl<'a> InflateLocals<'a> {
    #[inline]
    pub fn avail_out(&self) -> usize {
        assert!(self.put <= self.output_buffer.len());
        self.output_buffer.len() - self.put
    }

    #[inline]
    pub fn avail_in(&self) -> usize {
        assert!(self.next <= self.input_buffer.len());
        self.input_buffer.len() - self.next
    }

    #[inline]
    pub fn left(&self) -> usize {
        self.output_buffer.len() - self.put
    }

    #[inline]
    pub fn have(&self) -> usize {
        self.input_buffer.len() - self.next
    }
}



/*
int ZEXPORT inflateGetDictionary(strm, dictionary, dictLength)
z_streamp strm;
Bytef *dictionary;
uInt *dictLength;
{
    struct Inflater FAR *state;

    /* check state */
    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct Inflater FAR *)strm.state;

    /* copy dictionary */
    if (state.whave && dictionary != Z_NULL) {
        copy_memory(dictionary, state.window + state.wnext,
                state.whave - state.wnext);
        copy_memory(dictionary + state.whave - state.wnext,
                state.window, state.wnext);
    }
    if (dictLength != Z_NULL)
        *dictLength = state.whave;
    return Z_OK;
}

int ZEXPORT inflateSetDictionary(strm, dictionary, dictLength)
z_streamp strm;
const Bytef *dictionary;
uInt dictLength;
{
    struct Inflater FAR *state;
    unsigned long dictid;
    int ret;

    /* check state */
    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct Inflater FAR *)strm.state;
    if (state.wrap != 0 && state.mode != DICT)
        return Z_STREAM_ERROR;

    /* check for correct dictionary identifier */
    if (state.mode == DICT) {
        dictid = adler32(0L, Z_NULL, 0);
        dictid = adler32(dictid, dictionary, dictLength);
        if (dictid != state.check)
            return Z_DATA_ERROR;
    }

    /* copy dictionary to window using updatewindow(), which will amend the
       existing dictionary if appropriate */
    ret = updatewindow(strm, dictionary + dictLength, dictLength);
    if (ret) {
        state.mode = MEM;
        return Z_MEM_ERROR;
    }
    state.havedict = 1;
    debug!("inflate:   dictionary set");
    return Z_OK;
}

int ZEXPORT inflateGetHeader(strm, head)
z_streamp strm;
gz_headerp head;
{
    struct Inflater FAR *state;

    /* check state */
    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct Inflater FAR *)strm.state;
    if ((state.wrap & 2) == 0) return Z_STREAM_ERROR;

    /* save header structure */
    state.head = head;
    head.done = 0;
    return Z_OK;
}

/*
   Search buf[0..len-1] for the pattern: 0, 0, 0xff, 0xff.  Return when found
   or when out of input.  When called, *have is the number of pattern bytes
   found in order so far, in 0..3.  On return *have is updated to the new
   state.  If on return *have equals four, then the pattern was found and the
   return value is how many bytes were read including the last byte of the
   pattern.  If *have is less than four, then the pattern has not been found
   yet and the return value is len.  In the latter case, syncsearch() can be
   called again with more data and the *have state.  *have is initialized to
   zero for the first call.
 */
local unsigned syncsearch(have, buf, len)
unsigned FAR *have;
const unsigned char FAR *buf;
unsigned len;
{
    unsigned got;
    unsigned next;

    got = *have;
    next = 0;
    while (next < len && got < 4) {
        if ((int)(buf[next]) == (got < 2 ? 0 : 0xff))
            got++;
        else if (buf[next])
            got = 0;
        else
            got = 4 - got;
        next++;
    }
    *have = got;
    return next;
}

int ZEXPORT inflateSync(strm)
z_streamp strm;
{
    unsigned len;               /* number of bytes to look at or looked at */
    unsigned long in, out;      /* temporary to save total_in and total_out */
    unsigned char buf[4];       /* to restore bit buffer to byte string */
    struct Inflater FAR *state;

    /* check parameters */
    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct Inflater FAR *)strm.state;
    if (strm.avail_in == 0 && state.bits < 8) return Z_BUF_ERROR;

    /* if first time, start search in bit buffer */
    if (state.mode != SYNC) {
        state.mode = SYNC;
        state.hold <<= state.bits & 7;
        state.bits -= state.bits & 7;
        len = 0;
        while (state.bits >= 8) {
            buf[len++] = (unsigned char)(state.hold);
            state.hold >>= 8;
            state.bits -= 8;
        }
        state.have = 0;
        syncsearch(&(state.have), buf, len);
    }

    /* search available input */
    len = syncsearch(&(state.have), strm.next_in, strm.avail_in);
    strm.avail_in -= len;
    strm.next_in += len;
    strm.total_in += len;

    /* return no joy or set up to restart inflate() on a new block */
    if (state.have != 4) return Z_DATA_ERROR;
    in = strm.total_in;  out = strm.total_out;
    inflateReset(strm);
    strm.total_in = in;  strm.total_out = out;
    state.mode = TYPE;
    return Z_OK;
}

/*
   Returns true if inflate is currently at the end of a block generated by
   Z_SYNC_FLUSH or Z_FULL_FLUSH. This function is used by one PPP
   implementation to provide an additional safety check. PPP uses
   Z_SYNC_FLUSH but removes the length bytes of the resulting empty stored
   block. When decompressing, PPP checks that at the end of input packet,
   inflate is waiting for these length bytes.
 */
int ZEXPORT inflateSyncPoint(strm)
z_streamp strm;
{
    struct Inflater FAR *state;

    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct Inflater FAR *)strm.state;
    return state.mode == STORED && state.bits == 0;
}

int ZEXPORT inflateCopy(dest, source)
z_streamp dest;
z_streamp source;
{
    struct Inflater FAR *state;
    struct Inflater FAR *copy;
    unsigned char FAR *window;
    unsigned wsize;

    /* check input */
    if (dest == Z_NULL || source == Z_NULL || source.state == Z_NULL ||
        source.zalloc == (alloc_func)0 || source.zfree == (free_func)0)
        return Z_STREAM_ERROR;
    state = (struct Inflater FAR *)source.state;

    /* allocate space */
    copy = (struct Inflater FAR *)
           ZALLOC(source, 1, sizeof(struct Inflater));
    if (copy == Z_NULL) return Z_MEM_ERROR;
    window = Z_NULL;
    if (state.window != Z_NULL) {
        window = (unsigned char FAR *)
                 ZALLOC(source, 1U << state.wbits, sizeof(unsigned char));
        if (window == Z_NULL) {
            ZFREE(source, copy);
            return Z_MEM_ERROR;
        }
    }

    /* copy state */
    copy_memory((voidpf)dest, (voidpf)source, sizeof(z_stream));
    copy_memory((voidpf)copy, (voidpf)state, sizeof(struct Inflater));
    if (state.lencode >= state.codes &&
        state.lencode <= state.codes + ENOUGH - 1) {
        copy.lencode = copy.codes + (state.lencode - state.codes);
        copy.distcode = copy.codes + (state.distcode - state.codes);
    }
    copy.next = copy.codes + (state.next - state.codes);
    if (window != Z_NULL) {
        wsize = 1U << state.wbits;
        copy_memory(window, state.window, wsize);
    }
    copy.window = window;
    dest.state = (struct internal_state FAR *)copy;
    return Z_OK;
}

int ZEXPORT inflateUndermine(strm, subvert)
z_streamp strm;
int subvert;
{
    struct Inflater FAR *state;

    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct Inflater FAR *)strm.state;
    state.sane = !subvert;
#ifdef INFLATE_ALLOW_INVALID_DISTANCE_TOOFAR_ARRR
    return Z_OK;
#else
    state.sane = 1;
    return Z_DATA_ERROR;
#endif
}

long ZEXPORT inflateMark(strm)
z_streamp strm;
{
    struct Inflater FAR *state;

    if (strm == Z_NULL || strm.state == Z_NULL) return -1L << 16;
    state = (struct Inflater FAR *)strm.state;
    return ((long)(state.back) << 16) +
        (state.mode == COPY ? state.length :
            (state.mode == MATCH ? state.was - state.length : 0));
}
*/

