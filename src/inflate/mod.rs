// http://www.gzip.org/zlib/rfc-gzip.html

use std::slice::bytes::copy_memory;
use crc32::crc32;
use adler32::adler32;
use self::inffast::inflate_fast;
use self::inffast::BufPos;
use self::inftrees::inflate_table;

use GZipHeader;
use ZStream;
use DEF_WBITS;
use swap32;
use {Z_DEFLATED,Z_BLOCK,Z_TREES,Z_FINISH};

const DEFAULT_DMAX: uint = 32768;

mod inffast;
mod inftrees;

/* Structure for decoding tables.  Each entry provides either the
   information needed to do the operation requested by the code that
   indexed that table entry, or it provides a pointer to another
   table that indexes more bits of the code.  op indicates whether
   the entry is a pointer to another table, a literal, a length or
   distance, an end-of-block, or an invalid code.  For a table
   pointer, the low four bits of op is the number of index bits of
   that table.  For a length or distance, the low four bits of op
   is the number of extra bits to get after the code.  bits is
   the number of bits in this code or part of the code to drop off
   of the bit buffer.  val is the actual byte to output in the case
   of a literal, the base length or distance, or the offset from
   the current table to the next table.  Each entry is four bytes. */
#[deriving(Copy)]
struct Code  // was 'code'
{
    op: u8,           /* operation, extra bits, table bits */
    bits: u8,         /* bits in this part of the code */
    val: u16,         /* offset in table or code value */
}

impl Code
{
    pub fn new() -> Code
    {
        Code { op: 0, bits: 0, val: 0 }
    }
}

/* op values as set by inflate_table():
    00000000 - literal
    0000tttt - table link, tttt != 0 is the number of table index bits
    0001eeee - length or distance, eeee is the number of extra bits
    01100000 - end of block
    01000000 - invalid code
 */

/* Maximum size of the dynamic table.  The maximum number of code structures is
   1444, which is the sum of 852 for literal/length codes and 592 for distance
   codes.  These values were found by exhaustive searches using the program
   examples/enough.c found in the zlib distribtution.  The arguments to that
   program are the number of symbols, the initial root table size, and the
   maximum bit length of a code.  "enough 286 9 15" for literal/length codes
   returns returns 852, and "enough 30 6 15" for distance codes returns 592.
   The initial root table size (9 or 6) is found in the fifth argument of the
   inflate_table() calls in inflate.c and infback.c.  If the root table size is
   changed, then these maximum sizes would be need to be recalculated and
   updated. */
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
    NeedOutput,             // could decode more, but need more output buffer space
    InvalidData,            // input data is malformed, decoding has halted
}

// inflate.h -- internal inflate state definition
// Copyright (C) 1995-2009 Mark Adler
// For conditions of distribution and use, see copyright notice in zlib.h

// /* define NO_GZIP when compiling if you want to disable gzip header and
//    trailer decoding by inflate().  NO_GZIP would be used to avoid linking in
//    the crc code when it is not needed.  For shared libraries, gzip decoding
//    should be left enabled. */
// #ifndef NO_GZIP
// #  define GUNZIP
// #endif
// */

/* Possible inflate modes between inflate() calls */
#[deriving(Show,Copy)]
pub enum InflateMode {
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

/* state maintained between inflate() calls.  Approximately 10K bytes. */
pub struct InflateState // was inflate_state
{
    mode: InflateMode,              // current inflate mode
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
    bits: uint,                 // number of bits in "in"

    // for string and stored block copying
    length: uint,               // literal or length of data to copy
    offset: uint,               // distance back to copy string from

    // for table and code decoding
    extra: uint,                // extra bits needed

    // fixed and dynamic code tables
    lencode: uint,              // starting table for length/literal codes       // index into 'codes'
    distcode: uint,             // starting table for distance codes        // index into 'codes'
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
    codes: [Code, ..ENOUGH],        // space for code tables
    pub sane: bool,                 // if false, allow invalid distance too far
    pub back: uint,                 // bits back of last unprocessed length/lit
    pub was: uint,                  // initial length of match
}

pub const WINDOW_BITS_MIN: uint = 8;
pub const WINDOW_BITS_MAX: uint = 15;
pub const WINDOW_BITS_DEFAULT: uint = WINDOW_BITS_MAX;

impl InflateState
{
    pub fn new(window_bits: uint) -> InflateState
    {
        assert!(window_bits >= WINDOW_BITS_MIN && window_bits <= WINDOW_BITS_MAX);

        let wsize: uint = 1 << window_bits;

        InflateState {
            mode: InflateMode::HEAD,
            last: false,
            wrap: 0,                    // bit 0 true for zlib, bit 1 true for gzip
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
            codes: [Code::new(), ..ENOUGH],    // space for code tables
            sane: false,                // if false, allow invalid distance too far
            back: 0,                    // bits back of last unprocessed length/lit
            was: 0,                     // initial length of match
        }
    }
}

// inflate.c -- zlib decompression
// Copyright (C) 1995-2012 Mark Adler
// For conditions of distribution and use, see copyright notice in zlib.h
//

/*
 * Change history:
 *
 * 1.2.beta0    24 Nov 2002
 * - First version -- complete rewrite of inflate to simplify code, avoid
 *   creation of window when not needed, minimize use of window when it is
 *   needed, make inffast.c even faster, implement gzip decoding, and to
 *   improve code readability and style over the previous zlib inflate code
 *
 * 1.2.beta1    25 Nov 2002
 * - Use pointers for available input and output checking in inffast.c
 * - Remove input and output counters in inffast.c
 * - Change inffast.c entry and loop from avail_in >= 7 to >= 6
 * - Remove unnecessary second byte pull from length extra in inffast.c
 * - Unroll direct copy to three copies per loop in inffast.c
 *
 * 1.2.beta2    4 Dec 2002
 * - Change external routine names to reduce potential conflicts
 * - Correct filename to inffixed.h for fixed tables in inflate.c
 * - Make hbuf[] unsigned char to match parameter type in inflate.c
 * - Change strm.next_out[-state.offset] to *(strm.next_out - state.offset)
 *   to avoid negation problem on Alphas (64 bit) in inflate.c
 *
 * 1.2.beta3    22 Dec 2002
 * - Add comments on state.bits assertion in inffast.c
 * - Add comments on op field in inftrees.h
 * - Fix bug in reuse of allocated window after inflateReset()
 * - Remove bit fields--back to byte structure for speed
 * - Remove distance extra == 0 check in inflate_fast()--only helps for lengths
 * - Change post-increments to pre-increments in inflate_fast(), PPC biased?
 * - Add compile time option, POSTINC, to use post-increments instead (Intel?)
 * - Make MATCH copy in inflate() much faster for when inflate_fast() not used
 * - Use local copies of stream next and avail values, as well as local bit
 *   buffer and bit count in inflate()--for speed when inflate_fast() not used
 *
 * 1.2.beta4    1 Jan 2003
 * - Split ptr - 257 statements in inflate_table() to avoid compiler warnings
 * - Move a comment on output buffer sizes from inffast.c to inflate.c
 * - Add comments in inffast.c to introduce the inflate_fast() routine
 * - Rearrange window copies in inflate_fast() for speed and simplification
 * - Unroll last copy for window match in inflate_fast()
 * - Use local copies of window variables in inflate_fast() for speed
 * - Pull out common wnext == 0 case for speed in inflate_fast()
 * - Make op and len in inflate_fast() unsigned for consistency
 * - Add FAR to lcode and dcode declarations in inflate_fast()
 * - Simplified bad distance check in inflate_fast()
 * - Added inflateBackInit(), inflateBack(), and inflateBackEnd() in new
 *   source file infback.c to provide a call-back interface to inflate for
 *   programs like gzip and unzip -- uses window as output buffer to avoid
 *   window copying
 *
 * 1.2.beta5    1 Jan 2003
 * - Improved inflateBack() interface to allow the caller to provide initial
 *   input in strm.
 * - Fixed stored blocks bug in inflateBack()
 *
 * 1.2.beta6    4 Jan 2003
 * - Added comments in inffast.c on effectiveness of POSTINC
 * - Typecasting all around to reduce compiler warnings
 * - Changed loops from while (1) or do {} while (1) to for (;;), again to
 *   make compilers happy
 * - Changed type of window in inflateBackInit() to unsigned char *
 *
 * 1.2.beta7    27 Jan 2003
 * - Changed many types to unsigned or unsigned short to avoid warnings
 * - Added inflateCopy() function
 *
 * 1.2.0        9 Mar 2003
 * - Changed inflateBack() interface to provide separate opaque descriptors
 *   for the in() and out() functions
 * - Changed inflateBack() argument and in_func typedef to swap the length
 *   and buffer address return values for the input function
 * - Check next_in and next_out for Z_NULL on entry to inflate()
 *
 * The history for versions after 1.2.0 are in ChangeLog in zlib distribution.
 */

fn inflate_reset_keep(strm: &mut ZStream, state: &mut InflateState)
{
    strm.total_in = 0;
    strm.total_out = 0;
    state.total = 0;
    strm.msg = None;
    if state.wrap != 0 {
        /* to support ill-conceived Java test suite */
        strm.adler = state.wrap as u32 & 1;
    }
    state.mode = InflateMode::HEAD;
    state.last = false;
    state.havedict = false;
    state.dmax = DEFAULT_DMAX;
    state.head = None;
    state.hold = 0;
    state.bits = 0;

    // state.lencode =
    // state.distcode =
    // state.next = state.codes;
    state.lencode = 0;      // index into state.codes
    state.distcode = 0;     // index into state.codes
    state.next = 0;         // index into state.codes

    state.sane = true;
    state.back = -1;
    debug!("inflate: reset");
}

pub fn inflate_reset(strm: &mut ZStream, state: &mut InflateState)
{
    state.wsize = 0;
    state.whave = 0;
    state.wnext = 0;
    inflate_reset_keep(strm, state);
}

pub fn inflate_reset2(strm: &mut ZStream, state: &mut InflateState, window_bits: int)
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

    if state.window.len() != 0 && state.wbits != wbits as uint {
        state.window.clear();
    }

    // update state and reset the rest of it
    state.wrap = wrap;
    state.wbits = wbits as uint;
    inflate_reset(strm, state);
}

pub fn inflate_init2(strm: &mut ZStream, state: &mut InflateState, window_bits: int)
{
    strm.msg = None;                 // in case we return an error
    state.window.clear();
    inflate_reset2(strm, state, window_bits);
}

pub fn inflate_init(strm: &mut ZStream, state: &mut InflateState)
{
    inflate_init2(strm, state, DEF_WBITS as int);
}

pub fn inflate_prime(strm: &mut ZStream, state: &mut InflateState, bits: int, value: u32)
{
    if bits < 0 {
        state.hold = 0;
        state.bits = 0;
        return;
    }

    assert!(bits <= 16);
    assert!(state.bits as int + bits <= 32);

    let val = value & (1 << bits as uint) - 1;
    state.hold += val << state.bits;
    state.bits += bits as uint;
}

// Return state with length and distance decoding tables and index sizes set to
// fixed code decoding.  Normally this returns fixed tables from inffixed.h.
// If BUILDFIXED is defined, then instead this routine builds the tables the
// first time it's called, and returns those tables the first time and
// thereafter.  This reduces the size of the code by about 2K bytes, in
// exchange for a little execution time.  However, BUILDFIXED should not be
// used for threaded applications, since the rewriting of the tables and virgin
// may not be thread-safe.
fn fixedtables(strm: &mut ZStream, state: &mut InflateState)
{
    debug!("fixedtables");

    let mut fixed: [Code, ..544] = [Code::new(), ..544];

    // build fixed huffman tables

    /* literal/length table */
    {
        let mut sym :uint = 0;
        while sym < 144 { state.lens[sym] = 8; sym += 1; }
        while sym < 256 { state.lens[sym] = 9; sym += 1; }
        while sym < 280 { state.lens[sym] = 7; sym += 1; }
        while sym < 288 { state.lens[sym] = 8; sym += 1; }
    }

    let mut next :uint = 0;         // index into 'fixed' table
    let lenfix: uint = 0;       // index into 'fixed' table
    let (err, bits) = inflate_table(LENS, &state.lens, 288, &mut fixed, &mut next, 9, state.work.as_mut_slice());
    assert!(err == 0);

    /* distance table */
    {
        let mut sym :uint = 0;
        while sym < 32 { state.lens[sym] = 5; sym += 1; }
    }
    let distfix: uint = next;      // index into 'fixed' table

    let (err, bits) = inflate_table(DISTS, &state.lens, 32, &mut fixed, &mut next, 5, state.work.as_mut_slice());
    assert!(err == 0);

// #else /* !BUILDFIXED */
// #   include "inffixed.h"
// #endif /* BUILDFIXED */
    state.lencode = lenfix;
    state.lenbits = 9;
    state.distcode = distfix;
    state.distbits = 5;
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
fn updatewindow(loc: &mut InflateLocals, end: uint, copy: uint)
{
    debug!("updatewindow: end = {}, copy = {}", end, copy);

    let mut copy = copy;
    let mut dist: uint;

    /* if it hasn't been done already, allocate space for the window */
    loc.state.window.clear();
    loc.state.window.grow(1 << loc.state.wbits, 0);

    /* if window not in use yet, initialize */
    if loc.state.wsize == 0 {
        loc.state.wsize = 1 << loc.state.wbits;
        loc.state.wnext = 0;
        loc.state.whave = 0;
    }

    /* copy state.wsize or less output bytes into the circular window */
    if copy >= loc.state.wsize {
        debug!("copy >= wsize, copy = {}, wsize = {}", copy, loc.state.wsize);
        copy_memory(loc.state.window.as_mut_slice(), subslice(loc.output_buffer, end - loc.state.wsize, loc.state.wsize));
        loc.state.wnext = 0;
        loc.state.whave = loc.state.wsize;
    }
    else {
        debug!("copy < wsize, copy = {}, wsize = {}", copy, loc.state.wsize);
        dist = loc.state.wsize - loc.state.wnext;
        if dist > copy {
            dist = copy;
        }
        debug!("copying from output_buffer[{}] to window[{}] length: {}", end - copy, loc.state.wnext, dist);
        copy_memory(
            loc.state.window.slice_mut(loc.state.wnext, loc.state.wnext + dist),
            loc.output_buffer.slice(end - copy, end - copy + dist));
        copy -= dist;
        if copy != 0 {
            debug!("copying second chunk, from output_buffer[{}] to window[0] length: {}", end - copy, copy);
            copy_memory(loc.state.window.as_mut_slice(), subslice(loc.output_buffer, end - copy, copy));
            loc.state.wnext = copy;
            loc.state.whave = loc.state.wsize;
        }
        else {
            loc.state.wnext += dist;
            if loc.state.wnext == loc.state.wsize {
                loc.state.wnext = 0;
            }
            if loc.state.whave < loc.state.wsize {
                loc.state.whave += dist;
            }
        }
    }
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

// was CRC2
fn crc2(check: u32, word: u32) -> u32       // returns 'check'
{
    let mut hbuf :[u8, ..2] = [0, ..2];
    hbuf[0] = (word & 0xff) as u8;
    hbuf[1] = ((word >> 8) & 0xff) as u8;
    return crc32(check, &hbuf);
}

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
fn load_locals(loc: &mut InflateLocals) {
    loc.put = loc.strm.next_out;
    loc.left = loc.strm.avail_out;
    loc.next = loc.strm.next_in;
    loc.have = loc.strm.avail_in;
    loc.hold = loc.state.hold;
    loc.bits = loc.state.bits;

    debug!("load_locals: put: {} left: {} next: {} have: {} hold: {} bits: {}",
        loc.put, loc.left, loc.next, loc.have, loc.hold, loc.bits);
}

/* Restore state from registers in inflate() */
// was RESTORE
fn restore_locals(loc: &mut InflateLocals) {
    debug!("restore_locals: put: {} left: {} next: {} have: {} hold: {} bits: {}",
        loc.put, loc.left, loc.next, loc.have, loc.hold, loc.bits);

    loc.strm.next_out = loc.put;
    loc.strm.avail_out = loc.left;
    loc.strm.next_in = loc.next;
    loc.strm.avail_in = loc.have;
    loc.state.hold = loc.hold;
    loc.state.bits = loc.bits;
}

macro_rules! BADINPUT {
    ($loc:expr, $msg:expr) => {
        {
            $loc.strm.msg = Some($msg.to_string());
            $loc.state.mode = InflateMode::BAD;
            return inf_leave($loc);
        }
    }
}

/* Clear the input bit accumulator */
fn initbits(loc: &mut InflateLocals) {
    loc.hold = 0;
    loc.bits = 0;
}

/* Get a byte of input into the bit accumulator, or return from inflate()
   if there is no input available. */
macro_rules! PULLBYTE {
    ($loc:expr) => {
        {
            if $loc.have == 0 {
                debug!("PULLBYTE: bailing because we have no input data");
                return inf_leave($loc);
            }
            $loc.have -= 1;
            let b = $loc.input_buffer[$loc.next];
            $loc.hold += b as u32 << $loc.bits;
            $loc.next += 1;
            $loc.bits += 8;
            // debug!("PULLBYTE: 0x{:2x} {:3}, have: {} hold: {:8x} next: {} bits: {}", b, b, $loc.have, $loc.hold, $loc.next, $loc.bits);
        }
    }
}

/* Assure that there are at least n bits in the bit accumulator.  If there is
   not enough available input to do that, then return from inflate(). */
macro_rules! NEEDBITS {
    ($loc:expr, $n:expr) => {
        {
            let n :uint = $n;
            while ($loc.bits as uint) < n {
                PULLBYTE!($loc);
            }
            // debug!("NEEDBITS: got {} bits", n);
        }
    }
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
    // debug!("dropbits: dropped {} bit(s), have {} left, hold = {:8x}", n, loc.bits, loc.hold);
}

/* Remove zero to seven bits as needed to go to a byte boundary */
fn bytebits(loc: &mut InflateLocals) {
    if (loc.bits & 7) != 0 {
        debug!("dropping {} bits to align to byte boundary", loc.bits & 7);
    }
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

    have :uint,         // available input
    left :uint,         // available output
    hold :u32,          // bit buffer
    bits :uint,         // bits in bit buffer

    next: uint,    // z_const unsigned char FAR *next;    /* next input */ // index into input_buffer
    put: uint,  // unsigned char FAR *put;     /* next output */ // index into output_buffer

    in_ :uint,           // save starting available input
    out :uint,          // save starting available output

    flush: u32
}

// The 'a lifetime allows inflate() to use input/output streams,
// whose lifetime is constrained to be less than that of strm/state.
pub fn inflate<'a>(
    strm: &mut ZStream,
    state: &mut InflateState,
    flush: u32,
    input_buffer: &'a [u8],
    output_buffer: &'a mut[u8]) -> InflateResult
{
    let mut locs = InflateLocals {
        strm: strm,
        state: state,
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
        flush: flush
    };
    let loc = &mut locs;

    let mut copy: uint;         // number of stored or match bytes to copy
    // unsigned char FAR *from;    // where to copy match bytes from
    let mut here: Code;         // current decoding table entry
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
        debug!("inflate: mode = {}", loc.state.mode);
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
            debug!("max distance (dmax) = {} 0x{:x}", loc.state.dmax, loc.state.dmax);
            debug!("inflate:   zlib header ok");
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
            debug!("FLAGS: flags: 0x{:8x} is_text: {}", flags, is_text);

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
                debug!("FLAGS contains time");
                loc.state.check = crc2(loc.state.check, loc.hold);
            }
            else {
                debug!("FLAGS did not have a time");
            }
            initbits(loc);
            loc.state.mode = InflateMode::TIME;
        }

        InflateMode::TIME => {
            NEEDBITS!(loc, 32);
            let time :u32 = loc.state.hold;
            debug!("TIME: t: {}", time);

            /*if (state.head != Z_NULL)
                state.head.time = time;
            */

            if (loc.state.flags & 0x0200) != 0 {
                loc.state.check = crc4(loc.state.check, time);
            }
            initbits(loc);
            loc.state.mode = InflateMode::OS;
        }

        InflateMode::OS => {
            NEEDBITS!(loc, 16);
            let ostype = loc.state.hold;
            let xflags = ostype & 0xff;
            let os = ostype >> 8;
            debug!("OS: os 0x{:x} xflags 0x{:x}", os, xflags);
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
            loc.state.mode = InflateMode::EXLEN;
        }

        InflateMode::EXLEN => {
            if (loc.state.flags & 0x0400) != 0 {
                NEEDBITS!(loc, 16);
                let extra_len = loc.state.hold & 0xffff;
                loc.state.length = extra_len as uint;

                debug!("EXTRALEN: extra_len = {}", extra_len);

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
                debug!("no EXTRALEN");
                match loc.state.head {
                    Some(ref mut h) => {
                        // h.extra = Z_NULL;
                        h.extra_len = 0;
                    }
                    None => ()
                }
            }
            loc.state.mode = InflateMode::EXTRA;
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
                        loc.state.check = crc32(loc.state.check, subslice(loc.input_buffer, loc.next, copy));
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
                debug!("no EXTRA");
            }
            loc.state.length = 0;
            loc.state.mode = InflateMode::NAME;
        }

        InflateMode::NAME => {
            if (loc.state.flags & 0x0800) != 0 {
                debug!("NAME: header flags indicate that stream contains a NAME record");
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
                    loc.state.check = crc32(loc.state.check, subslice(loc.input_buffer, loc.next, copy));
                }
                loc.have -= copy;
                loc.next += copy;
                if len != 0 { return inf_leave(loc); }
            }
            else
            {
                debug!("NAME: header does not contain a NAME record");
                /*TODO if (state.head != Z_NULL) {
                    state.head.name = Z_NULL;
                }*/
            }
            loc.state.length = 0;
            loc.state.mode = InflateMode::COMMENT;
        }

        InflateMode::COMMENT => {
            if (loc.state.flags & 0x1000) != 0 {
                debug!("COMMENT: header contains a COMMENT record");
                if loc.have == 0 {
                    debug!("have no data, returning");
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
                    loc.state.check = crc32(loc.state.check, subslice(input_buffer, loc.next, copy));
                }
                loc.have -= copy;
                loc.next += copy;
                if len != 0 {
                    return inf_leave(loc);
                }
            }
            else {
                debug!("COMMENT: header does not contain a COMMENT record");
                // TODO
                // if (state.head != Z_NULL)
                //     state.head.comment = Z_NULL;
            }
            loc.state.mode = InflateMode::HCRC;
        }

        InflateMode::HCRC => {
            if (loc.state.flags & 0x0200) != 0 {
                NEEDBITS!(loc, 16);
                let expected_crc = loc.hold;
                debug!("HCRC: header says expected CRC = 0x{:x}", expected_crc);
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
            debug!("check = 0x{:x}", check);
            loc.strm.adler = check;
            loc.state.check = check;
            initbits(loc);
            loc.state.mode = InflateMode::DICT;
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
            loc.state.mode = InflateMode::TYPE;
        }

        InflateMode::TYPE => {
            if flush == Z_BLOCK || flush == Z_TREES {
                debug!("TYPE: flush is Z_BLOCK or Z_TREES, returning");
                return inf_leave(loc);
            }
            loc.state.mode = InflateMode::TYPEDO;
        }

        InflateMode::TYPEDO => {
            if loc.state.last {
                debug!("TYPEDO: is last block, --> CHECK");
                bytebits(loc);
                loc.state.mode = InflateMode::CHECK;
            }
            else {
                NEEDBITS!(loc, 3);
                loc.state.last = bitbool(loc);
                dropbits(loc, 1);
                debug!("TYPEDO: last = {}, kind = {}", loc.state.last, bits(loc, 2));
                match bits(loc, 2) {
                    0 => {
                        /* stored block */
                        if loc.state.last {
                            debug!("inflate:     stored block (last)");
                        }
                        else {
                            debug!("inflate:     stored block");
                        }
                        loc.state.mode = InflateMode::STORED;
                    }

                    1 => { /* fixed block */
                        unimplemented!(); /*
                        fixedtables(loc, state);
                        if loc.state.last {
                            debug!("inflate:     fixed codes block (last)");
                        }
                        else {
                            debug!("inflate:      fixed codes block");
                        }
                        state.mode = LEN_;             /* decode codes */
                        if (flush == Z_TREES) {
                            dropbits(loc, 2);
                            return inf_leave(loc);
                        }*/
                    }

                    2 => { /* dynamic block */
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
            if flush == Z_TREES {
                debug!("flush = Z_TREES, so returning");
                return inf_leave(loc);
            }
        }
        InflateMode::COPY_ => {
            loc.state.mode = InflateMode::COPY;
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
                copy_memory(loc.output_buffer.slice_mut(loc.put, copy), subslice(loc.input_buffer, loc.next, copy));
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
            loc.state.nlen = bits(loc, 5) as uint + 257;
            dropbits(loc, 5);
            loc.state.ndist = bits(loc, 5) as uint + 1;
            dropbits(loc, 5);
            loc.state.ncode = bits(loc, 4) as uint + 4;
            dropbits(loc, 4);
            debug!("TABLE: nlen {} ndist {} ncode {}", loc.state.nlen, loc.state.ndist, loc.state.ncode);
// #ifndef PKZIP_BUG_WORKAROUND
            if loc.state.nlen > 286 || loc.state.ndist > 30 {
                BADINPUT!(loc, "too many length or distance symbols");
            }
// #endif
            loc.state.have = 0;
            loc.state.mode = InflateMode::LENLENS;
        }

        InflateMode::LENLENS => {
            debug!("have = {}, ncode = {}, reading {} lengths", loc.state.have, loc.state.ncode, loc.state.ncode - loc.state.have);
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
            debug!("inflating code lengths");
            loc.state.next = 0; // loc.state.codes;
            loc.state.lencode = loc.state.next; // TODO: deal with pointers
            loc.state.lenbits = 7;
            let (inflate_ret, inflate_bits) = inflate_table(CODES, &loc.state.lens, 19, &mut loc.state.codes, &mut loc.state.next,
                loc.state.lenbits, loc.state.work.as_mut_slice());
            ret = inflate_ret as uint;
            loc.state.lenbits = inflate_bits;
            if ret != 0 {
                BADINPUT!(loc, "invalid code lengths set");
            }
            debug!("code lengths are ok");
            loc.state.have = 0;
            loc.state.mode = InflateMode::CODELENS;
        }
        InflateMode::CODELENS => {
            while loc.state.have < loc.state.nlen + loc.state.ndist {
                loop {
                    here = loc.state.codes[loc.state.lencode + bits(loc, loc.state.lenbits) as uint];
                    if here.bits as uint <= loc.bits {
                        break;
                    }
                    PULLBYTE!(loc);
                }
                if here.val < 16 {
                    dropbits(loc, here.bits as uint);
                    loc.state.lens[loc.state.have] = here.val;
                    loc.state.have += 1;
                }
                else {
                    let mut copy :uint;
                    let mut len :uint;
                    if here.val == 16 {
                        NEEDBITS!(loc, here.bits as uint + 2);
                        dropbits(loc, here.bits as uint);
                        if loc.state.have == 0 {
                            BADINPUT!(loc, "invalid bit length repeat");
                        }
                        len = loc.state.lens[loc.state.have as uint - 1] as uint;
                        copy = 3 + bits(loc, 2) as uint;
                        dropbits(loc, 2);
                    }
                    else if here.val == 17 {
                        NEEDBITS!(loc, here.bits as uint + 3);
                        dropbits(loc, here.bits as uint);
                        len = 0;
                        copy = 3 + bits(loc, 3) as uint;
                        dropbits(loc, 3);
                    }
                    else {
                        NEEDBITS!(loc, here.bits as uint + 7);
                        dropbits(loc, here.bits as uint);
                        len = 0;
                        copy = 11 + bits(loc, 7) as uint;
                        dropbits(loc, 7);
                    }
                    if loc.state.have + copy > loc.state.nlen + loc.state.ndist {
                        BADINPUT!(loc, "invalid bit length repeat");
                    }
                    while copy != 0 {
                        copy -= 1;
                        loc.state.lens[loc.state.have] = len as u16;
                        loc.state.have += 1;
                    }
                }
            }

            /* handle error breaks in while */
            if loc.state.mode as u32 == InflateMode::BAD as u32 {
                continue;
            }

            /* check for end-of-block code (better have one) */
            if loc.state.lens[256] == 0 {
                BADINPUT!(loc, "invalid code -- missing end-of-block");
            }

            /* build code tables -- note: do not change the lenbits or distbits
               values here (9 and 6) without reading the comments in inftrees.h
               concerning the ENOUGH constants, which depend on those values */
            loc.state.next = 0; // loc.state.codes;
            loc.state.lencode = 0; // (const code FAR *)(state.next);
            loc.state.lenbits = 9;
            debug!("calling inflate_table for lengths");
            let (inflate_result, inflate_bits) = inflate_table(
                LENS, loc.state.lens.as_slice(), loc.state.nlen, &mut loc.state.codes, &mut loc.state.next,
                                loc.state.lenbits, loc.state.work.as_mut_slice());
            ret = inflate_result as uint;
            loc.state.lenbits = inflate_bits;
            if ret != 0 {
                BADINPUT!(loc, "invalid literal/lengths set");
            }
            loc.state.distcode = loc.state.next; // (const code FAR *)(state.next);
            loc.state.distbits = 6;
            debug!("calling inflate_table for codes");
            debug!("loc.state.lens = {}, loc.state.nlen = {}, loc.state.ndist = {}", loc.state.lens.len(), loc.state.nlen, loc.state.ndist);

            let (inflate_ret, inflate_bits) = {
                let codes_lens = subslice(loc.state.lens.as_slice(), loc.state.nlen, loc.state.ndist);
                debug!("codes_lens.len = {}", codes_lens);
                inflate_table(DISTS, codes_lens, loc.state.ndist,
                            &mut loc.state.codes, &mut loc.state.next, loc.state.distbits, loc.state.work.as_mut_slice()) };
            if inflate_ret != 0 {
                BADINPUT!(loc, "invalid distances set");
            }
            loc.state.distbits = inflate_bits;
            debug!("codes are ok");
            loc.state.mode = InflateMode::LEN_;
            if flush == Z_TREES {
                debug!("flush = Z_TREES, returning");
                return inf_leave(loc);
            }
        }
        InflateMode::LEN_ => {
            loc.state.mode = InflateMode::LEN;
        }
        InflateMode::LEN => {
            if loc.have >= 6 && loc.left >= 258 {
                restore_locals(loc);
                inflate_fast(loc.state, loc.strm, loc.input_buffer, loc.output_buffer, loc.out);
                load_locals(loc);
                if loc.state.mode as u32 == InflateMode::TYPE as u32 {
                    loc.state.back = -1;
                }
            }
            else {
                loc.state.back = 0;
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
                        Tracevv!("inflate:         literal '{}'", here.val as u8 as char);
                    }
                    else {
                        debug!("inflate:         literal 0x{:02x}", here.val);
                        Tracevv!("inflate:         literal 0x{:02x}", here.val);
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
                loc.state.mode = InflateMode::LENEXT;
            }
        }

        InflateMode::LENEXT => {
            if loc.state.extra != 0 {
                NEEDBITS!(loc, loc.state.extra);
                loc.state.length += bits(loc, loc.state.extra as uint) as uint;
                let extra = loc.state.extra;
                dropbits(loc, extra);
                loc.state.back += loc.state.extra;
            }
            debug!("inflate:         length {}", loc.state.length);
            Tracevv!("inflate:         length {}", loc.state.length);
            loc.state.was = loc.state.length;
            loc.state.mode = InflateMode::DIST;
        }

        InflateMode::DIST => {
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
            loc.state.mode = InflateMode::DISTEXT;
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
            Tracevv!("inflate:         distance {}", loc.state.offset);
            loc.state.mode = InflateMode::MATCH;
        }

        InflateMode::MATCH => {
            let mut from :BufPos; // index into loc.input_buffer (actually, several different buffers)
            if loc.left == 0 {
                return inf_leave(loc);
            }
            let mut copy = loc.out - loc.left;
            if loc.state.offset > copy {         /* copy from window */
                copy = loc.state.offset - copy;
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
            }
            else {
                /* copy from output */
                from = BufPos { buf: loc.input_buffer, pos: loc.put - loc.state.offset };
                copy = loc.state.length;
            }
            if copy > loc.left {
                copy = loc.left;
            }
            loc.left -= copy;
            loc.state.length -= copy;
            loop {
                loc.output_buffer[loc.put] = from.read();
                loc.put += 1;

                copy -= 1;
                if copy == 0 { break; }
            }
            if loc.state.length == 0 {
                loc.state.mode = InflateMode::LEN;
            }
        }

        InflateMode::LIT => {
            if loc.left == 0 {
                return inf_leave(loc);
            }
            loc.output_buffer[loc.put] = loc.state.length as u8;
            loc.put += 1;
            loc.left -= 1;
            loc.state.mode = InflateMode::LEN;
        }

        InflateMode::CHECK => {
            unimplemented!(); /*
            let mut from: uint; // index into loc.input_buffer
            if loc.state.wrap != 0 {
                NEEDBITS!(loc, 32);
                loc.out -= loc.left;
                loc.strm.total_out += loc.out;
                loc.state.total += loc.out;
                if loc.out != 0 {
                    let check = UPDATE(loc.state.check, (loc.put - loc.out) as u32, loc.out);
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
            loc.state.mode = LENGTH;
            */
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
            loc.state.mode = InflateMode::DONE;
        }

        InflateMode::DONE => {
            unimplemented!(); /*
            ret = Z_STREAM_END;
            return inf_leave(loc); */
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

        if loc.state.mode as u32 != oldmode as u32 {
            debug!("mode {} --> {}", oldmode, loc.state.mode);
        }
        // continue processing
    }
}

fn inf_leave(loc: &mut InflateLocals) -> InflateResult
{
    // Return from inflate(), updating the total counts and the check value.
    // If there was no progress during the inflate() call, return a buffer
    // error.  Call updatewindow() to create and/or update the window state.
    // Note: a memory error from inflate() is non-recoverable.

    debug!("inf_leave");
    restore_locals(loc);

    if loc.state.wsize != 0 || (loc.out != loc.strm.avail_out && (loc.state.mode as u32) < (InflateMode::BAD as u32) &&
            ((loc.state.mode as u32) < (InflateMode::CHECK as u32) || (loc.flush != Z_FINISH))) {

        debug!("strm.next_out = {}, strm.avail_out = {}, out = {}", loc.strm.next_out, loc.strm.avail_out, loc.out);
        assert!(loc.out >= loc.strm.avail_out);
        let e = loc.strm.next_out;
        let c = loc.out - loc.strm.avail_out;
        updatewindow(loc, e, c);
    }
    loc.in_ -= loc.strm.avail_in;
    loc.out -= loc.strm.avail_out;
    loc.strm.total_in += loc.in_;
    loc.strm.total_out += loc.out;
    loc.state.total += loc.out;
    if loc.state.wrap != 0 && loc.out != 0 {
        let updated_check = update(loc.state.flags, loc.state.check, loc.output_buffer.slice(loc.strm.next_out - loc.out, loc.strm.next_out));
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
//    return ret;
    InflateResult::Decoded(0, 0) // probably not right
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

fn subslice<T>(s: &[T], start: uint, len: uint) -> &[T]
{
    s.slice(start, start + len)
}

fn subslice_mut<T>(s: &mut [T], start: uint, len: uint) -> &mut [T]
{
    s.slice_mut(start, start + len)
}

