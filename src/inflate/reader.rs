use std::cmp::max;
use std::io;
use std::io::{Reader, IoResult};

use WINDOW_BITS_DEFAULT;
use inflate::{Inflater,InflateResult};

/// Provides an implementation of `Reader` for inflating (decompression) INFLATE / GZIP streams.
pub struct InflateReader<R> {
    src: R,
    state: Inflater,

    inbuf: Vec<u8>,
    next_in: usize,

    /// Set true when 'src' reports EOF.
    src_eof: bool,
}

impl<R:Reader> InflateReader<R> {
    /// Creates a new InflateReader which uses `src` as its input stream.
    pub fn new_gzip(
        inbufsize: usize,
        src: R) -> InflateReader<R> {
        debug!("InflateReader::new_gzip()");
        InflateReader::new_with_inflater(inbufsize, Inflater::new_gzip(), src)
    }

    pub fn new_inflate(
        inbufsize: usize,
        src: R) -> InflateReader<R> {
        debug!("InflateReader::new_inflate()");
        InflateReader::new_with_inflater(inbufsize,
            Inflater::new_inflate(WINDOW_BITS_DEFAULT),
            src)
    }

    pub fn new_with_inflater(
        inbufsize: usize,
        inflater: Inflater,
        src: R) -> InflateReader<R> {
        let inbufsize = max(inbufsize, 0x1000);
        InflateReader {
            src: src,
            inbuf: Vec::with_capacity(inbufsize),
            next_in: 0,
            src_eof: false,
            state: inflater
        }        
    }

    pub fn inner(&mut self) -> &mut R {
        &mut self.src // self.src.deref_mut()
    }

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

impl<R:Reader> Reader for InflateReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let mut outpos: usize = 0;

        if buf.len() == 0 {
            // not really
            debug!("input buffer is zero-length!");
            return Err(io::standard_error(io::EndOfFile));
        }

        while outpos < buf.len() {
            debug!("outpos={} buf.len={}", outpos, buf.len());
            if self.next_in == self.inbuf.len() && !self.src_eof {
                match self.fill_buffer() {
                    Err(err) => {
                        debug!("fill_buffer() returned error: {}", err);
                        return Err(err);
                    }
                    Ok(()) => ()
                }
            }

            let inbuf = self.inbuf.slice_from(self.next_in);
            let buflen = buf.len();
            debug!("InflateReader: calling inflate, in_len={} out_len={}", inbuf.len(), buflen - outpos);
            match self.state.inflate(None, inbuf, buf.slice_from_mut(outpos)) {
                InflateResult::Decoded(in_bytes, out_bytes) => {
                    debug!("decoded: in_bytes={} out_bytes={}", in_bytes, out_bytes);
                    self.next_in += in_bytes;
                    outpos += out_bytes;
                }
                InflateResult::Eof(_) => {
                    if outpos == 0 {
                        debug!("inflater says EOF, no data transferred, returning EOF error");
                        return Err(io::standard_error(io::EndOfFile));
                    }
                    else {
                        debug!("inflater says EOF, some data transferred, returning that count");
                        return Ok(outpos)
                    }
                }
                InflateResult::InvalidData => {
                    warn!("InflateResult::InvalidData");
                    return Err(io::standard_error(io::InvalidInput))
                }
                InflateResult::NeedInput => {
                    warn!("InflateResult::NeedInput");
                    break;
                }
            }
        }

        if outpos == 0 {
            debug!("end of loop, no data transferred, returning EOF");
            Err(io::standard_error(io::EndOfFile))
        }
        else {
            debug!("end of loop, some data transfered, returning {}", outpos);
            Ok(outpos)
        }
    }
}
