#![allow(unstable)]
#![feature(plugin)]

#[plugin]
#[macro_use]
extern crate log;

extern crate test;
extern crate zlib;

use std::io;
use std::iter::repeat;
use std::io::IoErrorKind;
use test::Bencher;

use zlib::inflate::{Inflater,InflateResult};

fn run_zbench(
    bencher: &mut Bencher,
    filename: &str,
    input_buffer_size: usize, 
    output_buffer_size: usize,
    read_entire_file: bool)
{
    let iter_count: usize = 1;

    let input_path = Path::new(&filename);

    // open compressed input file
    let mut input_file = io::File::open(&input_path);

    // If we are going to read the entire file, then do so now.
    // Else, set up the input buffer for reading in chunks.
    let mut input_buffer: Vec<u8>;
    if read_entire_file {
        input_buffer = input_file.read_to_end().unwrap();
        info!("read entire input file, size = {}", input_buffer.len());
    }
    else {
        info!("using buffered mode.");
        info!("    input buffer size: 0x{:x} {}", input_buffer_size, input_buffer_size);
        info!("    output buffer size: 0x{:x} {}", output_buffer_size, output_buffer_size);
        input_buffer = Vec::with_capacity(input_buffer_size);
    }

    // Allocate output buffer
    let mut output_buffer: Vec<u8> = Vec::with_capacity(output_buffer_size);
    output_buffer.extend(repeat(0).take(output_buffer_size));

    let out_data = output_buffer.as_mut_slice();

    let mut state = Inflater::new_gzip();
    let mut cycle: usize = 0;

    for _ in (0..iter_count) {
        // This is the decode loop for an entire file.
        input_file.seek(0, io::SeekSet).unwrap();
        state.reset();

        let mut input_eof = false;
        let mut total_in: u64 = 0;
        let mut total_out: u64 = 0;

        let mut input_pos: usize = 0;

        bencher.iter(|| {
            loop {
                // Load more input data, if necessary.
                if input_pos == input_buffer.len() && !input_eof && !read_entire_file {
                    // println!("input buffer is empty; loading data");
                    input_buffer.clear();
                    input_pos = 0;
                    match input_file.push(input_buffer_size, &mut input_buffer) {
                        Ok(_) => {
                            // ok, loaded some input data
                        }
                        Err(err) => {
                            if err.kind == IoErrorKind::EndOfFile {
                                // println!("input stream EOF");
                                input_eof = true;
                            }
                            else {
                                println!("unexpected input error: {}", err.desc);
                                break;
                            }
                        }
                    };
                }

                match state.inflate(None, input_buffer.slice_from(input_pos), out_data) {
                    InflateResult::Eof(_) => {
                        break;
                    }

                    InflateResult::InvalidData => {
                        println!("InvalidData");
                        break;
                    }

                    InflateResult::Decoded(input_bytes_read, output_bytes_written) => {
                        // println!("InflateDecoded: input_bytes_read: {} output_bytes_written: {}", input_bytes_read, output_bytes_written);                
                        // println!("zlibtest: in_read={}, out_written={}", input_bytes_read, output_bytes_written);
                        total_in += input_bytes_read as u64;

                        assert!(input_bytes_read + input_pos <= input_buffer.len());
                        input_pos += input_bytes_read;

                        total_out += output_bytes_written as u64;
                    }

                    InflateResult::NeedInput => {
                        println!("NeedInput");
                        unimplemented!();
                    }
                }

                cycle += 1;
            }

        });
    }
}

#[bench] fn bench_small_0x1000_0x1000(b: &mut Bencher) { run_zbench(b, "zlib-1.2.8.tar.gz", 0x1000, 0x1000, false); }
#[bench] fn bench_small_0x10000_0x10000(b: &mut Bencher) { run_zbench(b, "zlib-1.2.8.tar.gz", 0x10000, 0x10000, false); }
#[bench] fn bench_small_0x100000_0x100000(b: &mut Bencher) { run_zbench(b, "zlib-1.2.8.tar.gz", 0x100000, 0x100000, false); }
