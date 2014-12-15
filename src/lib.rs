#![feature(globs)]
#![feature(macro_rules)]
#![allow(dead_code)]
#![allow(unused_variables)]

#![feature(phase)]
#[phase(plugin, link)] extern crate log;

extern crate crc32;

use treedefs::*;
use statictrees::*;

pub macro_rules! Tracevv {
    ($($arg:tt)*) => {
        if cfg!(not(ndebug)) {
            println!($($arg)*)
        }
    }
}

mod adler32;
pub mod inflate;
mod inffast;
mod inftrees;
mod statictrees;
mod treedefs;

// From deflate.h

// ===========================================================================
// Internal compression state.


/// Maximum heap size
pub const HEAP_SIZE :uint = 2 * L_CODES + 1;

/// Size of bit buffer in bi_buf
pub const BUF_SIZE :uint = 16;

pub const INIT_STATE    :uint = 42;
pub const EXTRA_STATE   :uint = 69;
pub const NAME_STATE    :uint = 73;
pub const COMMENT_STATE :uint = 91;
pub const HCRC_STATE    :uint = 103;
pub const BUSY_STATE    :uint = 113;
pub const FINISH_STATE  :uint = 666;
/* Stream status */


/* A Pos is an index in the character window. We use short instead of int to
 * save space in the various tables. IPos is used only for parameter passing.
 */
pub type Pos = u16;
pub type Posf = u16;    // replace with Pos
pub type IPos = u32;

pub struct InternalState {
    status :uint,        /* as the name implies */
    pending_buf :Vec<u8>,  /* output still pending */
    pending_buf_size: uint, /* size of pending_buf */    // use pending_buf.len()
    // u8 *pending_out,  /* next pending byte to output to the stream */
    pending: uint,                  // nb of bytes in the pending buffer
    wrap: uint,                     // bit 0 true for zlib, bit 1 true for gzip
    gzhead: Option<GZipHeader>,     // gzip header information to write
    gzindex: uint,                  // where in extra, name, or comment
    method: u8,                     // can only be DEFLATED
    last_flush :int,                // value of flush param for previous deflate call

    /* used by deflate.c: */
    w_size :uint,        /* LZ77 window size (32K by default) */
    w_bits :uint,        /* log2(w_size)  (8..16) */
    w_mask :uint,        /* w_size - 1 */

    // Sliding window. Input bytes are read into the second half of the window,
    // and move to the first half later to keep a dictionary of at least wSize
    // bytes. With this organization, matches are limited to a distance of
    // wSize-MAX_MATCH bytes, but this ensures that IO is always
    // performed with a length multiple of the block size. Also, it limits
    // the window size to 64K, which is quite useful on MSDOS.
    // To do: use the user input buffer as sliding window.
    window :Vec<u8>,

    // Actual size of window: 2*wSize, except when the user input buffer
    // is directly used as sliding window.
    window_size :uint,

    // Link to older string with same hash index. To limit the size of this
    // array to 64K, this link is maintained only for the last 32K strings.
    // An index in this array is thus a window index modulo 32K.
    prev: Vec<Pos>,

    head: Vec<Pos>, // Heads of the hash chains or NIL.

    ins_h :uint,          // hash index of string to be inserted
    hash_size :uint,      // number of elements in hash table
    hash_bits :uint,      // log2(hash_size)
    hash_mask :uint,      // hash_size-1

    // Number of bits by which ins_h must be shifted at each input
    // step. It must be such that after MIN_MATCH steps, the oldest
    // byte no longer takes part in the hash key, that is:
    //   hash_shift * MIN_MATCH >= hash_bits
    hash_shift :uint,

    // Window position at the beginning of the current output block. Gets
    // negative when the window is moved backwards.
    block_start :int,

    match_length :uint,           /* length of best match */
    prev_match: IPos,             /* previous match */
    match_available: int,         /* set if previous match exists */
    strstart :uint,               /* start of string to insert */
    match_start :uint,            /* start of matching string */
    lookahead :uint,              /* number of valid bytes ahead in window */

    // Length of the best match at previous step. Matches not greater than this
    // are discarded. This is used in the lazy match evaluation.
    prev_length :uint,

    // To speed up deflation, hash chains are never searched beyond this
    // length.  A higher limit improves compression ratio but degrades the speed.
    max_chain_length :uint,

    // Attempt to find a better match only when the current match is strictly
    // smaller than this value. This mechanism is used only for compression
    // levels >= 4.
    max_lazy_match :uint,

// #   define max_insert_length  max_lazy_match
    /* Insert new strings in the hash table only if the match length is not
     * greater than this length. This saves time but degrades compression.
     * max_insert_length is used only for compression levels <= 3.
     */

    level: uint,    /* compression level (1..9) */
    strategy: uint, /* favor or force Huffman coding*/

    good_match: uint,
    /* Use a faster search when the previous match is longer than this */

    nice_match: int, /* Stop searching when current match exceeds this */

                /* used by trees.c: */
    /* Didn't use ct_data typedef below to suppress compiler warning */
    dyn_ltree_fc :[u16, ..HEAP_SIZE],   /* literal and length tree */
    dyn_ltree_dl :[u16, ..HEAP_SIZE],

    dyn_dtree_fc :[u16, ..2*D_CODES+1], /* distance tree */
    dyn_dtree_dl :[u16, ..2*D_CODES+1], /* distance tree */

    bl_tree_fc :[u16, ..2*BL_CODES+1],  /* Huffman tree for bit lengths */
    bl_tree_dl :[u16, ..2*BL_CODES+1],  /* Huffman tree for bit lengths */

    l_desc :TreeDesc,               /* desc. for literal tree */
    d_desc :TreeDesc,               /* desc. for distance tree */
    bl_desc :TreeDesc,              /* desc. for bit length tree */

    bl_count :[u16, ..MAX_BITS+1],
    /* number of codes at each bit length for an optimal tree */

    heap :[int, ..2*L_CODES+1],      /* heap used to build the Huffman trees */
    heap_len: uint,               /* number of elements in the heap */
    heap_max: uint,               /* element of largest frequency */
    /* The sons of heap[n] are heap[2*n] and heap[2*n+1]. heap[0] is not used.
     * The same heap array is used to build all trees.
     */

    depth: [u8, ..2*L_CODES+1],
    /* Depth of each subtree used as tie breaker for trees of equal frequency
     */

    l_buf :Vec<u8>,          /* buffer for literals or lengths */

    lit_bufsize :uint,
    /* Size of match buffer for literals/lengths.  There are 4 reasons for
     * limiting lit_bufsize to 64K:
     *   - frequencies can be kept in 16 bit counters
     *   - if compression is not successful for the first block, all input
     *     data is still in the window so we can still emit a stored block even
     *     when input comes from standard input.  (This can also be done for
     *     all blocks if lit_bufsize is not greater than 32K.)
     *   - if compression is not successful for a file smaller than 64K, we can
     *     even emit a stored file instead of a stored block (saving 5 bytes).
     *     This is applicable only for zip (not gzip or zlib).
     *   - creating new Huffman trees less frequently may not provide fast
     *     adaptation to changes in the input data statistics. (Take for
     *     example a binary file with poorly compressible code followed by
     *     a highly compressible string table.) Smaller buffer sizes give
     *     fast adaptation but have of course the overhead of transmitting
     *     trees more frequently.
     *   - I can't count above 4
     */

    last_lit: uint,      /* running index in l_buf */

    d_buf: Vec<u16>,
    /* Buffer for distances. To simplify the code, d_buf and l_buf have
     * the same number of elements. To use different lengths, an extra flag
     * array would be necessary.
     */

    opt_len: uint,        /* bit length of current block with optimal trees */
    static_len: uint,     /* bit length of current block with static trees */
    matches: uint,       /* number of string matches in current block */
    insert: uint,        /* bytes at end of window left to insert */

// #ifdef DEBUG
    compressed_len: uint, /* total bit length of compressed file mod 2^32 */
    bits_sent: uint,      /* bit length of compressed data sent mod 2^32 */
// #endif

    bi_buf: u16,
    /* Output buffer. bits are inserted starting at the bottom (least
     * significant bits).
     */
    bi_valid: uint,
    /* Number of valid bits in bi_buf.  All bits above the last valid bit
     * are always zero.
     */

    high_water: uint,
    /* High water mark offset in window for initialized bytes -- bytes above
     * this are set to zero in order to avoid memory check warnings when
     * longest match routines access bytes past the input.  This is then
     * updated to the new high water mark.
     */
}

pub type DeflateState = InternalState;

/* Output a byte on the stream.
 * IN assertion: there is enough room in pending_buf.
 */
pub fn put_byte(s :&mut DeflateState, c: u8)
{
    s.pending_buf[s.pending] = c;
    s.pending += 1;
}



/// Minimum amount of lookahead, except at the end of the input file.
/// See deflate.c for comments about the MIN_MATCH+1.
pub const MIN_LOOKAHEAD :uint = (MAX_MATCH+MIN_MATCH+1);

// was MAX_DIST
pub fn max_dist(s :&mut DeflateState) -> uint
{
    s.w_size - MIN_LOOKAHEAD
}

/* In order to simplify the code, particularly on 16 bit machines, match
 * distances are limited to MAX_DIST instead of WSIZE.
 */

/// Number of bytes after end of data in window to initialize in order to avoid
/// memory checker errors from longest match routines */
pub const WIN_INIT :uint = MAX_MATCH;

pub fn d_code(dist :u16) -> u16
{
    if dist < 256 {
        DIST_CODE[dist as uint] as u16
    }
    else {
        DIST_CODE[256 + (dist as uint >> 7)] as u16
    }
}

/*
static _length_code :[u8, ..];
static _dist_code :[u8, ..];
*/

pub fn _tr_tally_lit(s :&mut DeflateState, c :u8)
    -> bool     // returns 'flush' value
{
    let cc :u8 = c;
    s.d_buf[s.last_lit] = 0;
    s.l_buf[s.last_lit] = cc;
    s.last_lit += 1;
    s.dyn_ltree_fc /* freq */[cc as uint] += 1;
    s.last_lit == s.lit_bufsize - 1
}

pub fn _tr_tally_dist(s :&mut DeflateState, distance :u16, length :u8)
    -> bool     // returns 'flush' value
{
    let len = length;
    let mut dist = distance;
    s.d_buf[s.last_lit as uint] = dist;
    s.l_buf[s.last_lit as uint] = len;
    s.last_lit += 1;
    dist -= 1;
    s.dyn_ltree_fc/*freq*/[LENGTH_CODE[len as uint] as uint + LITERALS + 1] += 1;
    s.dyn_dtree_fc/*freq*/[d_code(dist) as uint] += 1;
    s.last_lit == s.lit_bufsize - 1
}

// The three kinds of block type
const STORED_BLOCK: uint = 0;
const STATIC_TREES: uint = 1;
const DYN_TREES: uint = 2;

pub const PRESET_DICT: uint = 0x20; /* preset dictionary flag in zlib header */

pub struct ZStream {
    pub next_in: uint,          // index of next input byte, within input_buffer (passed elsewhere)
    pub avail_in: uint,         // number of bytes available at next_in
    pub total_in: uint,         // total number of input bytes read so far
    pub next_out: uint,         // position within output_buffer where to write the next byte
    pub avail_out: uint,        // remaining free space at next_out
    pub total_out: uint,        // total number of bytes output so far
    pub msg: Option<String>,    // last error message, if any
    pub data_type :uint,        // best guess about the data type: binary or text
    pub adler: u32              // adler32 value of the uncompressed data
}

/// gzip header information passed to and from zlib routines.  See RFC 1952
/// for more details on the meanings of these fields.
pub struct GZipHeader {
    pub text: bool,                     // true if compressed data believed to be text
    pub time: u32,                      // modification time
    pub xflags: u32,                    // extra flags (not used when writing a gzip file)
    pub os: u32,                        // operating system
    pub extra_len: uint,                // length of the 'extra' data, in bytes
    pub extra: Option<Box<Vec<u8>>>,    // extra field data, if any
//    pub name_len: uint,                 // length of the 'name' data, in bytes (not chars!)
    pub name: Option<Box<String>>,      // filename, if any
//    pub comm_len: uint,                 // length of the 'comment' data, in bytes (not chars!)
    pub comment: Option<Box<String>>,   // comment string, if any
    pub hcrc: bool,                     // true if there was or will be a header crc
    pub done: bool,                     // true when done reading gzip header (not used when writing a gzip file)
}

impl GZipHeader {
    fn new() -> GZipHeader {
        GZipHeader {
            text: false,
            time: 0,
            xflags: 0,
            os: 0,
            extra: None,
            extra_len: 0,
            name: None,
            comment: None,
            hcrc: false,
            done: false
        }
    }
}

/*
     The application must update next_in and avail_in when avail_in has dropped
   to zero.  It must update next_out and avail_out when avail_out has dropped
   to zero.  All other fields are set by the compression
   library and must not be updated by the application.

     The fields total_in and total_out can be used for statistics or progress
   reports.  After compression, total_in holds the total size of the
   uncompressed data and may be saved for use in the decompressor (particularly
   if the decompressor wants to decompress everything in a single step).
*/

/* constants */

/* Allowed flush values; see deflate() and inflate() below for details */
pub const Z_NO_FLUSH     : u32 = 0;
pub const Z_PARTIAL_FLUSH: u32 = 1;
pub const Z_SYNC_FLUSH   : u32 = 2;
pub const Z_FULL_FLUSH   : u32 = 3;
pub const Z_FINISH       : u32 = 4;
pub const Z_BLOCK        : u32 = 5;
pub const Z_TREES        : u32 = 6;

#[deriving(Copy)]
pub enum ZERR {
    Ok              = 0,        // Z_OK            = 0,
    StreamEnd       = 1,        // Z_STREAM_END    = 1,
    NeedDict        = 2,        // Z_NEED_DICT     = 2,
    Errno           = -1,        // Z_ERRNO         = -1,
    StreamError     = -2,        // Z_STREAM_ERROR  = -2,
    DataError       = -3,        // Z_DATA_ERROR    = -3,
    MemError        = -4,        // Z_MEM_ERROR     = -4,
    BufError        = -5,        // Z_BUF_ERROR     = -5,
    VersionError    = -6,        // Z_VERSION_ERROR = -6,
    // Return codes for the compression/decompression functions. Negative values
    // are errors, positive values are used for special but normal events.
}

/* compression levels */
pub const Z_NO_COMPRESSION     : int = 0;
pub const Z_BEST_SPEED         : int = 1;
pub const Z_BEST_COMPRESSION   : int = 9;
pub const Z_DEFAULT_COMPRESSION: int = -1;

pub const Z_FILTERED            :uint = 1;
pub const Z_HUFFMAN_ONLY        :uint = 2;
pub const Z_RLE                 :uint = 3;
pub const Z_FIXED               :uint = 4;
pub const Z_DEFAULT_STRATEGY    :uint = 0;
/* compression strategy; see deflateInit2() below for details */

pub const Z_BINARY   :uint = 0;
pub const Z_TEXT     :uint = 1;
pub const Z_ASCII    :uint = Z_TEXT;   /* for compatibility with 1.2.2 and earlier */
pub const Z_UNKNOWN  :uint = 2;
/* Possible values of the data_type field (though see inflate()) */

pub const Z_DEFLATED :uint = 8;
/* The deflate compression method (the only one supported in this version) */

impl InternalState
{
    fn new() -> InternalState
    {
        let mut s = InternalState {
            status: 0,
            pending_buf: Vec::new(),
            pending_buf_size: 0,
            pending: 0,
            wrap: 0,
            gzhead: None,
            gzindex: 0,
            method: 0,
            last_flush: 0,
            w_size: 0,
            w_bits: 0,
            w_mask: 0,
            window: Vec::new(),
            window_size: 0,
            prev: Vec::new(),
            head: Vec::new(),
            ins_h: 0,
            hash_size: 0,
            hash_bits: 0,
            hash_mask: 0,
            hash_shift: 0,
            block_start: 0,
            match_length: 0,
            prev_match: 0,
            match_available: 0,
            strstart: 0,
            match_start: 0,
            lookahead: 0,
            prev_length: 0,
            max_chain_length: 0,
            max_lazy_match: 0,
        // #   define max_insert_length  max_lazy_match
            level: 0,
            strategy: 0,
            good_match: 0,
            nice_match: 0,
            dyn_ltree_fc : [0, ..HEAP_SIZE],   /* literal and length tree */
            dyn_ltree_dl : [0, ..HEAP_SIZE],   /* literal and length tree */
            dyn_dtree_fc : [0, ..2*D_CODES+1], /* distance tree */
            dyn_dtree_dl : [0, ..2*D_CODES+1], /* distance tree */
            bl_tree_fc   : [0, ..2*BL_CODES+1],  /* Huffman tree for bit lengths */
            bl_tree_dl   : [0, ..2*BL_CODES+1],  /* Huffman tree for bit lengths */

            l_desc: TreeDesc::new(StaticTreeDesc {
                lengths: &STATIC_LTREE_LENGTHS,
                codes: &STATIC_LTREE_CODES,
                extra_bits: &EXTRA_LBITS,
                extra_base: LITERALS+1,
                elems: L_CODES,
                max_length: MAX_BITS
            }),

            d_desc: TreeDesc::new(StaticTreeDesc {
                lengths: &STATIC_DTREE_LENGTHS,
                codes: &STATIC_DTREE_CODES,
                extra_bits: &EXTRA_DBITS,
                extra_base: 0,
                elems: D_CODES,
                max_length: MAX_BITS
            }),

            bl_desc: TreeDesc::new(StaticTreeDesc {
                lengths: &[],
                codes: &[],
                extra_bits: &EXTRA_BLBITS,
                extra_base: 0,
                elems: BL_CODES,
                max_length: MAX_BL_BITS
            }),

            bl_count: [0u16, ..MAX_BITS+1],

            heap: [0, ..2*L_CODES+1],
            heap_len: 0,
            heap_max: 0,

            depth: [0u8, ..2*L_CODES+1],

            l_buf: Vec::new(),

            lit_bufsize: 0,
            last_lit: 0,

            d_buf: Vec::new(),

            opt_len: 0,
            static_len: 0,
            matches: 0,
            insert: 0,

        // #ifdef DEBUG
            compressed_len: 0,
            bits_sent: 0,
        // #endif

            bi_buf: 0,
            bi_valid: 0,

            high_water: 0,
        };

        init_block(&mut s);

        return s;
    }
}

impl ZStream
{
    pub fn new() -> ZStream
    {
        ZStream {
            next_in: 0,
            avail_in: 0,
            total_in: 0,
            next_out: 0,
            avail_out: 0,
            total_out: 0,
            msg: None,
            data_type: 0,  /* best guess about the data type: binary or text */
            adler: 0,      /* adler32 value of the uncompressed data */
        }
    }
}

/* trees.c -- output deflated data using Huffman coding
 * Copyright (C) 1995-2012 Jean-loup Gailly
 * detect_data_type() function provided freely by Cosmin Truta, 2006
 * For conditions of distribution and use, see copyright notice in zlib.h
 */
/*
 *  ALGORITHM
 *
 *      The "deflation" process uses several Huffman trees. The more
 *      common source values are represented by shorter bit sequences.
 *
 *      Each code tree is stored in a compressed form which is itself
 * a Huffman encoding of the lengths of all the code strings (in
 * ascending order by source values).  The actual code strings are
 * reconstructed from the lengths in the inflate process, as described
 * in the deflate specification.
 *
 *  REFERENCES
 *
 *      Deutsch, L.P.,"'Deflate' Compressed Data Format Specification".
 *      Available in ftp.uu.net:/pub/archiving/zip/doc/deflate-1.1.doc
 *
 *      Storer, James A.
 *          Data Compression:  Methods and Theory, pp. 49-50.
 *          Computer Science Press, 1988.  ISBN 0-7167-8156-5.
 *
 *      Sedgewick, R.
 *          Algorithms, p290.
 *          Addison-Wesley, 1983. ISBN 0-201-06672-6.
 */

/* ===========================================================================
 * Constants
 */

const MAX_BL_BITS :uint = 7;
/* Bit length codes must not exceed MAX_BL_BITS bits */

const END_BLOCK :uint = 256;
/* end of block literal code */

const REP_3_6 :uint = 16;
/* repeat previous bit length 3-6 times (2 bits of repeat count) */

const REPZ_3_10 :uint = 17;
/* repeat a zero length 3-10 times  (3 bits of repeat count) */

const REPZ_11_138 :uint = 18;
/* repeat a zero length 11-138 times  (7 bits of repeat count) */

// from trees.c

struct TreeDesc {
    // dyn_tree_freq: Vec<u16>,           /* the dynamic tree */
    // dyn_tree_other: Vec<u8>,
    fc: Vec<u16>,           /* the dynamic tree */
    dl: Vec<u8>,

    max_code: uint,            /* largest code with non zero frequency */
    stat_desc: StaticTreeDesc, /* the corresponding static tree */
}

#[deriving(Copy)]
pub struct TreeRef {
    pub codes: &'static [u16],
    pub lengths: &'static [u8],
}

impl TreeDesc {
    fn new(stat_desc: StaticTreeDesc) -> TreeDesc {
        TreeDesc {
            fc: Vec::new(),
            dl: Vec::new(),
            max_code: 0,
            stat_desc: stat_desc
        }
    }
}

fn send_code(s :&mut DeflateState, c :u8, tree :&TreeDesc)
{
    // if (z_verbose>2) fprintf(stderr,"\ncd %3d ",(c));
    println!("send_code: {:3}", c);

    send_bits(s,
        tree.fc/*codes*/[c as uint] as u32,
        tree.dl/*lengths*/[c as uint] as uint);
}

/* ===========================================================================
 * Output a short LSB first on the stream.
 * IN assertion: there is enough room in pendingBuf.
 */
fn put_short(s :&mut DeflateState, w :u16)
{
    put_byte(s, (w & 0xff) as u8);
    put_byte(s, (w >> 8) as u8);
}


/* ===========================================================================
 * Send a value on a given number of bits.
 * IN assertion: length <= 16 and value fits in length bits.
 */
// #ifdef DEBUG

fn send_bits(s :&mut DeflateState, value: u32, length: uint)
    // DeflateState *s;
    // int value;  /* value to send */
    // int length; /* number of bits */
{
    // Tracevv((stderr," l %2d v %4x ", length, value));
    assert!(length > 0 && length <= 15);
    s.bits_sent += length;

    // If not enough room in bi_buf, use (valid) bits from bi_buf and
    // (16 - bi_valid) bits from value, leaving (width - (16-bi_valid))
    // unused bits in value.
    if s.bi_valid > BUF_SIZE - length {
        let s_bi_buf = s.bi_buf | (value << s.bi_valid) as u16;
        s.bi_buf |= s_bi_buf;
        put_short(s, s_bi_buf);
        s.bi_buf = (value >> (BUF_SIZE - s.bi_valid)) as u16;
        s.bi_valid += length - BUF_SIZE;
    } else {
        s.bi_buf |= (value << s.bi_valid) as u16;
        s.bi_valid += length;
    }
}
/* #else /* !DEBUG */

#define send_bits(s, value, length) \
{ int len = length;\
  if (s.bi_valid > (int)Buf_size - len) {\
    int val = value;\
    s.bi_buf |= (ush)val << s.bi_valid;\
    put_short(s, s.bi_buf);\
    s.bi_buf = (ush)val >> (Buf_size - s.bi_valid);\
    s.bi_valid += len - Buf_size;\
  } else {\
    s.bi_buf |= (ush)(value) << s.bi_valid;\
    s.bi_valid += len;\
  }\
}
#endif /* DEBUG */
*/



/* ===========================================================================
 * Initialize a new block.
 */

fn init_block(s :&mut DeflateState)
{
    /* Initialize the trees. */
    for n in range(0, L_CODES) { s.l_desc.fc[n] = 0; }
    for n in range(0, D_CODES) { s.d_desc.fc[n] = 0; }
    for n in range(0, BL_CODES) { s.bl_desc.fc[n] = 0; }

    s.l_desc.fc[END_BLOCK] = 1;
    s.opt_len = 0;
    s.static_len = 0;
    s.last_lit = 0;
    s.matches = 0;
}

const SMALLEST :uint = 1;
/* Index within the heap array of least frequent node in the Huffman tree */


/* ===========================================================================
 * Remove the smallest element from the heap and recreate the heap with
 * one less element. Updates heap and heap_len.
 */
fn pqremove(s :&mut DeflateState, tree :&TreeDesc) -> int
{
    let top = s.heap[SMALLEST];
    s.heap_len -= 1;
    s.heap[SMALLEST] = s.heap[s.heap_len];
    pqdownheap(s, tree, SMALLEST);
    return top;
}

/* ===========================================================================
 * Compares two subtrees, using the tree depth as tie breaker when
 * the subtrees have equal frequency. This minimizes the worst case length.
 */
fn smaller(tree :&TreeDesc, n :uint, m :uint, depth :&[u8]) -> bool
{
    tree.fc/*freq*/[n] < tree.fc/*freq*/[m] || (tree.fc/*freq*/[n] == tree.fc/*freq*/[m] && depth[n] <= depth[m])
}

/* ===========================================================================
 * Restore the heap property by moving down the tree starting at node k,
 * exchanging a node with the smallest of its two sons if necessary, stopping
 * when the heap property is re-established (each father smaller than its
 * two sons).
 */
fn pqdownheap(
    s: &mut DeflateState,
    tree :&TreeDesc,        /* the tree to restore */
    k: uint)                /* node to move down */
{
    let mut k = k;
    let v = s.heap[k];
    let mut j = k << 1;  /* left son of k */
    while j <= s.heap_len {
        /* Set j to the smallest of the two sons: */
        if j < s.heap_len && smaller(tree, s.heap[j+1] as uint, s.heap[j] as uint, s.depth.as_slice()) {
            j += 1;
        }
        /* Exit if v is smaller than both sons */
        if smaller(tree, v as uint, s.heap[j] as uint, &s.depth) {
            break;
        }

        /* Exchange v with the smallest son */
        s.heap[k] = s.heap[j];
        k = j;

        /* And continue down the tree, setting j to the left son of k */
        j <<= 1;
    }
    s.heap[k] = v;
}

/*

/* ===========================================================================
 * Compute the optimal bit lengths for a tree and update the total bit length
 * for the current block.
 * IN assertion: the fields freq and dad are set, heap[heap_max] and
 *    above are the tree nodes sorted by increasing frequency.
 * OUT assertions: the field len is set to the optimal bit length, the
 *     array bl_count contains the frequencies for each bit length.
 *     The length opt_len is updated; static_len is also updated if stree is
 *     not null.
 */
fn gen_bitlen(s :&mut DeflateState, desc)
     *s;
    TreeDesc *desc;    /* the tree descriptor */
{
    ct_data *tree        = desc.dyn_tree;
    int max_code         = desc.max_code;
    const ct_data *stree = desc.stat_desc.static_tree;
    const intf *extra    = desc.stat_desc.extra_bits;
    int base             = desc.stat_desc.extra_base;
    int max_length       = desc.stat_desc.max_length;
    int h;              /* heap index */
    int n, m;           /* iterate over the tree elements */
    int bits;           /* bit length */
    int xbits;          /* extra bits */
    ush f;              /* frequency */
    int overflow = 0;   /* number of elements with bit length too large */

    for (bits = 0; bits <= MAX_BITS; bits++) s.bl_count[bits] = 0;

    /* In a first pass, compute the optimal bit lengths (which may
     * overflow in the case of the bit length tree).
     */
    tree[s.heap[s.heap_max]].Len = 0; /* root of the heap */

    for (h = s.heap_max+1; h < HEAP_SIZE; h++) {
        n = s.heap[h];
        bits = tree[tree[n].Dad].Len + 1;
        if (bits > max_length) bits = max_length, overflow++;
        tree[n].Len = (ush)bits;
        /* We overwrite tree[n].Dad which is no longer needed */

        if (n > max_code) continue; /* not a leaf node */

        s.bl_count[bits]++;
        xbits = 0;
        if (n >= base) xbits = extra[n-base];
        f = tree[n].Freq;
        s.opt_len += (ulg)f * (bits + xbits);
        if (stree) s.static_len += (ulg)f * (stree[n].Len + xbits);
    }
    if (overflow == 0) return;

    Trace((stderr,"\nbit length overflow\n"));
    /* This happens for example on obj2 and pic of the Calgary corpus */

    /* Find the first bit length which could increase: */
    do {
        bits = max_length-1;
        while (s.bl_count[bits] == 0) bits--;
        s.bl_count[bits]--;      /* move one leaf down the tree */
        s.bl_count[bits+1] += 2; /* move one overflow item as its brother */
        s.bl_count[max_length]--;
        /* The brother of the overflow item also moves one step up,
         * but this does not affect bl_count[max_length]
         */
        overflow -= 2;
    } while (overflow > 0);

    /* Now recompute all bit lengths, scanning in increasing frequency.
     * h is still equal to HEAP_SIZE. (It is simpler to reconstruct all
     * lengths instead of fixing only the wrong ones. This idea is taken
     * from 'ar' written by Haruhiko Okumura.)
     */
    for (bits = max_length; bits != 0; bits--) {
        n = s.bl_count[bits];
        while (n != 0) {
            m = s.heap[--h];
            if (m > max_code) continue;
            if ((unsigned) tree[m].Len != (unsigned) bits) {
                Trace((stderr,"code %d bits %d.%d\n", m, tree[m].Len, bits));
                s.opt_len += ((long)bits - (long)tree[m].Len)
                              *(long)tree[m].Freq;
                tree[m].Len = (ush)bits;
            }
            n--;
        }
    }
}

*/

/*

/* ===========================================================================
 * Construct one Huffman tree and assigns the code bit strings and lengths.
 * Update the total bit length for the current block.
 * IN assertion: the field freq is set for all tree elements.
 * OUT assertions: the fields len and code are set to the optimal bit length
 *     and corresponding code. The length opt_len is updated; static_len is
 *     also updated if stree is not null. The field max_code is set.
 */
local void build_tree(s, desc)
    DeflateState *s;
    TreeDesc *desc; /* the tree descriptor */
{
    ct_data *tree         = desc.dyn_tree;
    const ct_data *stree  = desc.stat_desc.static_tree;
    int elems             = desc.stat_desc.elems;
    int n, m;          /* iterate over heap elements */
    int max_code = -1; /* largest code with non zero frequency */
    int node;          /* new node being created */

    /* Construct the initial heap, with least frequent element in
     * heap[SMALLEST]. The sons of heap[n] are heap[2*n] and heap[2*n+1].
     * heap[0] is not used.
     */
    s.heap_len = 0, s.heap_max = HEAP_SIZE;

    for (n = 0; n < elems; n++) {
        if (tree[n].Freq != 0) {
            s.heap[++(s.heap_len)] = max_code = n;
            s.depth[n] = 0;
        } else {
            tree[n].Len = 0;
        }
    }

    /* The pkzip format requires that at least one distance code exists,
     * and that at least one bit should be sent even if there is only one
     * possible code. So to avoid special checks later on we force at least
     * two codes of non zero frequency.
     */
    while (s.heap_len < 2) {
        node = s.heap[++(s.heap_len)] = (max_code < 2 ? ++max_code : 0);
        tree[node].Freq = 1;
        s.depth[node] = 0;
        s.opt_len--; if (stree) s.static_len -= stree[node].Len;
        /* node is 0 or 1 so it does not have extra bits */
    }
    desc.max_code = max_code;

    /* The elements heap[heap_len/2+1 .. heap_len] are leaves of the tree,
     * establish sub-heaps of increasing lengths:
     */
    for (n = s.heap_len/2; n >= 1; n--) pqdownheap(s, tree, n);

    /* Construct the Huffman tree by repeatedly combining the least two
     * frequent nodes.
     */
    node = elems;              /* next internal node of the tree */
    do {
        pqremove(s, tree, n);  /* n = node of least frequency */
        m = s.heap[SMALLEST]; /* m = node of next least frequency */

        s.heap[--(s.heap_max)] = n; /* keep the nodes sorted by frequency */
        s.heap[--(s.heap_max)] = m;

        /* Create a new node father of n and m */
        tree[node].Freq = tree[n].Freq + tree[m].Freq;
        s.depth[node] = (uch)((s.depth[n] >= s.depth[m] ?
                                s.depth[n] : s.depth[m]) + 1);
        tree[n].Dad = tree[m].Dad = (ush)node;
#ifdef DUMP_BL_TREE
        if (tree == s.bl_tree) {
            fprintf(stderr,"\nnode %d(%d), sons %d(%d) %d(%d)",
                    node, tree[node].Freq, n, tree[n].Freq, m, tree[m].Freq);
        }
#endif
        /* and insert the new node in the heap */
        s.heap[SMALLEST] = node++;
        pqdownheap(s, tree, SMALLEST);

    } while (s.heap_len >= 2);

    s.heap[--(s.heap_max)] = s.heap[SMALLEST];

    /* At this point, the fields freq and dad are set. We can now
     * generate the bit lengths.
     */
    gen_bitlen(s, (tree_desc *)desc);

    /* The field len is now set, we can generate the bit codes */
    gen_codes ((ct_data *)tree, max_code, s.bl_count);
}

/* ===========================================================================
 * Scan a literal or distance tree to determine the frequencies of the codes
 * in the bit length tree.
 */
local void scan_tree (s, tree, max_code)
    DeflateState *s;
    ct_data *tree;   /* the tree to be scanned */
    int max_code;    /* and its largest code of non zero frequency */
{
    int n;                     /* iterates over all tree elements */
    int prevlen = -1;          /* last emitted length */
    int curlen;                /* length of current code */
    int nextlen = tree[0].Len; /* length of next code */
    int count = 0;             /* repeat count of the current code */
    int max_count = 7;         /* max repeat count */
    int min_count = 4;         /* min repeat count */

    if (nextlen == 0) max_count = 138, min_count = 3;
    tree[max_code+1].Len = (ush)0xffff; /* guard */

    for (n = 0; n <= max_code; n++) {
        curlen = nextlen; nextlen = tree[n+1].Len;
        if (++count < max_count && curlen == nextlen) {
            continue;
        } else if (count < min_count) {
            s.bl_tree[curlen].Freq += count;
        } else if (curlen != 0) {
            if (curlen != prevlen) s.bl_tree[curlen].Freq++;
            s.bl_tree[REP_3_6].Freq++;
        } else if (count <= 10) {
            s.bl_tree[REPZ_3_10].Freq++;
        } else {
            s.bl_tree[REPZ_11_138].Freq++;
        }
        count = 0; prevlen = curlen;
        if (nextlen == 0) {
            max_count = 138, min_count = 3;
        } else if (curlen == nextlen) {
            max_count = 6, min_count = 3;
        } else {
            max_count = 7, min_count = 4;
        }
    }
}

/* ===========================================================================
 * Send a literal or distance tree in compressed form, using the codes in
 * bl_tree.
 */
local void send_tree (s, tree, max_code)
    DeflateState *s;
    ct_data *tree; /* the tree to be scanned */
    int max_code;       /* and its largest code of non zero frequency */
{
    int n;                     /* iterates over all tree elements */
    int prevlen = -1;          /* last emitted length */
    int curlen;                /* length of current code */
    int nextlen = tree[0].Len; /* length of next code */
    int count = 0;             /* repeat count of the current code */
    int max_count = 7;         /* max repeat count */
    int min_count = 4;         /* min repeat count */

    /* tree[max_code+1].Len = -1; */  /* guard already set */
    if (nextlen == 0) max_count = 138, min_count = 3;

    for (n = 0; n <= max_code; n++) {
        curlen = nextlen; nextlen = tree[n+1].Len;
        if (++count < max_count && curlen == nextlen) {
            continue;
        } else if (count < min_count) {
            do { send_code(s, curlen, s.bl_tree); } while (--count != 0);

        } else if (curlen != 0) {
            if (curlen != prevlen) {
                send_code(s, curlen, s.bl_tree); count--;
            }
            Assert(count >= 3 && count <= 6, " 3_6?");
            send_code(s, REP_3_6, s.bl_tree); send_bits(s, count-3, 2);

        } else if (count <= 10) {
            send_code(s, REPZ_3_10, s.bl_tree); send_bits(s, count-3, 3);

        } else {
            send_code(s, REPZ_11_138, s.bl_tree); send_bits(s, count-11, 7);
        }
        count = 0; prevlen = curlen;
        if (nextlen == 0) {
            max_count = 138, min_count = 3;
        } else if (curlen == nextlen) {
            max_count = 6, min_count = 3;
        } else {
            max_count = 7, min_count = 4;
        }
    }
}

/* ===========================================================================
 * Construct the Huffman tree for the bit lengths and return the index in
 * bl_order of the last bit length code to send.
 */
local int build_bl_tree(s)
    DeflateState *s;
{
    int max_blindex;  /* index of last bit length code of non zero freq */

    /* Determine the bit length frequencies for literal and distance trees */
    scan_tree(s, (ct_data *)s.dyn_ltree, s.l_desc.max_code);
    scan_tree(s, (ct_data *)s.dyn_dtree, s.d_desc.max_code);

    /* Build the bit length tree: */
    build_tree(s, (tree_desc *)(&(s.bl_desc)));
    /* opt_len now includes the length of the tree representations, except
     * the lengths of the bit lengths codes and the 5+5+4 bits for the counts.
     */

    /* Determine the number of bit length codes to send. The pkzip format
     * requires that at least 4 bit length codes be sent. (appnote.txt says
     * 3 but the actual value used is 4.)
     */
    for (max_blindex = BL_CODES-1; max_blindex >= 3; max_blindex--) {
        if (s.bl_tree[bl_order[max_blindex]].Len != 0) break;
    }
    /* Update opt_len to include the bit length tree and counts */
    s.opt_len += 3*(max_blindex+1) + 5+5+4;
    Tracev((stderr, "\ndyn trees: dyn %ld, stat %ld",
            s.opt_len, s.static_len));

    return max_blindex;
}

/* ===========================================================================
 * Send the header for a block using dynamic Huffman trees: the counts, the
 * lengths of the bit length codes, the literal tree and the distance tree.
 * IN assertion: lcodes >= 257, dcodes >= 1, blcodes >= 4.
 */
local void send_all_trees(s, lcodes, dcodes, blcodes)
    DeflateState *s;
    int lcodes, dcodes, blcodes; /* number of codes for each tree */
{
    int rank;                    /* index in bl_order */

    Assert (lcodes >= 257 && dcodes >= 1 && blcodes >= 4, "not enough codes");
    Assert (lcodes <= L_CODES && dcodes <= D_CODES && blcodes <= BL_CODES,
            "too many codes");
    Tracev((stderr, "\nbl counts: "));
    send_bits(s, lcodes-257, 5); /* not +255 as stated in appnote.txt */
    send_bits(s, dcodes-1,   5);
    send_bits(s, blcodes-4,  4); /* not -3 as stated in appnote.txt */
    for (rank = 0; rank < blcodes; rank++) {
        Tracev((stderr, "\nbl code %2d ", bl_order[rank]));
        send_bits(s, s.bl_tree[bl_order[rank]].Len, 3);
    }
    Tracev((stderr, "\nbl tree: sent %ld", s.bits_sent));

    send_tree(s, (ct_data *)s.dyn_ltree, lcodes-1); /* literal tree */
    Tracev((stderr, "\nlit tree: sent %ld", s.bits_sent));

    send_tree(s, (ct_data *)s.dyn_dtree, dcodes-1); /* distance tree */
    Tracev((stderr, "\ndist tree: sent %ld", s.bits_sent));
}

/* ===========================================================================
 * Send a stored block
 */
void ZLIB_INTERNAL _tr_stored_block(s, buf, stored_len, last)
    DeflateState *s;
    charf *buf;       /* input block */
    ulg stored_len;   /* length of input block */
    int last;         /* one if this is the last block for a file */
{
    send_bits(s, (STORED_BLOCK<<1)+last, 3);    /* send block type */
#ifdef DEBUG
    s.compressed_len = (s.compressed_len + 3 + 7) & (ulg)~7L;
    s.compressed_len += (stored_len + 4) << 3;
#endif
    copy_block(s, buf, (unsigned)stored_len, 1); /* with header */
}

/* ===========================================================================
 * Flush the bits in the bit buffer to pending output (leaves at most 7 bits)
 */
void ZLIB_INTERNAL _tr_flush_bits(s)
    DeflateState *s;
{
    bi_flush(s);
}

/* ===========================================================================
 * Send one empty static block to give enough lookahead for inflate.
 * This takes 10 bits, of which 7 may remain in the bit buffer.
 */
void ZLIB_INTERNAL _tr_align(s)
    DeflateState *s;
{
    send_bits(s, STATIC_TREES<<1, 3);
    send_code(s, END_BLOCK, static_ltree);
#ifdef DEBUG
    s.compressed_len += 10L; /* 3 for block type, 7 for EOB */
#endif
    bi_flush(s);
}

/* ===========================================================================
 * Determine the best encoding for the current block: dynamic trees, static
 * trees or store, and output the encoded block to the zip file.
 */
void ZLIB_INTERNAL _tr_flush_block(s, buf, stored_len, last)
    DeflateState *s;
    charf *buf;       /* input block, or NULL if too old */
    ulg stored_len;   /* length of input block */
    int last;         /* one if this is the last block for a file */
{
    ulg opt_lenb, static_lenb; /* opt_len and static_len in bytes */
    int max_blindex = 0;  /* index of last bit length code of non zero freq */

    /* Build the Huffman trees unless a stored block is forced */
    if (s.level > 0) {

        /* Check if the file is binary or text */
        if (s.strm.data_type == Z_UNKNOWN)
            s.strm.data_type = detect_data_type(s);

        /* Construct the literal and distance trees */
        build_tree(s, (tree_desc *)(&(s.l_desc)));
        Tracev((stderr, "\nlit data: dyn %ld, stat %ld", s.opt_len,
                s.static_len));

        build_tree(s, (tree_desc *)(&(s.d_desc)));
        Tracev((stderr, "\ndist data: dyn %ld, stat %ld", s.opt_len,
                s.static_len));
        /* At this point, opt_len and static_len are the total bit lengths of
         * the compressed block data, excluding the tree representations.
         */

        /* Build the bit length tree for the above two trees, and get the index
         * in bl_order of the last bit length code to send.
         */
        max_blindex = build_bl_tree(s);

        /* Determine the best encoding. Compute the block lengths in bytes. */
        opt_lenb = (s.opt_len+3+7)>>3;
        static_lenb = (s.static_len+3+7)>>3;

        Tracev((stderr, "\nopt %lu(%lu) stat %lu(%lu) stored %lu lit %u ",
                opt_lenb, s.opt_len, static_lenb, s.static_len, stored_len,
                s.last_lit));

        if (static_lenb <= opt_lenb) opt_lenb = static_lenb;

    } else {
        Assert(buf != (char*)0, "lost buf");
        opt_lenb = static_lenb = stored_len + 5; /* force a stored block */
    }

#ifdef FORCE_STORED
    if (buf != (char*)0) { /* force stored block */
#else
    if (stored_len+4 <= opt_lenb && buf != (char*)0) {
                       /* 4: two words for the lengths */
#endif
        /* The test buf != NULL is only necessary if LIT_BUFSIZE > WSIZE.
         * Otherwise we can't have processed more than WSIZE input bytes since
         * the last block flush, because compression would have been
         * successful. If LIT_BUFSIZE <= WSIZE, it is never too late to
         * transform a block into a stored block.
         */
        _tr_stored_block(s, buf, stored_len, last);

#ifdef FORCE_STATIC
    } else if (static_lenb >= 0) { /* force static trees */
#else
    } else if (s.strategy == Z_FIXED || static_lenb == opt_lenb) {
#endif
        send_bits(s, (STATIC_TREES<<1)+last, 3);
        compress_block(s, (const ct_data *)static_ltree,
                       (const ct_data *)static_dtree);
#ifdef DEBUG
        s.compressed_len += 3 + s.static_len;
#endif
    } else {
        send_bits(s, (DYN_TREES<<1)+last, 3);
        send_all_trees(s, s.l_desc.max_code+1, s.d_desc.max_code+1,
                       max_blindex+1);
        compress_block(s, (const ct_data *)s.dyn_ltree,
                       (const ct_data *)s.dyn_dtree);
#ifdef DEBUG
        s.compressed_len += 3 + s.opt_len;
#endif
    }
    Assert (s.compressed_len == s.bits_sent, "bad compressed size");
    /* The above check is made mod 2^32, for files larger than 512 MB
     * and uLong implemented on 32 bits.
     */
    init_block(s);

    if (last) {
        bi_windup(s);
#ifdef DEBUG
        s.compressed_len += 7;  /* align on byte boundary */
#endif
    }
    Tracev((stderr,"\ncomprlen %lu(%lu) ", s.compressed_len>>3,
           s.compressed_len-7*last));
}

/* ===========================================================================
 * Save the match info and tally the frequency counts. Return true if
 * the current block must be flushed.
 */
int ZLIB_INTERNAL _tr_tally (s, dist, lc)
    DeflateState *s;
    unsigned dist;  /* distance of matched string */
    unsigned lc;    /* match length-MIN_MATCH or unmatched char (if dist==0) */
{
    s.d_buf[s.last_lit] = (ush)dist;
    s.l_buf[s.last_lit++] = (uch)lc;
    if (dist == 0) {
        /* lc is the unmatched char */
        s.dyn_ltree[lc].Freq++;
    } else {
        s.matches++;
        /* Here, lc is the match length - MIN_MATCH */
        dist--;             /* dist = match distance - 1 */
        Assert((ush)dist < (ush)MAX_DIST(s) &&
               (ush)lc <= (ush)(MAX_MATCH-MIN_MATCH) &&
               (ush)d_code(dist) < (ush)D_CODES,  "_tr_tally: bad match");

        s.dyn_ltree[_length_code[lc]+LITERALS+1].Freq++;
        s.dyn_dtree[d_code(dist)].Freq++;
    }

#ifdef TRUNCATE_BLOCK
    /* Try to guess if it is profitable to stop the current block here */
    if ((s.last_lit & 0x1fff) == 0 && s.level > 2) {
        /* Compute an upper bound for the compressed length */
        ulg out_length = (ulg)s.last_lit*8L;
        ulg in_length = (ulg)((long)s.strstart - s.block_start);
        int dcode;
        for (dcode = 0; dcode < D_CODES; dcode++) {
            out_length += (ulg)s.dyn_dtree[dcode].Freq *
                (5L+extra_dbits[dcode]);
        }
        out_length >>= 3;
        Tracev((stderr,"\nlast_lit %u, in %ld, out ~%ld(%ld%%) ",
               s.last_lit, in_length, out_length,
               100L - out_length*100L/in_length));
        if (s.matches < s.last_lit/2 && out_length < in_length/2) return 1;
    }
#endif
    return (s.last_lit == s.lit_bufsize-1);
    /* We avoid equality with lit_bufsize because of wraparound at 64K
     * on 16 bit machines and because stored blocks are restricted to
     * 64K-1 bytes.
     */
}

/* ===========================================================================
 * Send the block data compressed using the given Huffman trees
 */
local void compress_block(s, ltree, dtree)
    DeflateState *s;
    const ct_data *ltree; /* literal tree */
    const ct_data *dtree; /* distance tree */
{
    unsigned dist;      /* distance of matched string */
    int lc;             /* match length or unmatched char (if dist == 0) */
    unsigned lx = 0;    /* running index in l_buf */
    unsigned code;      /* the code to send */
    int extra;          /* number of extra bits to send */

    if (s.last_lit != 0) do {
        dist = s.d_buf[lx];
        lc = s.l_buf[lx++];
        if (dist == 0) {
            send_code(s, lc, ltree); /* send a literal byte */
            Tracecv(isgraph(lc), (stderr," '%c' ", lc));
        } else {
            /* Here, lc is the match length - MIN_MATCH */
            code = _length_code[lc];
            send_code(s, code+LITERALS+1, ltree); /* send the length code */
            extra = extra_lbits[code];
            if (extra != 0) {
                lc -= base_length[code];
                send_bits(s, lc, extra);       /* send the extra length bits */
            }
            dist--; /* dist is now the match distance - 1 */
            code = d_code(dist);
            Assert (code < D_CODES, "bad d_code");

            send_code(s, code, dtree);       /* send the distance code */
            extra = extra_dbits[code];
            if (extra != 0) {
                dist -= base_dist[code];
                send_bits(s, dist, extra);   /* send the extra distance bits */
            }
        } /* literal or match pair ? */

        /* Check that the overlay between pending_buf and d_buf+l_buf is ok: */
        Assert((uint)(s.pending) < s.lit_bufsize + 2*lx,
               "pendingBuf overflow");

    } while (lx < s.last_lit);

    send_code(s, END_BLOCK, ltree);
}

/* ===========================================================================
 * Check if the data type is TEXT or BINARY, using the following algorithm:
 * - TEXT if the two conditions below are satisfied:
 *    a) There are no non-portable control characters belonging to the
 *       "black list" (0..6, 14..25, 28..31).
 *    b) There is at least one printable character belonging to the
 *       "white list" (9 {TAB}, 10 {LF}, 13 {CR}, 32..255).
 * - BINARY otherwise.
 * - The following partially-portable control characters form a
 *   "gray list" that is ignored in this detection algorithm:
 *   (7 {BEL}, 8 {BS}, 11 {VT}, 12 {FF}, 26 {SUB}, 27 {ESC}).
 * IN assertion: the fields Freq of dyn_ltree are set.
 */
local int detect_data_type(s)
    DeflateState *s;
{
    /* black_mask is the bit mask of black-listed bytes
     * set bits 0..6, 14..25, and 28..31
     * 0xf3ffc07f = binary 11110011111111111100000001111111
     */
    unsigned long black_mask = 0xf3ffc07fUL;
    int n;

    /* Check for non-textual ("black-listed") bytes. */
    for (n = 0; n <= 31; n++, black_mask >>= 1)
        if ((black_mask & 1) && (s.dyn_ltree[n].Freq != 0))
            return Z_BINARY;

    /* Check for textual ("white-listed") bytes. */
    if (s.dyn_ltree[9].Freq != 0 || s.dyn_ltree[10].Freq != 0
            || s.dyn_ltree[13].Freq != 0)
        return Z_TEXT;
    for (n = 32; n < LITERALS; n++)
        if (s.dyn_ltree[n].Freq != 0)
            return Z_TEXT;

    /* There are no "black-listed" or "white-listed" bytes:
     * this stream either is empty or has tolerated ("gray-listed") bytes only.
     */
    return Z_BINARY;
}
*/

/*
/* ===========================================================================
 * Flush the bit buffer, keeping at most 7 bits in it.
 */
local void bi_flush(s)
    DeflateState *s;
{
    if (s.bi_valid == 16) {
        put_short(s, s.bi_buf);
        s.bi_buf = 0;
        s.bi_valid = 0;
    } else if (s.bi_valid >= 8) {
        put_byte(s, (Byte)s.bi_buf);
        s.bi_buf >>= 8;
        s.bi_valid -= 8;
    }
}

/* ===========================================================================
 * Flush the bit buffer and align the output on a byte boundary
 */
local void bi_windup(s)
    DeflateState *s;
{
    if (s.bi_valid > 8) {
        put_short(s, s.bi_buf);
    } else if (s.bi_valid > 0) {
        put_byte(s, (Byte)s.bi_buf);
    }
    s.bi_buf = 0;
    s.bi_valid = 0;
#ifdef DEBUG
    s.bits_sent = (s.bits_sent+7) & ~7;
#endif
}

/* ===========================================================================
 * Copy a stored block, storing first the length and its
 * one's complement if requested.
 */
local void copy_block(s, buf, len, header)
    DeflateState *s;
    charf    *buf;    /* the input data */
    unsigned len;     /* its length */
    int      header;  /* true if block header must be written */
{
    bi_windup(s);        /* align on byte boundary */

    if (header) {
        put_short(s, (ush)len);
        put_short(s, (ush)~len);
#ifdef DEBUG
        s.bits_sent += 2*16;
#endif
    }
#ifdef DEBUG
    s.bits_sent += (ulg)len<<3;
#endif
    while (len--) {
        put_byte(s, *buf++);
    }
}



*/

/* Maximum value for windowBits in deflateInit2 and inflateInit2.
 * WARNING: reducing MAX_WBITS makes minigzip unable to extract .gz files
 * created by gzip. (Files created by minigzip can still be extracted by
 * gzip.)
 */
pub const MAX_WBITS :uint = 15; /* 32K LZ77 window */
pub const DEF_WBITS :uint = MAX_WBITS;

pub fn swap32(n: u32) -> u32
{
    (n >> 24)
    | ((n >> 8) & 0xff00)
    | ((n << 8) & 0xff0000)
    | (n << 24)
}

