#![allow(unused_imports)]
#![allow(unused_mut)]
#![feature(macro_rules)]
#![feature(phase)]

#[phase(plugin, link)]
extern crate log;

extern crate test;
extern crate zlib;

use std::io;
use std::os;
use std::os::set_exit_status;
use std::io::IoErrorKind;
use std::io::IoError;
use test::Bencher;

use zlib::{WINDOW_BITS_DEFAULT};
use zlib::inflate::{Inflater,InflateResult};
use zlib::inflate::InflateReader;

fn run_zbench(
    bencher: &mut Bencher,
    filename: &str,
    input_buffer_size: uint, 
    output_buffer_size: uint,
    read_entire_file: bool)
{
    let iter_count: uint = 1;

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
    output_buffer.grow(output_buffer_size, 0);

    let out_data = output_buffer.as_mut_slice();

    let mut state = Inflater::new_gzip();
    let mut cycle: uint = 0;

    for iter in range(0, iter_count) {
        // This is the decode loop for an entire file.
        input_file.seek(0, io::SeekSet).unwrap();
        state.reset();

        let mut input_eof = false;
        let mut total_in: u64 = 0;
        let mut total_out: u64 = 0;

        let mut input_pos: uint = 0;

        bencher.iter(|| {
            loop {
                // Load more input data, if necessary.
                if input_pos == input_buffer.len() && !input_eof && !read_entire_file {
                    // println!("input buffer is empty; loading data");
                    input_buffer.clear();
                    input_pos = 0;
                    match input_file.push(input_buffer_size, &mut input_buffer) {
                        Ok(bytes_read) => {
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
