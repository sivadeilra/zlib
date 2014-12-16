#![feature(phase)]
#[phase(plugin, link)] extern crate log;
extern crate zlib;

use std::fmt::Show;
use std::io;
use std::os;

use zlib::WINDOW_BITS_DEFAULT;
use zlib::ZStream;
use zlib::inflate;
use zlib::inflate::InflateState;
use zlib::inflate::InflateResult;

const INBUF_SIZE :uint = 0x1000;
const OUTBUF_SIZE :uint = 0x1000;

#[allow(unused_variables)]

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
    let input_path = Path::new("tests/hamlet.tar.gz");
    let check_path = Path::new("tests/hamlet.tar");			// contains the expected (good) output

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

    let mut strm = ZStream::new();
    let mut state = InflateState::new(WINDOW_BITS_DEFAULT, 2);
    let mut input_eof = false;
    let mut loop_count: uint = 0;

    // Main loop
    loop {
        // println!("decode loop: avail_in = {}, next_in = {}, avail_out = {}, next_out = {}",
        //     strm.avail_in, strm.next_in, strm.avail_out, strm.next_out);

        if input_pos == input_buffer.len() && !input_eof {
            // println!("input buffer is empty; loading data");
            let bytes_read = input_file.push(in_bufsize, &mut input_buffer).unwrap();
            assert!(bytes_read > 0);
            input_pos = 0;
            println!("loaded {} input bytes", bytes_read);
        }

        // give it some output buffer
        strm.avail_out = out_data.len();
        strm.next_out = 0;

        let total_out: u64 = strm.total_out;

        match state.inflate(&mut strm, None, input_buffer.slice_from(input_pos), out_data) {
            InflateResult::Eof(crc) => {
                println!("Eof");
                break;
            }

            InflateResult::InvalidData => {
                println!("InvalidData");
                break;
            }

            InflateResult::Decoded(input_bytes_read, output_bytes_written) => {
                // println!("InflateDecoded: input_bytes_read: {} output_bytes_written: {}", input_bytes_read, output_bytes_written);                
                println!("zlibtest: next_out = {}, avail_out = {}", strm.next_out, strm.avail_out);

                assert!(input_bytes_read + input_pos <= input_buffer.len());
                input_pos += input_bytes_read;

                // Check the data that we just received against the same data in the known-good file.
                if (output_bytes_written != 0) {
                	assert!(check_buffer.len() == 0);
                	let check_bytes_read = check_file.push(output_bytes_written, &mut check_buffer).unwrap();
                	assert!(check_bytes_read == output_bytes_written);
                	for i in range(0, output_bytes_written) {
                		if check_buffer[i] != out_data[i] {
                			panic!("outputs differ!  at output offset {}, expected {} found {}", total_out + (i as u64), check_buffer[i], out_data[i]);
                		}
                	}

					check_buffer.clear();
                }

                loop_count += 1;
                if loop_count >= 4 {
                    println!("stopping");
                    break;
                }
            }

            InflateResult::NeedInput => {
                println!("NeedInput");
                unimplemented!();
            }

            InflateResult::NeedOutput => {
                println!("NeedOutput");
                unimplemented!();
            }
        }

    }
}

