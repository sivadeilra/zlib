/* adler32.c -- compute the Adler-32 checksum of a data stream
 * Copyright (C) 1995-2011 Mark Adler
 * For conditions of distribution and use, see copyright notice in zlib.h
 */

const BASE: u32 = 65521; // largest prime smaller than 65536
const NMAX: u32 = 5552; // NMAX is the largest n such that 255n(n+1)/2 + (n+1)(BASE-1) <= 2^32-1

pub fn adler32(adler: u32, buf: &[u8]) -> u32 {
    // split Adler-32 into component sums
    let mut sum2: u32 = (adler >> 16) & 0xffff;
    let mut adler: u32 = adler & 0xffff;

    // This is the "short loop" from adler32.c.
    /* in case short lengths are provided, keep it somewhat fast */
    for &b in buf.iter() {
        adler += b as u32;
        sum2 += adler;
    }
    if adler >= BASE {
        adler -= BASE;
    }
    sum2 %= BASE;
    return adler | (sum2 << 16);
}

