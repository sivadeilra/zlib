pub struct StaticTreeDesc
{
    // static_tree :&'static [CtData],  /* static tree or NULL */
    pub lengths :&'static [u8],
    pub codes :&'static [u16],

    pub extra_bits :&'static [u8],      /* extra bits for each code or NULL */
    pub extra_base :usize,          /* base index for extra_bits */
    pub elems :usize,               /* max number of elements in the tree */
    pub max_length :usize,         /* max bit length for the codes */
}

// This contains definitions for the ZLIB static trees.
// These definitions are in this file, rather than elsewhere,
// so that we can compile both the ZLIB library and the tool
// which generates the static tree tables.

pub const LENGTH_CODES :usize = 29;
/* number of length codes, not counting the special END_BLOCK code */

pub const LITERALS :usize = 256;
/* number of literal bytes 0..255 */

pub const L_CODES :usize = LITERALS + 1 + LENGTH_CODES;
/* number of Literal or Length codes, including the END_BLOCK code */

pub const D_CODES :usize = 30;
/* number of distance codes */

pub const BL_CODES :usize = 19;
/* number of codes used to transfer the bit lengths */

pub const DIST_CODE_LEN :usize = 512; /* see definition of array dist_code below */

pub const MIN_MATCH :usize = 3;
pub const MAX_MATCH :usize = 258;
/* The minimum and maximum match lengths */

pub const MAX_BITS :usize = 15;
/* All codes must not exceed MAX_BITS bits */

pub static EXTRA_LBITS: [u8; LENGTH_CODES] /* extra bits for each length code */
   = [0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0];

pub static EXTRA_DBITS: [u8; D_CODES] /* extra bits for each distance code */
   = [0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13];

pub static EXTRA_BLBITS: [u8; BL_CODES]/* extra bits for each bit length code */
   = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2,3,7];

pub static BL_ORDER: [u8; BL_CODES]
   = [16,17,18,0,8,7,9,6,10,5,11,4,12,3,13,2,14,1,15];
/* The lengths of the bit length codes are sent in order of decreasing
 * probability, to avoid transmitting the lengths for unused bit length codes.
 */

/* Data structure describing a single value and its code string. */
pub struct CtData // was ct_data
{
    // union {
    //     ush  freq;       /* frequency count */
    //     ush  code;       /* bit string */
    // } fc;
    pub fc :u16,

    // union {
    //     ush  dad;        /* father node in Huffman tree */
    //     ush  len;        /* length of bit string */
    // } dl;
    pub dl :u16,
}

// #define Freq fc.freq
// #define Code fc.code
// #define Dad  dl.dad
// #define Len  dl.len
