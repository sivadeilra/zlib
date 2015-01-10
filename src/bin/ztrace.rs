#![allow(unused_imports)]
#![allow(unused_mut)]

extern crate zlib;

use std::io;
use std::iter::repeat;
use std::os;
use std::os::set_exit_status;
use zlib::{WINDOW_BITS_DEFAULT};
use zlib::inflate::{Inflater,InflateResult};
use zlib::inflate::InflateReader;
use std::io::IoErrorKind;
use std::io::IoError;

macro_rules! bad_arg {
    ($arg:expr, $msg:expr) => {
        {
            println!("arg '{}' is invalid: {}", $arg, $msg);
            set_exit_status(1);
            return;
        }
    }
}

macro_rules! get_int_arg {
    ($arg:expr, $valopt:expr, $min:expr) => {
        {
            if let Some(aval) = $valopt {
                let value: uint = if let Some(value) = aval.parse::<uint>() { value } else {
                    println!("arg '{}' is invalid: value is not a valid number", $arg);
                    set_exit_status(1);
                    return;
                };
                if value < $min {
                    println!("arg '{}' is invalid: the value is too small", $arg);
                    set_exit_status(1);
                    return;
                }
                value
            }
            else {
                println!("arg '{}' is invalid: it requires that a value be specified, i.e. -foo:<nnn>", $arg);
                set_exit_status(1);
                return;
            }
        }
    }
}

fn usage() {
    println!("usage: [flags] <input-path>");
    println!("");
    println!("    -v            verbose mode");
    println!("    -i:<nnn>      iteration count");
    println!("    -ib:<nnn>     input buffer size");
    println!("    -ob:<nnn>     output buffer size");
}

struct CheckFileState {
    reader: io::File,
}


fn main() {
    let mut iter_count: uint = 1;
    let mut input_filename: Option<String> = None;
    let mut check_filename: Option<String> = None;
    let mut input_buffer_size: uint = 0x10000;
    let mut output_buffer_size: uint = 0x10000;
    let mut verbose = false;
    let mut verbose_print_blocks = false;
    let mut read_entire_file = false;
    
    let arg_prefix = "-";

    let args = os::args();
    if args.len() == 1 {
        usage();
        set_exit_status(1);
        return;
    }
    for i in range(1, args.len()) {
        let arg = &args[i];
        if arg.starts_with(arg_prefix) {
            let mut ai = arg.slice_from(arg_prefix.len()).splitn(1, ':');
            let aname: &str = if let Some(aname) = ai.next() { aname } else {
                bad_arg!(arg, "expected arg name");
            };
            let valopt: Option<&str> = ai.next();

            match aname {
                "i" => {
                    iter_count = get_int_arg!(arg, valopt, 1);
                }
                "ib" => {
                    input_buffer_size = get_int_arg!(arg, valopt, 1);
                }
                "ob" => {
                    output_buffer_size = get_int_arg!(arg, valopt, 1);
                }
                "v" => {
                    verbose = true;
                }
                "vv" => {
                    verbose_print_blocks = true;
                }
                "c" => {
                    if check_filename == None {
                        if let Some(val) = valopt {
                            check_filename = Some(val.to_string());
                        }
                        else {
                            bad_arg!(arg, "value is required");
                        }
                    }
                    else {
                        bad_arg!(arg, "the check filename cannot be specified more than once.");
                    }
                }
                "F" => {
                    read_entire_file = true
                }
                _ => {
                    bad_arg!(arg, "arg not recognized");
                }
            }
        }
        else {
            if input_filename == None {
                input_filename = Some(arg.to_string());
            }
            else {
                println!("error: input filename specified more than once.");
                set_exit_status(1);
                return;
            }
        }
    }

    // Check and rebind 'input_filename'
    let input_filename = if let Some(fname) = input_filename {
        fname
    }
    else {
        println!("error: input filename not specified.");
        set_exit_status(1);
        return;
    };
    let input_path = Path::new(&input_filename);

    // open compressed input file
    let mut input_file = io::File::open(&input_path);

    // If we are going to read the entire file, then do so now.
    // Else, set up the input buffer for reading in chunks.
    let mut input_buffer: Vec<u8>;
    if read_entire_file {
        input_buffer = input_file.read_to_end().unwrap();
        // println!("read entire input file, size = {}", input_buffer.len());
    }
    else {
        println!("using buffered mode.");
        println!("    input buffer size: 0x{:x} {}", input_buffer_size, input_buffer_size);
        println!("    output buffer size: 0x{:x} {}", output_buffer_size, output_buffer_size);
        input_buffer = Vec::with_capacity(input_buffer_size);
    }

    // If a check file was specified, then open it.
    let mut check_state = if let Some(ref check_fn) = check_filename {
        let check_path = Path::new(check_fn);
        let check_file_raw = io::File::open(&check_path).unwrap();
        // let check_file = io::BufferedReader::new(check_file_raw);
        Some(CheckFileState {
            reader: check_file_raw
        })
    }
    else {
        // println!("no check file specified.");
        None
    };
    let mut check_buffer: Vec<u8> = Vec::new();

    // Allocate output buffer
    let mut output_buffer: Vec<u8> = Vec::with_capacity(output_buffer_size);
    output_buffer.extend(repeat(0).take(output_buffer_size));

    let out_data = output_buffer.as_mut_slice();

    let mut state = Inflater::new_gzip();
    let mut cycle: uint = 0;

    for iter in range(0, iter_count) {
        if verbose {
            println!("starting iteration #{}", iter);
        }

        // This is the decode loop for an entire file.
        input_file.seek(0, io::SeekSet).unwrap();
        state.reset();

        if let Some(ref mut cs) = check_state {
            cs.reader.seek(0, io::SeekSet).unwrap();
        }
        check_buffer.clear();

        let mut input_eof = false;
        let mut total_in: u64 = 0;
        let mut total_out: u64 = 0;

        let mut input_pos: uint = 0;

        loop {
            if verbose {
                println!("cycle = {}", cycle);
            }

            // Load more input data, if necessary.
            if input_pos == input_buffer.len() && !input_eof && !read_entire_file {
                // println!("input buffer is empty; loading data");
                input_buffer.clear();
                input_pos = 0;
                match input_file.push(input_buffer_size, &mut input_buffer) {
                    Ok(bytes_read) => {
                        if verbose {
                            println!("zlibtest: loaded {} input bytes", bytes_read);
                        }
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

            if verbose {
                println!("calling inflate, cycle = {}, input_pos = {}, input_buffer.len = {}", cycle, input_pos, input_buffer.len());
            }

            match state.inflate(None, input_buffer.slice_from(input_pos), out_data) {
                InflateResult::Eof(_) => {
                    if verbose {
                        println!("zlib says Z_STREAM_END");
                    }
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
                    if verbose {
                        println!("zlibtest: cycle = {}, input_bytes_read = {}, output_bytes_written = {}", cycle, input_bytes_read, output_bytes_written);
                        println!("total_in = {}", total_in);
                        if verbose_print_blocks {
                            print_block(out_data.slice(0, output_bytes_written));
                        }
                    }

                    assert!(input_bytes_read + input_pos <= input_buffer.len());
                    input_pos += input_bytes_read;

                    // Check the data that we just received against the same data in the known-good file.
                    if output_bytes_written != 0 {
                        if let Some(ref mut cs) = check_state {
                	        assert!(check_buffer.len() == 0);

                            // read chunks from the check stream and verify them
                            let mut cpos = 0;
                            while cpos < output_bytes_written {
                                let clen_want = output_bytes_written - cpos;
                	            assert!(check_buffer.len() == 0);
                	            let clen_got = cs.reader.push(clen_want, &mut check_buffer).unwrap();
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
                    }

                    total_out += output_bytes_written as u64;
                }

                InflateResult::NeedInput => {
                    println!("NeedInput");
                    unimplemented!();
                }
            }

            cycle += 1;
        }
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
