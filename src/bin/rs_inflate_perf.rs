#![allow(unused_imports)]
#![allow(unused_mut)]
#![allow(dead_code)]

extern crate zlib;

use std::io;
use std::os;
use zlib::{WINDOW_BITS_DEFAULT,ZStream};
use zlib::inflate::{InflateState,InflateResult};
use zlib::inflate::InflateReader;
use std::io::IoErrorKind;
use std::io::IoError;
use std::cmp::min;

fn main()
{
    let out_bufsize: uint = 1 << 20; // fails at 0x10000

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
    output_buffer.grow(out_bufsize, 0);

    let out_data = output_buffer.as_mut_slice();

    let mut strm = ZStream::new();
    let mut state = InflateState::new(WINDOW_BITS_DEFAULT, 2);

    let iter_count: uint = 100;

    let input_slice = input_data.as_slice();

    for iter in range(0, iter_count) {

        state.reset(&mut strm);
        let mut inpos: uint = 0; // position within input_data

        let mut cycle: uint = 0;

        // Main loop
        loop {
            // let end = min(input_slice.len(), inpos);
            // let in_slice = input_slice.slice(inpos, end);
            let in_slice = input_slice.slice_from(inpos);
            match state.inflate(&mut strm, None, in_slice, out_data) {
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

fn print_block(data: &[u8]) {
    let mut s = String::new();

    let width = 32;

    static HEX: [char; 0x10] = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f' ];

    println!("print_block: len={}", data.len());

    for i in range(0, data.len()) {
        let b = data[i];
        s.push(' ');
        s.push(HEX[(b >> 4) as uint]);
        s.push(HEX[(b & 0xf) as uint]);
        if ((i + 1) % width) == 0 {
            println!("{}", s);
            s.clear();
        }
    }

    if (data.len() % width) != 0 {
        println!("{}", s);
    }
}
