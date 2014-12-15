extern crate getopts;
extern crate zlib;
use std::io;
use std::os;
// use zlib::inflate;

use getopts::{reqopt,optopt,getopts,OptGroup,usage};

use zlib::ZStream;
use zlib::inflate::InflateState;
use zlib::inflate::InflateResult;
use zlib::inflate::inflate;

const INBUF_SIZE :uint = 0x1000;
const OUTBUF_SIZE :uint = 0x1000;

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

fn main() {



    let input_filename: Option<String> = None;
    let output_filename: Option<String> = None;
    let knowngood_filename: Option<String> = None;

    let args = os::args();

    let main_opts: [OptGroup, ..4] = [
        optopt("g", "good", "known-good input filename", "good.bin"),
        optopt("h", "help", "Show help", ""),
        reqopt("i", "in", "input filename", "foo.tar.gz"),
        reqopt("o", "out", "output filename", "foo.tar"),
    ];

    let argmatches = match getopts(args.tail(), &main_opts) {
        Err(err) => {
            println!("Invalid arguments.  Use -h for help.");
            usage("", &main_opts);
            return;
        },
        Ok(m) => m
    };

    if argmatches.opt_present("h") {
        usage("", &main_opts);
        return;
    }

    let input_filename = argmatches.opt_str("i").unwrap();
    let output_filename = argmatches.opt_str("o").unwrap();
    let good_filename = argmatches.opt_str("g");

    // test zlib

    // let filename = &os::args()[1];
    // let filename = "c:\\users\\arlied\\downloads\\zlib-1.2.8.tar.gz";
    let input_path = Path::new(&input_filename);
    let input_file = io::File::open(&input_path);
    let mut input_reader = io::BufferedReader::new(input_file);

    let mut good_reader = good_filename.map(|goodfn| {
        println!("opening known-good file...");
        let path = Path::new(goodfn);
        let good_file = io::File::open(&path);
        Some(io::BufferedReader::new(good_file))
    });

    // Open the output file
    let output_file = io::File::create(&Path::new(&output_filename)).unwrap();
    let mut output_writer = io::BufferedWriter::new(output_file);

    let mut good_buffer: Vec<u8> = Vec::new();

    {
        let mut in_buffer: Vec<u8> = Vec::with_capacity(INBUF_SIZE);
        let mut out_buffer: Vec<u8> = Vec::with_capacity(OUTBUF_SIZE);

        in_buffer.grow(INBUF_SIZE, 0);
        out_buffer.grow(OUTBUF_SIZE, 0);

        let in_data = in_buffer.as_mut_slice();
        let out_data = out_buffer.as_mut_slice();

        let mut strm = ZStream::new();

        let mut state = InflateState::new(zlib::inflate::WINDOW_BITS_DEFAULT);
        state.wrap = 2;

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

            match inflate(&mut strm, &mut state, 0, in_data, out_data) {
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
/*
                    loop_count += 1;
                    if loop_count >= 4 {
                        println!("stopping");
                        break;
                    }
*/
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
}
