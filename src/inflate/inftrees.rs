use std::iter::range_inclusive;

/* inftrees.h -- header to use inftrees.c
 * Copyright (C) 1995-2005, 2010 Mark Adler
 * For conditions of distribution and use, see copyright notice in zlib.h
 */

/* WARNING: this file should *not* be used by applications. It is
   part of the implementation of the compression library and is
   subject to change. Applications should only use zlib.h.
 */

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
#[derive(Copy,Default)]
pub struct Code {
    // operation, extra bits, table bits
    // op values as set by inflate_table():
    // 00000000 - literal
    // 0000tttt - table link, tttt != 0 is the number of table index bits
    // 0001eeee - length or distance, eeee is the number of extra bits
    // 01100000 - end of block
    // 01000000 - invalid code
    pub op: u8,

    /// bits in this part of the code
    pub bits: u8,

    /// offset in table or code value
    pub val: u16,
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
pub const ENOUGH_LENS :usize = 852;
pub const ENOUGH_DISTS :usize = 592;
pub const ENOUGH :usize = ENOUGH_LENS + ENOUGH_DISTS;

/* Type of code to build for inflate_table() */
// enum codetype {
pub type CodeType = u8;
    pub const CODES: u8 = 0;
    pub const LENS: u8 = 1;
    pub const DISTS: u8 = 2;
// }

// inftrees.c -- generate Huffman trees for efficient decoding
// Copyright (C) 1995-2013 Mark Adler
// For conditions of distribution and use, see copyright notice in zlib.h

pub const MAXBITS :usize = 15;

// const char inflate_copyright[] =
//    " inflate 1.2.8 Copyright 1995-2013 Mark Adler ";

/*
  If you use the zlib library in a product, an acknowledgment is welcome
  in the documentation of your product. If for some reason you cannot
  include such an acknowledgment, I would appreciate that you keep this
  copyright string in the executable of your product.
 */

/*
   Build a set of tables to decode the provided canonical Huffman code.
   The code lengths are lens[0..codes-1].  The result starts at *table,
   whose indices are 0..2^bits-1.  work is a writable array of at least
   lens shorts, which is used as a work area.  type is the type of code
   to be generated, CODES, LENS, or DISTS.  On return, zero is success,
   -1 is an invalid code, and +1 means that ENOUGH isn't enough.  table
   on return points to the next available entry's address.  bits is the
   requested root table index bits, and on return it is the actual root
   table index bits.  It will differ if the request is greater than the
   longest code or if it is less than the shortest code.
 */
pub fn inflate_table(
    ctype: CodeType,
    lens: &[u16],
    codes: usize,
    table: &mut [Code],
    table_pos: &mut usize,       // index into 'table'
    bits: usize,
    work: &mut [u16])
    -> (isize /*error*/, usize /*bits*/)
{
    // debug!("inflate_table: ctype {}, codes {}, bits {}", ctype as u32, codes, bits);
    static LBASE :[u16; 31] = [ /* Length codes 257..285 base */
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31,
        35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 0, 0];
    static LEXT :[u16; 31] = [ /* Length codes 257..285 extra */
        16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 18, 18, 18, 18,
        19, 19, 19, 19, 20, 20, 20, 20, 21, 21, 21, 21, 16, 72, 78 ];
    static DBASE :[u16; 32] = [ /* Distance codes 0..29 base */
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193,
        257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
        8193, 12289, 16385, 24577, 0, 0 ];
    static DEXT :[u16; 32] = [ /* Distance codes 0..29 extra */
        16, 16, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22, 22,
        23, 23, 24, 24, 25, 25, 26, 26, 27, 27,
        28, 28, 29, 29, 64, 64 ];

    /*
       Process a set of code lengths to create a canonical Huffman code.  The
       code lengths are lens[0..codes-1].  Each length corresponds to the
       symbols 0..codes-1.  The Huffman code is generated by first sorting the
       symbols by length from short to long, and retaining the symbol order
       for codes with equal lengths.  Then the code starts with all zero bits
       for the first code of the shortest length, and the codes are integer
       increments for the same length, and zeros are appended as the length
       increases.  For the deflate format, these bits are stored backwards
       from their more natural integer increment ordering, and so when the
       decoding tables are built in the large loop below, the integer codes
       are incremented backwards.

       This routine assumes, but does not check, that all of the entries in
       lens[] are in the range 0..MAXBITS.  The caller must assure this.
       1..MAXBITS is interpreted as that code length.  zero means that that
       symbol does not occur in this code.

       The codes are sorted by computing a count of codes for each length,
       creating from that a table of starting indices for each length in the
       sorted table, and then entering the symbols in order in the sorted
       table.  The sorted table is work[], with that space being provided by
       the caller.

       The length counts are used for other purposes as well, i.e. finding
       the minimum and maximum length codes, determining if there are any
       codes at all, checking for a valid set of lengths, and looking ahead
       at length counts to determine sub-table sizes when building the
       decoding tables.
     */

    // accumulate lengths for codes (assumes lens[] all in 0..MAXBITS)
    let mut count = [0u16; MAXBITS+1]; // number of codes of each length
    for sym in range(0, codes) {
        count[lens[sym] as usize] += 1;
    }

    // debug!("counts:");
    // for i in range_inclusive(0, MAXBITS) {
    //     debug!("    count[{}] = {}", i, count[i]);
    // }

    // bound code lengths, force root to be within code lengths
    let mut max :usize = MAXBITS;      // maximum code lengths
    while max >= 1 {
        if count[max] != 0 {
            break;
        }
        max -= 1;
    }

    if max == 0 {
        // no symbols to code at all
        debug!("max == 0, so there are no symbols to code at all");
        let here = Code { op: 64, bits: 1, val: 0, }; // invalid code marker
        // make a table to force an error
        table[*table_pos] = here; *table_pos += 1;
        table[*table_pos] = here; *table_pos += 1;
        return (0, 1);     /* no symbols, but wait for decoding to report error */
    }

    let mut min :usize = 1; // minimum code length
    while min < max {
        if count[min] != 0 {
            break;
        }
        min += 1;
    }

    let mut root :usize = bits;      // number of index bits for root table
    if root > max {
        root = max;
    }
    if root < min {
        root = min;
    }

    // debug!("root {}, min {}, max {}", root, min, max);

    // check for an over-subscribed or incomplete set of lengths
    {
        let mut left: i32 = 1; // number of prefix codes available
        for len in range_inclusive(1, MAXBITS) {
            left <<= 1;
            left -= count[len] as i32;
            if left < 0 {
                warn!("over-subscribed");
                return (-1, bits); // over-subscribed
            }
        }
        if left > 0 && (ctype == CODES || max != 1) {
            warn!("incomplete set of lengths");
            return (-1, bits); // incomplete set
        }
    }

    // generate offsets into symbol table for each length for sorting
    let mut offs = [0u16; MAXBITS+1];     // offsets in table for each length
    offs[1] = 0;
    for len in range(1, MAXBITS) {
        offs[len + 1] = offs[len] + count[len];
    }

    // sort symbols by length, by symbol order within each length
    for sym in range(0u16, codes as u16) {
        let symlen = lens[sym as usize] as usize;
        if symlen != 0 {
            let symoff = offs[symlen] as usize;
            work[symoff] = sym;
            offs[symlen] += 1;
        }
    }

    // Create and fill in decoding tables.  In this loop, the table being
    // filled is at table[next] and has curr index bits.  The code being used is huff
    // with length len.  That code is converted to an index by dropping drop
    // bits off of the bottom.  For codes where len is less than drop + curr,
    // those top drop + curr - len bits are incremented through all values to
    // fill the table with replicated entries.
    //
    // root is the number of index bits for the root table.  When len exceeds
    // root, sub-tables are created pointed to by the root entry with an index
    // of the low root bits of huff.  This is saved in low to check for when a
    // new sub-table should be started.  drop is zero when the root table is
    // being filled, and drop is root when sub-tables are being filled.
    //
    // When a new sub-table is needed, it is necessary to look ahead in the
    // code lengths to determine what size sub-table is needed.  The length
    // counts are used for this, and so count[] is decremented as codes are
    // entered in the tables.
    //
    // used keeps track of how many table entries have been allocated from the
    // provided *table space.  It is checked for LENS and DIST tables against
    // the constants ENOUGH_LENS and ENOUGH_DISTS to guard against changes in
    // the initial root table size constants.  See the comments in inftrees.h
    // for more information.
    //
    // sym increments through all symbols, and the loop terminates when
    // all codes of length max, i.e. all codes, have been processed.  This
    // routine permits incomplete codes, so another loop after this one fills
    // in the rest of the decoding tables with invalid code markers.

    static EMPTY_U16: [u16; 0] = [];

    let (base,              // base value table to use
        base_bias,          // offset into 'base' to use, can be negative
        extra,              // extra bits table to use
        extra_bias,         // offset into 'extra' to use, can be negative
        end)                // use base and extra for symbol > end
        = match ctype {
        CODES => (&EMPTY_U16[], 0, &EMPTY_U16[], 0, 19),    // base/extra not used
        LENS => (&LBASE[], -257, &LEXT[], -257, 256),
        _ /* DISTS */ => (&DBASE[], 0, &DEXT[], 0, -1)
    };

    // debug!("base.len = {}, extra.len = {}", base.len(), extra.len());

    // initialize state for loop
    let mut used :usize = 1 << root;     // code entries in table used; use root table entries
    let mask :usize = used - 1;          // mask for comparing low root bits

    // check available table space
    if (ctype == LENS && used > ENOUGH_LENS) ||
        (ctype == DISTS && used > ENOUGH_DISTS) {
        warn!("too many positions used");
        return (1, bits);
    }

    let mut huff: usize = 0;             // starting Huffman code
    let mut sym: usize = 0;              // index of code symbols; starting code symbol
    let mut len: usize = min;            // starting code length, in bits
    let mut next: usize = *table_pos;    // next available space in 'table'; current table to fill in
    let mut curr: usize = root;          // number of index bits for current table; current table index bits
    let mut drop: usize = 0;             // code bits to drop for sub-table; current bits to drop from code for index
    let mut low: usize = !0us;           // low bits for current root entry; trigger new sub-table when len > root

    // process all codes and make table entries
    // debug!("processing codes");
    loop {
        /* create table entry */
        let (here_op, here_val) =
            if (work[sym] as isize) < end {
                (0, work[sym])
            }
            else if (work[sym] as isize) > end {
                (extra[(extra_bias + work[sym] as isize) as usize] as u8,
                    base[(base_bias + work[sym] as isize) as usize])
            }
            else {
                (32 + 64, 0)         /* end of block */
            };
        let here = Code {
            bits: (len - drop) as u8,
            op: here_op,
            val: here_val
        };

        /* replicate for those indices with low len bits equal to huff */
        {
            let incr :usize = 1 << (len - drop);
            let mut fill :usize = 1 << curr;     // index for replicating entries
            min = fill;                 /* save offset to next table */
            loop {
                fill -= incr;
                table[next + (huff >> drop) + fill] = here;
                if fill == 0 {
                    break;
                }
            }
        }

        /* backwards increment the len-bit code huff */
        {
            let mut incr = 1 << (len - 1);
            while (huff & incr) != 0 {
                incr >>= 1;
            }
            huff = if incr != 0 {
                (huff & (incr - 1)) + incr
            }
            else {
                0
            };
        }
        /* go to next symbol, update count, len */
        sym += 1;
        count[len] -= 1;
        if count[len] == 0 {
            if len == max {
                break;
            }
            len = lens[work[sym] as usize] as usize;
        }

        /* create new sub-table if needed */
        if len > root && (huff & mask) != low {
            /* if first time, transition to sub-tables */
            if drop == 0 {
                drop = root;
            }

            /* increment past last table */
            next += min;            /* here min is 1 << curr */

            /* determine length of next table */
            curr = len - drop;
            let mut left :isize = 1 << curr;
            while curr + drop < max {
                left -= count[curr + drop] as isize;
                if left <= 0 {
                    break;
                }
                curr += 1;
                left <<= 1;
            }

            // check for enough space
            used += 1 << curr;
            if (ctype == LENS && used > ENOUGH_LENS) ||
                (ctype == DISTS && used > ENOUGH_DISTS) {
                return (1, bits);
            }

            // point entry in root table to sub-table
            low = huff & mask;
            table[*table_pos + low] = Code {
                op: curr as u8,
                bits: root as u8,
                val: (next - *table_pos) as u16
            };
        }
    }

    // fill in remaining table entry if code is incomplete (guaranteed to have
    // at most one remaining entry, since if the code is incomplete, the
    // maximum code length that was allowed to get this far is one bit)
    if huff != 0 {
        table[next + huff] = Code {
            op: 64,             /* invalid code marker */
            bits: (len - drop) as u8,
            val: 0
        };
    }

    /* set return parameters */
    *table_pos += used;
    // debug!("done.  table_pos = {}, used = {}, root = {}", *table_pos, used, root);
    return (0, root);
}
