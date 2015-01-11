#![allow(unstable)]

extern crate zlib;

use std::io;
use std::iter::repeat;
use std::os;
use zlib::inflate::{Inflater,InflateResult};

fn main() {
    let out_bufsize: usize = 1 << 20; // fails at 0x10000

    let args = os::args();

    if args.len() != 2 {
        println!("expected input filename");
        return;
    }

    let input_path = Path::new(&args[1]);

    // open compressed input file
    let mut input_file = io::BufferedReader::new(io::File::open(&input_path).unwrap());

    // read the entire input file
    println!("reading input file");
    let input_data: Vec<u8> = input_file.read_to_end().unwrap();

    println!("read {} bytes", input_data.len());

    let mut output_buffer: Vec<u8> = Vec::with_capacity(out_bufsize);
    output_buffer.extend(repeat(0).take(out_bufsize));

    let out_data = output_buffer.as_mut_slice();

    let mut state = Inflater::new_gzip();

    let iter_count: usize = 100;

    let input_slice = input_data.as_slice();

    for iter in (0..iter_count) {

        state.reset();
        let mut inpos: usize = 0; // position within input_data

        let mut cycle: usize = 0;

        // Main loop
        loop {
            // let end = min(input_slice.len(), inpos);
            // let in_slice = input_slice.slice(inpos, end);
            let in_slice = input_slice.slice_from(inpos);
            match state.inflate(None, in_slice, out_data) {
                InflateResult::Eof(_) => {
                    println!("zlib says Z_STREAM_END");
                    break;
                }

                InflateResult::InvalidData => {
                    println!("InvalidData");
                    break;
                }

                InflateResult::Decoded(input_bytes_read, _) => {
                    // println!("InflateDecoded: input_bytes_read: {} output_bytes_written: {}", input_bytes_read, output_bytes_written);                
                    // println!("zlibtest: in_read={}, out_written={}", input_bytes_read, output_bytes_written);
                    inpos += input_bytes_read;
                }

                InflateResult::NeedInput => {
                    println!("NeedInput");
                    unimplemented!();
                }
            }

            cycle += 1;
            println!("cycle = {}", cycle);
        }

        println!("iteration #{} done.", iter);
    }
}
