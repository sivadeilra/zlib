#![feature(phase)]
#[phase(plugin, link)] extern crate log;
extern crate zlib;

use std::fmt::Show;
use std::io;
use std::os;

use zlib::ZStream;
use zlib::inflate;
use zlib::inflate::InflateState;
use zlib::inflate::InflateResult;

const INBUF_SIZE :uint = 0x1000;
const OUTBUF_SIZE :uint = 0x1000;

#[allow(unused_variables)]

/*
fn str_match_start(s: &str, prefix: &str) -> Option<&str> {
    if s.starts_with(prefix) {
        Some(s.slice_from(prefix.len()))
    }
    else {
        None
    }
}
*/

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
fn test_inflate() {

    let input_filename = "tests/hamlet.tar.gz";
    let output_filename = "tests/out.tar";

    let input_path = Path::new(&input_filename);
    let input_file = unwrap_or_warn(io::File::open(&input_path));
    let mut input_reader = io::BufferedReader::new(input_file);

    // Open the output file
    let output_path = Path::new(&output_filename);
    let output_file = unwrap_or_warn(io::File::create(&output_path));
    let mut output_writer = io::BufferedWriter::new(output_file);

    let mut good_buffer: Vec<u8> = Vec::new();

	println!("successfully opened test files");

    let mut in_buffer: Vec<u8> = Vec::with_capacity(INBUF_SIZE);
    let mut out_buffer: Vec<u8> = Vec::with_capacity(OUTBUF_SIZE);

    in_buffer.grow(INBUF_SIZE, 0);
    out_buffer.grow(OUTBUF_SIZE, 0);

    let in_data = in_buffer.as_mut_slice();
    let out_data = out_buffer.as_mut_slice();

    let mut strm = ZStream::new();
    let mut state = InflateState::new(zlib::inflate::WINDOW_BITS_DEFAULT, 2);
    let mut input_eof = false;
    let mut loop_count: uint = 0;

    // Main loop
    loop {
        // println!("decode loop: avail_in = {}, next_in = {}, avail_out = {}, next_out = {}",
        //     strm.avail_in, strm.next_in, strm.avail_out, strm.next_out);

        if strm.avail_in == 0 && !input_eof {
            // println!("input buffer is empty; loading data");
            match input_reader.read(in_data) {
                Ok(bytes_read) => {
                    assert!(bytes_read > 0);
                    println!("zlibtest: loaded {} input bytes", bytes_read);
                    strm.next_in = 0;
                    strm.avail_in = bytes_read;
                },
                Err(_) => {
                    input_eof = true;
                    println!("input error (assuming eof)");
                }
            }
        }

        // give it some output buffer
        strm.avail_out = out_data.len();
        strm.next_out = 0;

        match state.inflate(&mut strm, None, in_data, out_data) {
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

                loop_count += 1;
                if loop_count >= 4 {
                    println!("stopping");
                    break;
                }

                if strm.next_out != 0 {
                    // Write the decoded contents to the output file
                    output_writer.write(out_data.slice(0, strm.next_out)).unwrap();                    
                    output_writer.flush().unwrap();
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

