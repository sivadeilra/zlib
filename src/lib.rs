#![feature(plugin)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unstable)]

#[plugin]
#[macro_use]
extern crate log;

extern crate crc32;

pub macro_rules! Tracevv {
    ($($arg:tt)*) => {
        if cfg!(not(ndebug)) {
            // println!($($arg)*)
        }
    }
}

mod adler32;
pub mod inflate;
mod statictrees;
mod treedefs;
mod deflate;

// From deflate.h

// The three kinds of block type
const STORED_BLOCK: u32 = 0;
const STATIC_TREES: u32 = 1;
const DYN_TREES: u32 = 2;

pub const PRESET_DICT: u32 = 0x20; /* preset dictionary flag in zlib header */

pub const WINDOW_BITS_MIN: usize = 8;
pub const WINDOW_BITS_MAX: usize = 15;
pub const WINDOW_BITS_DEFAULT: usize = WINDOW_BITS_MAX;

#[derive(Copy,Show,Eq,PartialEq)]
pub enum WrapKind {
    Zlib,
    Gzip
}

struct ZStream {
    pub total_in: u64,              // total number of input bytes read so far
    pub total_out: u64,             // total number of bytes output so far
    pub msg: Option<&'static str>,  // last error message, if any
    pub data_type :u32,            // best guess about the data type: binary or text
    pub adler: u32                  // adler32 value of the uncompressed data
}

impl ZStream {
    pub fn new() -> ZStream {
        ZStream {
            total_in: 0,
            total_out: 0,
            msg: None,
            data_type: 0,
            adler: 0,
        }
    }
}

/// gzip header information passed to and from zlib routines.  See RFC 1952
/// for more details on the meanings of these fields.
pub struct GZipHeader {
    pub text: bool,                     // true if compressed data believed to be text
    pub time: u32,                      // modification time
    pub xflags: u32,                    // extra flags (not used when writing a gzip file)
    pub os: u32,                        // operating system
    pub extra_len: usize,                // length of the 'extra' data, in bytes
    pub extra: Option<Box<Vec<u8>>>,    // extra field data, if any
//    pub name_len: usize,                 // length of the 'name' data, in bytes (not chars!)
    pub name: Option<Box<String>>,      // filename, if any
//    pub comm_len: usize,                 // length of the 'comment' data, in bytes (not chars!)
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

/* constants */

/* Allowed flush values; see deflate() and inflate() below for details */
#[derive(Copy,Show,PartialEq,Eq)]
pub enum Flush {
    None = 0,
    PartialFlush = 1,
    SyncFlush = 2,
    FullFlush = 3,
    Finish = 4,
    Block = 5,
    Trees = 6
}

/*
pub const Z_NO_FLUSH     : u32 = 0;
pub const Z_PARTIAL_FLUSH: u32 = 1;
pub const Z_SYNC_FLUSH   : u32 = 2;
pub const Z_FULL_FLUSH   : u32 = 3;
pub const Z_FINISH       : u32 = 4;
pub const Z_BLOCK        : u32 = 5;
pub const Z_TREES        : u32 = 6;
*/

#[derive(Copy,PartialEq,Eq)]
pub enum ZERR {
    Ok              = 0,        // Z_OK            = 0,
    StreamEnd       = 1,        // Z_STREAM_END    = 1,
    NeedDict        = 2,        // Z_NEED_DICT     = 2,
    Errno           = -1,        // Z_ERRNO         = -1,
    StreamError     = -2,        // Z_STREAM_ERROR  = -2,
    DataError       = -3,        // Z_DATA_ERROR    = -3,
//    BufError        = -5,        // Z_BUF_ERROR     = -5,
//    VersionError    = -6,        // Z_VERSION_ERROR = -6,
    // Return codes for the compression/decompression functions. Negative values
    // are errors, positive values are used for special but normal events.
}

/* compression levels */
pub const Z_NO_COMPRESSION     : i32 = 0;
pub const Z_BEST_SPEED         : i32 = 1;
pub const Z_BEST_COMPRESSION   : i32 = 9;
pub const Z_DEFAULT_COMPRESSION: i32 = -1;

pub const Z_FILTERED            :usize = 1;
pub const Z_HUFFMAN_ONLY        :usize = 2;
pub const Z_RLE                 :usize = 3;
pub const Z_FIXED               :usize = 4;
pub const Z_DEFAULT_STRATEGY    :usize = 0;
/* compression strategy; see deflateInit2() below for details */

pub const Z_BINARY   :u32 = 0;
pub const Z_TEXT     :u32 = 1;
pub const Z_ASCII    :u32 = Z_TEXT;   /* for compatibility with 1.2.2 and earlier */
pub const Z_UNKNOWN  :u32 = 2;
/* Possible values of the data_type field (though see inflate()) */

pub const Z_DEFLATED :u32 = 8;
/* The deflate compression method (the only one supported in this version) */

/* Maximum value for windowBits in deflateInit2 and inflateInit2.
 * WARNING: reducing MAX_WBITS makes minigzip unable to extract .gz files
 * created by gzip. (Files created by minigzip can still be extracted by
 * gzip.)
 */
pub const MAX_WBITS :usize = 15; /* 32K LZ77 window */
pub const DEF_WBITS :usize = MAX_WBITS;

fn swap32(n: u32) -> u32 {
    (n >> 24)
    | ((n >> 8) & 0xff00)
    | ((n << 8) & 0xff0000)
    | (n << 24)
}
