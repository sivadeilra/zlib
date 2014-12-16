
// Copyright (C) 1995-2009 Mark Adler
// For conditions of distribution and use, see copyright notice in zlib.h

// http://www.gzip.org/zlib/rfc-gzip.html

use std::slice::bytes::copy_memory;

use crc32::crc32;
use adler32::adler32;
use self::inffast::inflate_fast;
use self::inffast::BufPos;
use self::inftrees::inflate_table;
use std::default::Default;
use GZipHeader;
use ZStream;
use DEF_WBITS;
use swap32;
use Flush;
use Z_DEFLATED;
use WINDOW_BITS_MIN;
use WINDOW_BITS_MAX;

pub use self::reader::InflateReader;

const DEFAULT_DMAX: uint = 32768;

mod inffast;
mod inftrees;
mod reader;

macro_rules! BADINPUT {
    ($loc:expr, $msg:expr) => {
        {
panic!("bad input, total_in={}: {}", $loc.strm.total_in, $msg); /* TEMPORARY
            $loc.strm.msg = Some($msg.to_string());
            $loc.state.mode = InflateMode::BAD;
            return inf_leave($loc); */
        }
    }
}

// Get a byte of input into the bit accumulator, or return from inflat
// if there is no input available.
macro_rules! PULLBYTE {
    ($loc:expr) => {
        {
            if $loc.have == 0 {
                return inf_leave($loc);
            }
            $loc.have -= 1;
            let b = $loc.input_buffer[$loc.next];
            $loc.hold += b as u32 << $loc.bits;
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
            let n :uint = $n;
            while $loc.bits < n {
                PULLBYTE!($loc);
            }
        }
    }
}

// Structure for decoding tables.  Each entry provides either the
// information needed to do the operation requested by the code that
// indexed that table entry, or it provides a pointer to another
// table that indexes more bits of the code.  op indicates whether
// the entry is a pointer to another table, a literal, a length or
// distance, an end-of-block, or an invalid code.  For a table
// pointer, the low four bits of op is the number of index bits of
// that table.  For a length or distance, the low four bits of op
// is the number of extra bits to get after the code.  bits is
// the number of bits in this code or part of the code to drop off
// of the bit buffer.  val is the actual byte to output in the case
// of a literal, the base length or distance, or the offset from
// the current table to the next table.  Each entry is four bytes.
#[deriving(Copy,Default)]
struct Code
{
    // operation, extra bits, table bits
    // op values as set by inflate_table():
    // 00000000 - literal
    // 0000tttt - table link, tttt != 0 is the number of table index bits
    // 0001eeee - length or distance, eeee is the number of extra bits
    // 01100000 - end of block
    // 01000000 - invalid code
    op: u8,

    /// bits in this part of the code
    bits: u8,

    /// offset in table or code value
    val: u16,
}

// Maximum size of the dynamic table.  The maximum number of code structures is
// 1444, which is the sum of 852 for literal/length codes and 592 for distance
// codes.  These values were found by exhaustive searches using the program
// examples/enough.c found in the zlib distribtution.  The arguments to that
// program are the number of symbols, the initial root table size, and the
// maximum bit length of a code.  "enough 286 9 15" for literal/length codes
// returns returns 852, and "enough 30 6 15" for distance codes returns 592.
// The initial root table size (9 or 6) is found in the fifth argument of the
// inflate_table() calls in inflate.c and infback.c.  If the root table size is
// changed, then these maximum sizes would be need to be recalculated and
// updated.
const ENOUGH_LENS :uint = 852;
const ENOUGH_DISTS :uint = 592;
const ENOUGH :uint = ENOUGH_LENS + ENOUGH_DISTS;

/* Type of code to build for inflate_table() */
// enum codetype {
type CodeType = u8;
    pub const CODES: u8 = 0;
    pub const LENS: u8 = 1;
    pub const DISTS: u8 = 2;
// }

/// Describes the results of calling `inflate()`.
#[deriving(Copy)]
pub enum InflateResult
{
    Eof(u32),               // input data stream has reached its end; value is crc32 of stream
    NeedInput,              // could decode more, but need more input buffer space
    Decoded(uint, uint),    // decoded N bytes of input, wrote N bytes of output
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
#[deriving(Show,Copy,PartialEq,Eq)]
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

/// Defines the state needed to inflate (decompress) a stream.
/// Use InflateState::new() to create a stream.
pub struct InflateState // was inflate_state
{
    mode: InflateMode,          // current inflate mode
    last: bool,                 // true if processing last block
    wrap: u32,                  // bit 0 true for zlib, bit 1 true for gzip
    havedict: bool,             // true if dictionary provided
    flags: u32,                 // gzip header method and flags (0 if zlib)
    dmax: uint,                 // zlib header max distance (INFLATE_STRICT)
    check: u32,                 // protected copy of check value
    total: uint,                // protected copy of output count
    head: Option<GZipHeader>,   // where to save gzip header information

    // sliding window
    wbits: uint,                // log base 2 of requested window size
    wsize: uint,                // window size or zero if not using window
    whave: uint,                // valid bytes in the window
    wnext: uint,                // window write index
    window: Vec<u8>,            // allocated sliding window, if needed

    // bit accumulator
    hold: u32,                  // input bit accumulator
    bits: uint,                 // number of bits in "hold"

    // for string and stored block copying
    length: uint,               // literal or length of data to copy
    offset: uint,               // distance back to copy string from

    // for table and code decoding
    extra: uint,                // extra bits needed

    // fixed and dynamic code tables
    lencode: uint,              // starting table for length/literal codes; is an index into 'codes'
    distcode: uint,             // starting table for distance codes; is an index into 'codes'
    lenbits: uint,              // index bits for lencode
    distbits: uint,             // index bits for distcode

    // dynamic table building
    ncode: uint,                // number of code length code lengths
    nlen: uint,                 // number of length code lengths
    ndist: uint,                // number of distance code lengths
    have: uint,                 // number of code lengths in lens[]
    next: uint,                 // next available space in codes[]   // index into codes[]
    lens: [u16, ..320],         // temporary storage for code lengths
    work: [u16, ..288],         // work area for code table building
    codes: [Code, ..ENOUGH],    // space for code tables
    sane: bool,                 // if false, allow invalid distance too far
    back: uint,                 // bits back of last unprocessed length/lit
    was: uint,                  // initial length of match
}

impl InflateState
{
    pub fn new(window_bits: uint, wrap: u32) -> InflateState
    {
        assert!(window_bits >= WINDOW_BITS_MIN && window_bits <= WINDOW_BITS_MAX);

        let wsize: uint = 1 << window_bits;

        InflateState {
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
                w.grow(wsize, 0u8);
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
            lens: [0u16, ..320],        // temporary storage for code lengths
            work: [0u16, ..288],        // work area for code table building
            codes: [Default::default(), ..ENOUGH],    // space for code tables
            sane: false,                // if false, allow invalid distance too far
            back: 0,                    // bits back of last unprocessed length/lit
            was: 0,                     // initial length of match
        }
    }

    // was inflate_reset_keep 
    pub fn reset_keep(&mut self, strm: &mut ZStream)
    {
        strm.total_in = 0;
        strm.total_out = 0;
        self.total = 0;
        strm.msg = None;
        if self.wrap != 0 {
            // to support ill-conceived Java test suite
            strm.adler = self.wrap as u32 & 1;
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
        debug!("inflate: reset");
    }

    pub fn reset(&mut self, strm: &mut ZStream)
    {
        self.wsize = 0;
        self.whave = 0;
        self.wnext = 0;
        self.reset_keep(strm);
    }

    pub fn reset2(&mut self, strm: &mut ZStream, window_bits: int)
    {
        let wrap: u32;
        let mut wbits = window_bits;

        // extract wrap request from windowBits parameter
        if window_bits < 0 {
            wrap = 0;
            wbits = -wbits;
        }
        else {
            wrap = (wbits as u32 >> 4) + 1;
    // #ifdef GUNZIP
            if wbits < 48 {
                wbits &= 15;
            }
    // #endif
        }

        // set number of window bits, free window if different
        if wbits != 0 && (wbits < 8 || wbits > 15) {
            panic!("Z_STREAM_ERROR");
        }

        if self.window.len() != 0 && self.wbits != wbits as uint {
            self.window.clear();
        }

        // update state and reset the rest of it
        self.wrap = wrap;
        self.wbits = wbits as uint;
        self.reset(strm);
    }

    pub fn init2(&mut self, strm: &mut ZStream, window_bits: int)
    {
        strm.msg = None;                 // in case we return an error
        self.window.clear();
        self.reset2(strm, window_bits);
    }

    pub fn init(&mut self, strm: &mut ZStream)
    {
        self.init2(strm, DEF_WBITS as int);
    }

    pub fn prime(&mut self, strm: &mut ZStream, bits: int, value: u32)
    {
        if bits < 0 {
            self.hold = 0;
            self.bits = 0;
            return;
        }

        assert!(bits <= 16);
        assert!(self.bits as int + bits <= 32);

        let val = value & (1 << bits as uint) - 1;
        self.hold += val << self.bits;
        self.bits += bits as uint;
    }

    // The 'a lifetime allows inflate() to use input/output streams,
    // whose lifetime is constrained to be less than that of strm/state.
    pub fn inflate<'a>(
        &mut self,
        strm: &mut ZStream,
        flush: Option<Flush>,
        input_buffer: &'a [u8],
        output_buffer: &'a mut[u8]) -> InflateResult
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

        let strm_avail_out = output_buffer.len();
        let mut locs = InflateLocals {
            strm: strm,
            state: self,
            input_buffer: input_buffer,
            output_buffer: output_buffer,
            have: 0,
            left: 0,
            hold: 0,
            bits: 0,
            next: 0,
            put: 0,
            in_: 0,
            out: 0,
            flush: flush,
            strm_next_in: 0,
            strm_avail_in: input_buffer.len(),
            strm_next_out: 0,
            strm_avail_out: strm_avail_out,
            is_goto: false,
        };
        let loc = &mut locs;

        let mut copy: uint;         // number of stored or match bytes to copy
        let mut last: Code;         // parent table entry
        let mut len: uint;          // length to copy for repeats, bits to drop
        let mut ret: uint;          // return code

        static ORDER: [u16, ..19] = /* permutation of code lengths */
            [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

        match loc.state.mode {
            InflateMode::TYPE => {
                loc.state.mode = InflateMode::TYPEDO;      /* skip check */
            }
            _ => ()
        }
        load_locals(loc);

        loc.in_ = loc.have;
        loc.out = loc.left;

        // ret = Z_OK;
        loop {
            let oldmode = loc.state.mode;
            // debug!("inflate: mode = {}", loc.state.mode);

            if loc.is_goto {
                loc.is_goto = false;
            }
            else {
                debug!("inflate: mode={}", loc.state.mode as u32);
            }

            match loc.state.mode {
            InflateMode::HEAD => {
                if loc.state.wrap == 0 {
                    debug!("HEAD - wrap = 0, switching to TYPEDO");
                    loc.state.mode = InflateMode::TYPEDO;
                    continue;
                }
                debug!("HEAD: wrap: 0x{:x}", loc.state.wrap);
                NEEDBITS!(loc, 16);
    // #ifdef GUNZIP
                if (loc.state.wrap & 2) != 0 && loc.hold == 0x8b1f {  /* gzip header */
                    debug!("found GZIP header");
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
                    (((bits(loc, 8) as u32 << 8) + (loc.hold >> 8)) % 31) != 0 {
                    warn!("incorrect header check.  bits(8) = 0x{:2x}", bits(loc, 8));
                    BADINPUT!(loc, "incorrect header check");
                }
                if bits(loc, 4) != Z_DEFLATED as u32 {
                    BADINPUT!(loc, "unknown compression method");
                }
                dropbits(loc, 4);
                len = (bits(loc, 4) + 8) as uint;
                if loc.state.wbits == 0 {
                    loc.state.wbits = len;
                }
                else if len > loc.state.wbits {
                    BADINPUT!(loc, "invalid window size".to_string());
                }
                loc.state.dmax = 1 << len;
                // debug!("max distance (dmax) = {} 0x{:x}", loc.state.dmax, loc.state.dmax);
                // debug!("inflate:   zlib header ok");
                let adler_value = adler32(0, &[]);
                loc.strm.adler = adler_value;
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
                    loc.state.length = extra_len as uint;

                    // debug!("EXTRALEN: extra_len = {}", extra_len);

                    match loc.state.head {
                        Some(ref mut h) => {
                            h.extra_len = extra_len as uint;
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
                    if copy > loc.have {
                        copy = loc.have;
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
                        loc.have -= copy;
                        loc.next += copy;
                        loc.state.length -= copy;
                    }
                    if loc.state.length != 0 {
                        return inf_leave(loc);
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
                    if loc.have == 0 {
                        return inf_leave(loc);
                    }
                    let mut copy = 0;
                    loop {
                        len = loc.input_buffer[loc.next + copy] as uint;
                        copy += 1;

                        /* TODO
                        if (state.head != Z_NULL && state.head.name != Z_NULL && state.length < state.head.name_max) {
                            state.head.name[state.length++] = len;
                        } */

                        if !(len != 0 && copy < loc.have) {
                             break;
                        }
                    }
                    if (loc.state.flags & 0x0200) != 0 {
                        loc.state.check = crc32(loc.state.check, loc.input_buffer.slice(loc.next, loc.next + copy));
                    }
                    loc.have -= copy;
                    loc.next += copy;
                    if len != 0 { return inf_leave(loc); }
                }
                else
                {
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
                    if loc.have == 0 {
                        // debug!("have no data, returning");
                        return inf_leave(loc);
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

                        if !(len != 0 && copy < loc.have) {
                            break;
                        }
                    }
                    if (loc.state.flags & 0x0200) != 0 {
                        loc.state.check = crc32(loc.state.check, input_buffer.slice(loc.next, loc.next + copy));
                    }
                    loc.have -= copy;
                    loc.next += copy;
                    if len != 0 {
                        // We have not received all of the bytes for the comment, so bail.
                        return inf_leave(loc);
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
                loc.strm.adler = initial_crc;
                loc.state.check = initial_crc;
                loc.state.mode = InflateMode::TYPE;
            }
    // #endif
            InflateMode::DICTID => {
                NEEDBITS!(loc, 32);
                let check = swap32(loc.hold);
                // debug!("check = 0x{:x}", check);
                loc.strm.adler = check;
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
                loc.strm.adler = check;
                loc.state.check = check;
                goto_mode!(loc, TYPE);
            }

            InflateMode::TYPE => {
                if flush == Flush::Block || flush == Flush::Trees {
                    debug!("TYPE: flush is Z_BLOCK or Z_TREES, returning");
                    return inf_leave(loc);
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
                            loc.state.fixedtables(loc.strm);
                            if loc.state.last {
                                debug!("inflate:     fixed codes block (last)");
                            }
                            else {
                                debug!("inflate:      fixed codes block");
                            }
                            loc.state.mode = InflateMode::LEN_; // decode codes
                            if flush == Flush::Trees {
                                dropbits(loc, 2);
                                return inf_leave(loc);
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
                }
            }

            InflateMode::STORED => {
                bytebits(loc);                         /* go to byte boundary */
                NEEDBITS!(loc, 32);
                let len = loc.hold & 0xffff;
                let invlen = loc.hold >> 16;
                if len != invlen {
                    BADINPUT!(loc, "invalid stored block lengths");
                }
                debug!("STORED: len = {}", len);
                loc.state.length = len as uint;
                initbits(loc);
                loc.state.mode = InflateMode::COPY_;
                if flush == Flush::Trees {
                    debug!("flush = Z_TREES, so returning");
                    return inf_leave(loc);
                }
                goto_mode!(loc, COPY_);
            }
            InflateMode::COPY_ => {
                goto_mode!(loc, COPY);
            }
            InflateMode::COPY => {
                copy = loc.state.length;
                if copy != 0 {
                    debug!("copy length = {}", copy);
                    if copy > loc.have { copy = loc.have; }
                    if copy > loc.left { copy = loc.left; }
                    if copy == 0 {
                        debug!("cannot copy data right now (no buffer space) -- exiting");
                        return inf_leave(loc);
                    }
                    copy_memory(loc.output_buffer.slice_mut(loc.put, copy), loc.input_buffer.slice(loc.next, loc.next + copy));
                    loc.have -= copy;
                    loc.next += copy;
                    loc.left -= copy;
                    loc.put += copy;
                    loc.state.length -= copy;
                    // stay in state COPY
                }
                else {
                    debug!("inflate: stored end");
                    loc.state.mode = InflateMode::TYPE;
                }
            }

            InflateMode::TABLE => {
                NEEDBITS!(loc, 14);
                loc.state.nlen = bits_and_drop(loc, 5) as uint + 257;
                loc.state.ndist = bits_and_drop(loc, 5) as uint + 1;
                loc.state.ncode = bits_and_drop(loc, 4) as uint + 4;
                // debug!("TABLE: nlen {} ndist {} ncode {}", loc.state.nlen, loc.state.ndist, loc.state.ncode);
    // #ifndef PKZIP_BUG_WORKAROUND
                if loc.state.nlen > 286 || loc.state.ndist > 30 {
                    BADINPUT!(loc, "too many length or distance symbols");
                }
    // #endif
                loc.state.have = 0;
                goto_mode!(loc, LENLENS);
            }

            InflateMode::LENLENS => {
                // debug!("have = {}, ncode = {}, reading {} lengths", loc.state.have, loc.state.ncode, loc.state.ncode - loc.state.have);
                while loc.state.have < loc.state.ncode {
                    NEEDBITS!(loc, 3);
                    let lenlen = bits(loc, 3);
                    let lenindex = ORDER[loc.state.have] as uint;
                    // debug!("    lens[{}] := {}", lenindex , lenlen);
                    loc.state.lens[lenindex ] = lenlen as u16;
                    loc.state.have += 1;
                    dropbits(loc, 3);
                }
                while loc.state.have < 19 {
                    let lenindex = ORDER[loc.state.have] as uint;
                    debug!("clearing {}", lenindex);
                    loc.state.lens[lenindex] = 0;
                    loc.state.have += 1;
                }
                // debug!("inflating code lengths");
                loc.state.next = 0;
                loc.state.lencode = loc.state.next;
                loc.state.lenbits = 7;
                let (inflate_ret, inflate_bits) = inflate_table(CODES, &loc.state.lens, 19, &mut loc.state.codes, &mut loc.state.next,
                    loc.state.lenbits, loc.state.work.as_mut_slice());
                ret = inflate_ret as uint;
                loc.state.lenbits = inflate_bits;
                if ret != 0 {
                    BADINPUT!(loc, "invalid code lengths set");
                }
                // debug!("code lengths are ok");
                loc.state.have = 0;
                goto_mode!(loc, CODELENS);
            }
            InflateMode::CODELENS => {
                while loc.state.have < loc.state.nlen + loc.state.ndist {
                    let mut here: Code; // current decoding table entry
                    while { here = loc.state.codes[loc.state.lencode + bits(loc, loc.state.lenbits) as uint]; here.bits as uint > loc.bits } {
                        PULLBYTE!(loc);
                    }
                    if here.val < 16 {
                        dropbits(loc, here.bits as uint);
                        loc.state.lens[loc.state.have] = here.val;
                        loc.state.have += 1;
                    }
                    else {
                        let (len, copy) = if here.val == 16 {
                            NEEDBITS!(loc, here.bits as uint + 2);
                            dropbits(loc, here.bits as uint);
                            if loc.state.have == 0 {
                                BADINPUT!(loc, "invalid bit length repeat");
                            }
                            (loc.state.lens[loc.state.have as uint - 1], 3 + bits_and_drop(loc, 2) as uint)
                        }
                        else if here.val == 17 {
                            NEEDBITS!(loc, here.bits as uint + 3);
                            dropbits(loc, here.bits as uint);
                            (0, 3 + bits_and_drop(loc, 3) as uint)
                        }
                        else {
                            NEEDBITS!(loc, here.bits as uint + 7);
                            dropbits(loc, here.bits as uint);
                            (0, 11 + bits_and_drop(loc, 7) as uint)
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
                ret = inflate_result as uint;
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
                // debug!("codes are ok");
                loc.state.mode = InflateMode::LEN_;
                if flush == Flush::Trees {
                    debug!("flush = Z_TREES, returning");
                    return inf_leave(loc);
                }
                goto_mode!(loc, LEN_);
            }
            InflateMode::LEN_ => {
                goto_mode!(loc, LEN);
            }
            InflateMode::LEN => {
                debug!("LEN: left={}", loc.left); // fast path isn't correct yet
                if loc.have >= 6 && loc.left >= 258 && false {
                    debug!("LEN: fast path");
                    restore_locals(loc);
                    inflate_fast(loc.state, loc.strm, loc.input_buffer, loc.output_buffer, 
                        &mut loc.strm_next_in,
                        &mut loc.strm_avail_in,
                        &mut loc.strm_next_out,
                        &mut loc.strm_avail_out,
                        loc.out);
                    load_locals(loc);
                    if loc.state.mode == InflateMode::TYPE {
                        loc.state.back = -1;
                    }
                }
                else {
                    debug!("LEN: slow path");
                    loc.state.back = 0;
                    let mut here: Code;         // current decoding table entry
                    loop {
                        here = loc.state.codes[loc.state.lencode + bits(loc, loc.state.lenbits) as uint];
                        if here.bits as uint <= loc.bits as uint {
                            break;
                        }
                        PULLBYTE!(loc);
                    }
                    if here.op != 0 && (here.op & 0xf0) == 0 {
                        last = here;
                        loop {
                            here = loc.state.codes[loc.state.lencode + last.val as uint + (bits(loc, last.bits as uint + last.op as uint) as uint >> last.bits as uint)];
                            if (last.bits as uint + here.bits as uint) <= loc.bits {
                                break;
                            }
                            PULLBYTE!(loc);
                        }
                        dropbits(loc, last.bits as uint);
                        loc.state.back += last.bits as uint;
                    }
                    dropbits(loc, here.bits as uint);
                    loc.state.back += here.bits as uint;
                    loc.state.length = here.val as uint;
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
                    loc.state.extra = (here.op & 15) as uint;
                    goto_mode!(loc, LENEXT);
                }
            }

            InflateMode::LENEXT => {
                debug!("LENEXT: extra={}", loc.state.extra);
                if loc.state.extra != 0 {
                    NEEDBITS!(loc, loc.state.extra);
                    loc.state.length += bits(loc, loc.state.extra as uint) as uint;
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
                    here = loc.state.codes[loc.state.distcode as uint + bits(loc, loc.state.distbits) as uint];
                    if here.bits as uint <= loc.bits {
                        break;
                    }
                    PULLBYTE!(loc);
                }
                if (here.op & 0xf0) == 0 {
                    last = here;
                    loop {
                        here = loc.state.codes[loc.state.distcode + last.val as uint + (bits(loc, last.bits as uint + last.op as uint) >> last.bits as uint) as uint];
                        if (last.bits as uint + here.bits as uint) <= loc.bits as uint {
                            break;
                        }
                        PULLBYTE!(loc);
                    }
                    dropbits(loc, last.bits as uint);
                    loc.state.back += last.bits as uint;
                }
                dropbits(loc, here.bits as uint);
                loc.state.back += here.bits as uint;
                if (here.op & 64) != 0 {
                    BADINPUT!(loc, "invalid distance code");
                }
                loc.state.offset = here.val as uint;
                loc.state.extra = (here.op & 15) as uint;
                goto_mode!(loc, DISTEXT);
            }

            InflateMode::DISTEXT => {
                if loc.state.extra != 0 {
                    NEEDBITS!(loc, loc.state.extra);
                    loc.state.offset += bits(loc, loc.state.extra) as uint;
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
                if loc.left == 0 {
                    debug!("MATCH: inf_leave");
                    return inf_leave(loc);
                }
                let mut copy = loc.out - loc.left;
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
                        if copy > loc.left { copy = loc.left; }
                        loc.left -= copy;
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

                    if copy > loc.left {
                        debug!("copy={} > left={}, setting copy={}", copy, loc.left, copy);
                        copy = loc.left;
                    }
                    loc.left -= copy;
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

                    if copy > loc.left {
                        debug!("copy={} > left={}, setting copy={}", copy, loc.left, copy);
                        copy = loc.left;
                    }
                    loc.left -= copy;
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
                if loc.left == 0 {
                    return inf_leave(loc);
                }
                // debug!("LIT: write {}", loc.state.length as u8);
                loc.output_buffer[loc.put] = loc.state.length as u8;
                loc.put += 1;
                loc.left -= 1;
                loc.state.mode = InflateMode::LEN;
            }

            InflateMode::CHECK => {
                // let mut from: uint; // index into loc.input_buffer
                if loc.state.wrap != 0 {
                    NEEDBITS!(loc, 32);
                    loc.out -= loc.left;
                    loc.strm.total_out += loc.out as u64;
                    loc.state.total += loc.out;
                    if loc.out != 0 {
                        let check = update(loc.state.flags, loc.state.check, loc.output_buffer.slice(loc.put - loc.out, loc.put));
                        loc.strm.adler = check;
                        loc.state.check = check;
                    }
                    loc.out = loc.left;
    // #ifdef GUNZIP
                    let ch = if loc.state.flags != 0 { loc.hold } else { swap32(loc.hold) };
                    if ch != loc.state.check {
                        BADINPUT!(loc, "incorrect data check");
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
                    if loc.hold != (loc.state.total & 0xffffffff) as u32 {
                        BADINPUT!(loc, "incorrect length check");
                    }
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
                return inf_leave(loc);
            }

            InflateMode::BAD => {
                debug!("BAD state -- input data is invalid");
                match loc.strm.msg {
                    Some(ref errmsg) => {
                        debug!("message: {}", errmsg);
                    }
                    _ => {}
                }
                panic!();
                // ret = Z_DATA_ERROR;
                // return inf_leave(loc);
            }
            /*
            case MEM:
                return Z_MEM_ERROR;
            case SYNC:
            default:
                return Z_STREAM_ERROR;
            */

                _ => {
                    warn!("unimplemented mode: {}", loc.state.mode);
                    unimplemented!();
                }
            }

            // if loc.state.mode != oldmode {
            //     debug!("mode {} --> {}", oldmode, loc.state.mode);
            // }

            // continue processing
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
    fn fixedtables(&mut self, strm: &mut ZStream)
    {
        debug!("fixedtables");

        let mut fixed: [Code, ..544] = [Default::default(), ..544];

        // build fixed huffman tables

        /* literal/length table */
        {
            let mut sym :uint = 0;
            while sym < 144 { self.lens[sym] = 8; sym += 1; }
            while sym < 256 { self.lens[sym] = 9; sym += 1; }
            while sym < 280 { self.lens[sym] = 7; sym += 1; }
            while sym < 288 { self.lens[sym] = 8; sym += 1; }
        }

        let mut next :uint = 0;     // index into 'fixed' table
        let lenfix: uint = 0;       // index into 'fixed' table
        let (err, bits) = inflate_table(LENS, &self.lens, 288, &mut fixed, &mut next, 9, self.work.as_mut_slice());
        assert!(err == 0);

        /* distance table */
        {
            let mut sym :uint = 0;
            while sym < 32 { self.lens[sym] = 5; sym += 1; }
        }
        let distfix: uint = next;      // index into 'fixed' table

        let (err, bits) = inflate_table(DISTS, &self.lens, 32, &mut fixed, &mut next, 5, self.work.as_mut_slice());
        assert!(err == 0);

    // #else /* !BUILDFIXED */
    // #   include "inffixed.h"
    // #endif /* BUILDFIXED */
        self.lencode = lenfix;
        self.lenbits = 9;
        self.distcode = distfix;
        self.distbits = 5;
    }
}

/*
#ifdef MAKEFIXED
#include <stdio.h>

/*
   Write out the inffixed.h that is #include'd above.  Defining MAKEFIXED also
   defines BUILDFIXED, so the tables are built on the fly.  makefixed() writes
   those tables to stdout, which would be piped to inffixed.h.  A small program
   can simply call makefixed to do this:

    void makefixed(void);

    int main(void)
    {
        makefixed();
        return 0;
    }

   Then that can be linked with zlib built with MAKEFIXED defined and run:

    a.out > inffixed.h
 */
void makefixed()
{
    unsigned low, size;
    struct InflateState state;

    fixedtables(&state);
    puts("    /* inffixed.h -- table for decoding fixed codes");
    puts("     * Generated automatically by makefixed().");
    puts("     */");
    puts("");
    puts("    /* WARNING: this file should *not* be used by applications.");
    puts("       It is part of the implementation of this library and is");
    puts("       subject to change. Applications should only use zlib.h.");
    puts("     */");
    puts("");
    size = 1U << 9;
    printf("    static const code lenfix[%u] = {", size);
    low = 0;
    for (;;) {
        if ((low % 7) == 0) printf("\n        ");
        printf("{%u,%u,%d}", (low & 127) == 99 ? 64 : state.lencode[low].op,
               state.lencode[low].bits, state.lencode[low].val);
        if (++low == size) break;
        putchar(',');
    }
    puts("\n    };");
    size = 1U << 5;
    printf("\n    static const code distfix[%u] = {", size);
    low = 0;
    for (;;) {
        if ((low % 6) == 0) printf("\n        ");
        printf("{%u,%u,%d}", state.distcode[low].op, state.distcode[low].bits,
               state.distcode[low].val);
        if (++low == size) break;
        putchar(',');
    }
    puts("\n    };");
}
#endif /* MAKEFIXED */

*/

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
fn updatewindow(loc: &mut InflateLocals, end: uint, copy: uint)
{
    debug!("updatewindow: copy={}", copy);

    let mut copy = copy;
    let mut dist: uint;

    /* if it hasn't been done already, allocate space for the window */

    // loc.state.window.clear();
    // loc.state.window.grow(1 << loc.state.wbits, 0);

    /* if window not in use yet, initialize */
    if loc.state.wsize == 0 {
        debug!("wsize=0, initializing window, wbits={}", loc.state.wbits);
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
        debug!("partial window fill\n");
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
// #ifdef GUNZIP
// was UPDATE
fn update(flags: u32, check: u32, data: &[u8]) -> u32
{
    if flags != 0 {
        crc32(check, data)
    }
    else {
        adler32(check, data)
    }
}
/*#else
#  define UPDATE(check, buf, len) adler32(check, buf, len)
#endif
*/

/* check macros for header crc */
// #ifdef GUNZIP
// #  define CRC2(check, word) \

// Computes a CRC over two bytes.  The bytes are stored in a u32 value.
// The bits are packed in "little-endian" form; byte[0] is in bits [0..7],
// while byte[1] is in bits [8..15].
fn crc2(check: u32, word: u32) -> u32
{
    let mut hbuf :[u8, ..2] = [0, ..2];
    hbuf[0] = (word & 0xff) as u8;
    hbuf[1] = ((word >> 8) & 0xff) as u8;
    return crc32(check, &hbuf);
}

// Computes a CRC over four bytes.  The bytes are stored in a u32 value.
// The bits are packed in "little-endian" form; byte[0] is in bits [0..7],
// while byte[1] is in bits [8..15], etc.
fn crc4(check: u32, word: u32) -> u32
{
    let mut hbuf = [0u8, ..4];
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
    loc.put = loc.strm_next_out;
    loc.left = loc.strm_avail_out;
    loc.next = loc.strm_next_in;
    loc.have = loc.strm_avail_in;
    loc.hold = loc.state.hold;
    loc.bits = loc.state.bits;

    // debug!("load_locals: put: {} left: {} next: {} have: {} hold: {} bits: {}",
    //     loc.put, loc.left, loc.next, loc.have, loc.hold, loc.bits);
}

/* Restore state from registers in inflate() */
// was RESTORE
#[inline]
fn restore_locals(loc: &mut InflateLocals) {
    // debug!("restore_locals: put: {} left: {} next: {} have: {} hold: {} bits: {}",
    //     loc.put, loc.left, loc.next, loc.have, loc.hold, loc.bits);

    loc.strm_next_out = loc.put;
    loc.strm_avail_out = loc.left;
    loc.strm_next_in = loc.next;
    loc.strm_avail_in = loc.have;
    loc.state.hold = loc.hold;
    loc.state.bits = loc.bits;
}

// Clear the input bit accumulator
fn initbits(loc: &mut InflateLocals) {
    loc.hold = 0;
    loc.bits = 0;
}


/* Return the low n bits of the bit accumulator (n < 16) */
// was 'BITS'
fn bits(loc: &InflateLocals, n: uint) -> u32
{
    loc.hold & ((1 << n) - 1)
}

// was BITBOOL
fn bitbool(loc: &InflateLocals) -> bool
{
    bits(loc, 1) != 0
}

/* Remove n bits from the bit accumulator */
// was 'DROPBITS'
fn dropbits(loc: &mut InflateLocals, n: uint)
{
    loc.hold >>= n;
    loc.bits -= n;
}

fn bits_and_drop(loc: &mut InflateLocals, n: uint) -> u32
{
    let v = bits(loc, n);
    dropbits(loc, n);
    v
}

/* Remove zero to seven bits as needed to go to a byte boundary */
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
    strm: &'a mut ZStream,
    state: &'a mut InflateState,

    input_buffer: &'a [u8],
    output_buffer: &'a mut[u8],

    have: uint,         // available input
    left: uint,         // available output
    hold: u32,          // bit buffer
    bits: uint,         // bits in bit buffer
    next: uint,         // next input; is an index into input_buffer
    put: uint,          // next output; is an index into output_buffer
    in_: uint,          // save starting available input
    out: uint,          // save starting available output

    flush: Flush,

    strm_next_in: uint,     // value moved from ZStream.next_in to here
    strm_avail_in: uint,    // value moved from ZStream.avail_in to here
    strm_next_out: uint,
    strm_avail_out: uint,

    is_goto: bool,
}

fn inf_leave(loc: &mut InflateLocals) -> InflateResult
{
    // Return from inflate(), updating the total counts and the check value.
    // If there was no progress during the inflate() call, return a buffer
    // error.  Call updatewindow() to create and/or update the window state.
    // Note: a memory error from inflate() is non-recoverable.

    debug!("inf_leave");
    restore_locals(loc);

    debug!("left={}", loc.left);

    if loc.state.wsize != 0 || (loc.out != loc.strm_avail_out && (loc.state.mode as u32) < (InflateMode::BAD as u32) &&
            ((loc.state.mode as u32) < (InflateMode::CHECK as u32) || (loc.flush != Flush::Finish))) {
        debug!("calling updatewindow()");
        assert!(loc.out >= loc.strm_avail_out);
        let e = loc.strm_next_out;
        let c = loc.out - loc.strm_avail_out;
        updatewindow(loc, e, c);
    }

    loc.in_ -= loc.strm_avail_in;
    loc.out -= loc.strm_avail_out;

    debug!("in={} out={}", loc.in_, loc.out);

    let in_inflated = loc.in_;
    let out_inflated = loc.out;

    loc.strm.total_in += loc.in_ as u64;
    loc.strm.total_out += loc.out as u64;
    loc.state.total += loc.out;
    if loc.state.wrap != 0 && loc.out != 0 {
        let updated_check = update(loc.state.flags, loc.state.check, loc.output_buffer.slice(loc.strm_next_out - loc.out, loc.strm_next_out));
        loc.strm.adler = updated_check;
        loc.state.check = updated_check;
    }
    loc.strm.data_type = loc.state.bits
        + (if loc.state.last { 64 } else { 0 })
        + (if loc.state.mode as u32 == InflateMode::TYPE as u32 { 128 } else { 0 })
        + (if loc.state.mode as u32 == InflateMode::LEN_ as u32 || loc.state.mode as u32 == InflateMode::COPY_ as u32 { 256 } else { 0 });

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
        InflateResult::NeedInput
    }
}


// This is actually unnecessary in Rust.
pub fn inflate_end(strm :&ZStream)
{
}

/*
int ZEXPORT inflateGetDictionary(strm, dictionary, dictLength)
z_streamp strm;
Bytef *dictionary;
uInt *dictLength;
{
    struct InflateState FAR *state;

    /* check state */
    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct InflateState FAR *)strm.state;

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
    struct InflateState FAR *state;
    unsigned long dictid;
    int ret;

    /* check state */
    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct InflateState FAR *)strm.state;
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
    struct InflateState FAR *state;

    /* check state */
    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct InflateState FAR *)strm.state;
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
    struct InflateState FAR *state;

    /* check parameters */
    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct InflateState FAR *)strm.state;
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
    struct InflateState FAR *state;

    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct InflateState FAR *)strm.state;
    return state.mode == STORED && state.bits == 0;
}

int ZEXPORT inflateCopy(dest, source)
z_streamp dest;
z_streamp source;
{
    struct InflateState FAR *state;
    struct InflateState FAR *copy;
    unsigned char FAR *window;
    unsigned wsize;

    /* check input */
    if (dest == Z_NULL || source == Z_NULL || source.state == Z_NULL ||
        source.zalloc == (alloc_func)0 || source.zfree == (free_func)0)
        return Z_STREAM_ERROR;
    state = (struct InflateState FAR *)source.state;

    /* allocate space */
    copy = (struct InflateState FAR *)
           ZALLOC(source, 1, sizeof(struct InflateState));
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
    copy_memory((voidpf)copy, (voidpf)state, sizeof(struct InflateState));
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
    struct InflateState FAR *state;

    if (strm == Z_NULL || strm.state == Z_NULL) return Z_STREAM_ERROR;
    state = (struct InflateState FAR *)strm.state;
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
    struct InflateState FAR *state;

    if (strm == Z_NULL || strm.state == Z_NULL) return -1L << 16;
    state = (struct InflateState FAR *)strm.state;
    return ((long)(state.back) << 16) +
        (state.mode == COPY ? state.length :
            (state.mode == MATCH ? state.was - state.length : 0));
}
*/

