#![feature(macro_rules)]

extern crate zlib;

use std::io;
use std::fmt::Show;
use std::os;
use zlib::{WINDOW_BITS_DEFAULT};
use zlib::inflate::{Inflater,InflateResult};
use zlib::inflate::InflateReader;
use std::io::IoErrorKind;
use std::io::IoError;

const INBUF_SIZE :uint = 0x1000;
const OUTBUF_SIZE :uint = 0x1000;

fn unwrap_or_warn<T,E:Show>(op: Result<T,E>) -> T
{
	match op {
		Ok(val) => val,
		Err(err) => {
			panic!("Failed to open an input file.  Make sure you run this from the root of the 'zlib' dir.  {}", err);
		}
	}
}

#[test]
fn test_inflate_large_bufs() 
{
	test_inflate(0x10000, 0x10000); // 64 KB
}

#[test]
fn test_inflate_tiny_bufs()
{
	test_inflate(0x40, 0x40); // 64 bytes
}

#[test]
fn test_tiny_inbuf_large_outbuf()
{
	test_inflate(0x40, 0x10000);
}

#[test]
fn test_large_inbuf_tiny_outbuf()
{
	test_inflate(0x40, 0x10000);
}

fn test_inflate(in_bufsize: uint, out_bufsize: uint)
{
    let input_path = Path::new("zlib-1.2.8.tar.gz");
    let check_path = Path::new("zlib-1.2.8.tar");			// contains the expected (good) output

    // open compressed input file
    let mut input_file = io::BufferedReader::new(unwrap_or_warn(io::File::open(&input_path)));

    // open known-good input file
    let mut check_file = io::BufferedReader::new(unwrap_or_warn(io::File::open(&check_path)));

	println!("successfully opened test files");

    let mut input_buffer: Vec<u8> = Vec::with_capacity(in_bufsize);
    let mut output_buffer: Vec<u8> = Vec::with_capacity(out_bufsize);
    output_buffer.grow(out_bufsize, 0);
    let mut check_buffer: Vec<u8> = Vec::new();

    let mut input_pos: uint = 0; // index of next byte in input_buffer to read

    let out_data = output_buffer.as_mut_slice();

    let mut state = Inflater::new_gzip();
    let mut input_eof = false;
    let mut loop_count: uint = 0;
    let mut total_out: u64 = 0;

    // Main loop
    loop {
        if input_pos == input_buffer.len() && !input_eof {
            input_buffer.clear();
            input_pos = 0;
            let bytes_read = match input_file.push(in_bufsize, &mut input_buffer) {
                Ok(n) => {
                    assert!(n > 0);
                    println!("loaded {} input bytes", n);
                    n
                }
                Err(err) => {
                    if err.kind == io::EndOfFile {
                        println!("reached EOF on input");
                        input_eof = true;
                        0
                    }
                    else {
                        panic!("failed to read input: {}", err);
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
                println!("input_bytes_read = {}, output_bytes_written = {}", input_bytes_read, output_bytes_written);

                assert!(input_bytes_read + input_pos <= input_buffer.len());
                input_pos += input_bytes_read;

                // Check the data that we just received against the same data in the known-good file.
                if output_bytes_written != 0 {
                    let mut cpos = 0;
                    while cpos < output_bytes_written {
                        let clen_want = output_bytes_written - cpos;
                	    assert!(check_buffer.len() == 0);
                	    let clen_got = check_file.push(clen_want, &mut check_buffer).unwrap();
                        assert!(clen_got <= clen_want);

                	    for i in range(0, clen_got) {
                		    if check_buffer[i] != out_data[cpos + i] {
                			    panic!("outputs differ!  at output offset {}, expected {} found {}",
                                    total_out + (i as u64),
                                    check_buffer[i],
                                    out_data[cpos + i]);
                		    }
                	    }

                        cpos += clen_got;
					    check_buffer.clear();
                    }
                }

                loop_count += 1;
                total_out += output_bytes_written as u64;
            }

            InflateResult::NeedInput => {
                println!("NeedInput");
                unimplemented!();
            }
        }
    }
}

#[test]
fn test_inflate_reader_basic()
{
    test_inflate_reader("zlib-1.2.8.tar.gz", "zlib-1.2.8.tar", 0x1000, 0x1000);
}

fn test_inflate_reader(input_filename: &str, check_filename: &str, in_bufsize: uint, out_bufsize: uint)
{
    let input_path = Path::new(input_filename);
    let check_path = Path::new(check_filename);     // contains the expected (good) output

    // open compressed input file
    let input_file = io::BufferedReader::new(unwrap_or_warn(io::File::open(&input_path)));

    // open known-good input file
    let mut check_file = io::BufferedReader::new(unwrap_or_warn(io::File::open(&check_path)));

	println!("successfully opened test files");

    // create an InflateReader over the input file
    let mut inflater = InflateReader::new_gzip(in_bufsize, box input_file);

    let mut output_buffer: Vec<u8> = Vec::with_capacity(out_bufsize);
    let mut check_buffer: Vec<u8> = Vec::new();

    let mut total_out: u64 = 0;

    loop {
        match inflater.push(out_bufsize, &mut output_buffer) {
            Ok(output_bytes_written) => {
                println!("inflate reader returned {} bytes", output_bytes_written);

                // Check the data that we just received against the same data in the known-good file.
                if output_bytes_written != 0 {
                	assert!(check_buffer.len() == 0);
                	let check_bytes_read = check_file.push(output_bytes_written, &mut check_buffer).unwrap();
                	assert!(check_bytes_read == output_bytes_written);
                	for i in range(0, output_bytes_written) {
                		if check_buffer[i] != output_buffer[i] {
                			panic!("outputs differ!  at output offset {}, expected {} found {}", total_out + (i as u64), check_buffer[i], output_buffer[i]);
                		}
                	}

					check_buffer.clear();
                    output_buffer.clear();
                }

                total_out += output_bytes_written as u64;
            }
            Err(_) => {
                println!("push() returned error, assuming EOF for now");
                break;
            }
        }
    }
}
