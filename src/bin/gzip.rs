extern crate zlib;

use std::io;
use std::os;
use zlib::inflate::InflateReader;

fn main()
{
    let args = os::args();

    if args.len() != 3 {
        println!("usage: gzip <input> <output>");
        println!("note: this program is only a decompressor (gzip -d ...), not a compressor.");
        return;
    }

    let input_path = Path::new(&args[1]);
    let output_path = Path::new(&args[2]);

    let in_bufsize: uint = 1 << 22;
    let out_bufsize: uint = 1 << 22;

    // open compressed input file, create a decompressor for it
    let input_file = io::BufferedReader::new(io::File::open(&input_path).unwrap());
    let mut inflater = InflateReader::new_gzip(in_bufsize, box input_file);

    println!("opened input file");

    // open the output file
    let mut output_file = io::BufferedWriter::new(io::File::create(&output_path).unwrap());

    println!("opened output file");

    let mut buffer: Vec<u8> = Vec::with_capacity(out_bufsize);

    let mut total_out: u64 = 0;

    loop {
        match inflater.push(out_bufsize, &mut buffer) {
            Ok(chunk_bytes) => {
                output_file.write(buffer.as_slice()).unwrap();
                buffer.clear();
                total_out += chunk_bytes as u64;
            }
            Err(err) => {
                println!("push() returned error: {} {}", err.desc, err.detail);
                break;
            }
        }
    }

    println!("done.  wrote {} byte(s).", total_out);
}
