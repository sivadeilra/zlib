#![allow(dead_code)]

#![feature(phase)]
#[phase(plugin, link)]
extern crate log;

use std::default::Default;
use std::io;
use inftrees::{Code, LENS, DISTS, inflate_table};

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

fn makefixed(w: &mut Writer) {
    let mut fixed: [Code, ..544] = [Default::default(); 544];
    let mut work: [u16, ..288] = [Default::default(); 288];         // work area for code table building

    // build fixed huffman tables

    let mut lens: [u16, ..320] = [Default::default(); 320];         // temporary storage for code lengths

    // let mut next: uint = 0;

    /* literal/length table */
    {
        let mut sym :uint = 0;
        while sym < 144 { lens[sym] = 8; sym += 1; }
        while sym < 256 { lens[sym] = 9; sym += 1; }
        while sym < 280 { lens[sym] = 7; sym += 1; }
        while sym < 288 { lens[sym] = 8; sym += 1; }
    }

    let mut next :uint = 0;     // index into 'fixed' table
    let lenfix: uint = 0;       // index into 'fixed' table
    let (err, _) = inflate_table(LENS, &lens, 288, &mut fixed, &mut next, 9, work.as_mut_slice());
    assert!(err == 0);

    /* distance table */
    {
        let mut sym :uint = 0;
        while sym < 32 { lens[sym] = 5; sym += 1; }
    }
    let distfix: uint = next;      // index into 'fixed' table

    let (err, _) = inflate_table(DISTS, &lens, 32, &mut fixed, &mut next, 5, work.as_mut_slice());
    assert!(err == 0);

    let lencode = fixed.slice_from(lenfix);
    // let lenbits: uint = 9;
    let distcode = fixed.slice_from(distfix);
    // let distbits: uint = 5;

    w.write_str("    /* inffixed.h -- table for decoding fixed codes\n").unwrap();
    w.write_str("     * Generated automatically by makefixed().\n").unwrap();
    w.write_str("     */\n").unwrap();
    w.write_str("\n").unwrap();
    w.write_str("    /* WARNING: this file should *not* be used by applications.\n").unwrap();
    w.write_str("       It is part of the implementation of this library and is\n").unwrap();
    w.write_str("       subject to change. Applications should only use zlib.h.\n").unwrap();
    w.write_str("     */\n").unwrap();
    w.write_str("\n").unwrap();
    w.write_str("use super::inftrees::Code;").unwrap();

    let size = 1 << 9;
    w.write_str(format!("pub static LENFIX: [Code; {}] = [", size).as_slice()).unwrap();
    let mut low = 0;
    loop {
        if (low % 7) == 0 {
            w.write_str("\n        ").unwrap();
        }
        w.write_str(format!("Code {{ op: {}, bits: {}, val: {} }}", 
            if (low & 127) == 99 { 64 } else { lencode[low].op },
                lencode[low].bits,
                lencode[low].val).as_slice()).unwrap();
            low += 1;
            if low == size {
                break;
            }
        w.write_str(",").unwrap();
    }
    w.write_str("\n];").unwrap();

    let size = 1 << 5;
    w.write_str(format!("\npub static DISTFIX: [Code; {}] = [", size).as_slice()).unwrap();
    low = 0;
    loop {
        if (low % 6) == 0 {
            w.write_str("\n        ").unwrap();
        }
        w.write_str(format!("Code {{ op: {}, bits: {}, val: {} }}", distcode[low].op, distcode[low].bits, distcode[low].val).as_slice()).unwrap();
        low += 1;
        if low == size {
            break;
        }
        w.write_str(",").unwrap();
    }
    w.write_str("\n];").unwrap();
}

// Return state with length and distance decoding tables and index sizes set to
// fixed code decoding.  Normally this returns fixed tables from inffixed.h.
// If BUILDFIXED is defined, then instead this routine builds the tables the
// first time it's called, and returns those tables the first time and
// thereafter.  This reduces the size of the code by about 2K bytes, in
// exchange for a little execution time.  However, BUILDFIXED should not be
// used for threaded applications, since the rewriting of the tables and virgin
// may not be thread-safe.

#[path = "src/inflate/inftrees.rs"]
mod inftrees;

fn main() {
    let gen_path = Path::new("src/inflate/inffixed.rs");
    let mut gen_file = io::File::create(&gen_path);
    makefixed(&mut gen_file);
}