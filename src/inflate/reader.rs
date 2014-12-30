use std::cmp::max;
use std::io;
use std::io::{Reader, IoResult};

use inflate::{Inflater,InflateResult};

/// Provides an implementation of `Reader` for inflating (decompression) INFLATE / GZIP streams.
pub struct InflateReader {
    src: Box<Reader + 'static>,
    state: Inflater,

    inbuf: Vec<u8>,
    next_in: uint,

    /// Set true when 'src' reports EOF.
    src_eof: bool,
}

impl InflateReader
{
    /// Creates a new InflateReader which uses `src` as its input stream.
    pub fn new_gzip(
        inbufsize: uint,
        src: Box<Reader + 'static>) -> InflateReader {
        InflateReader::new_with_inflater(inbufsize, Inflater::new_gzip(), src)
    }

    pub fn new_with_inflater(
        inbufsize: uint,
        inflater: Inflater,
        src: Box<Reader + 'static>) -> InflateReader {
        let inbufsize = max(inbufsize, 0x1000);
        InflateReader {
            src: src,
            inbuf: Vec::with_capacity(inbufsize),
            next_in: 0,
            src_eof: false,
            state: Inflater::new_gzip()
        }        
    }
}

impl InflateReader {
    fn fill_buffer(&mut self) -> IoResult<()> {
        self.inbuf.clear();
        self.next_in = 0;
        let result = self.src.push(self.inbuf.capacity(), &mut self.inbuf);
        match result {
            Ok(count) => {
                Ok(())
            }
            // Err(err) if err.kind == io::EndOfFile {
            //     self.src_eof = true;
            // }
            Err(err) => {
                if err.kind == io::EndOfFile {
                    self.src_eof = true;
                    Ok(())
                }
                else {
                    self.src_eof = true;
                    Err(err)
                }
            }
        }
    }
}

impl Reader for InflateReader {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<uint> {
        let mut outpos: uint = 0;

        if buf.len() == 0 {
            // not really
            println!("input buffer is zero-length!");
            return Err(io::standard_error(io::EndOfFile));
        }

        while outpos < buf.len() {
            println!("outpos={} buf.len={}", outpos, buf.len());
            if self.next_in == self.inbuf.len() && !self.src_eof {
                match self.fill_buffer() {
                    Err(err) => {
                        println!("fill_buffer() returned error: {}", err);
                        return Err(err);
                    }
                    Ok(()) => ()
                }
            }

            let inbuf = self.inbuf.slice_from(self.next_in);
            let buflen = buf.len();
            println!("InflateReader: calling inflate, in_len={} out_len={}", inbuf.len(), buflen - outpos);
            match self.state.inflate(None, inbuf, buf.slice_from_mut(outpos)) {
                InflateResult::Decoded(in_bytes, out_bytes) => {
                    println!("decoded: in_bytes={} out_bytes={}", in_bytes, out_bytes);
                    self.next_in += in_bytes;
                    outpos += out_bytes;
                }
                InflateResult::Eof(_) => {
                    if outpos == 0 {
                        println!("inflater says EOF, no data transferred, returning EOF error");
                        return Err(io::standard_error(io::EndOfFile));
                    }
                    else {
                        println!("inflater says EOF, some data transferred, returning that count");
                        return Ok(outpos)
                    }
                }
                _ => {
                    unimplemented!();
                }
            }
        }

        if outpos == 0 {
            println!("end of loop, no data transferred, returning EOF");
            Err(io::standard_error(io::EndOfFile))
        }
        else {
            println!("end of loop, some data transfered, returning {}", outpos);
            Ok(outpos)
        }
    }
}
