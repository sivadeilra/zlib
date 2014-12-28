use std::io;
use std::io::{Reader, IoResult};
use inflate::{InflateState,InflateResult};
use ZStream;
use WINDOW_BITS_DEFAULT;
use std::cmp::max;

/// Provides an implementation of `Reader` for inflating (decompression) INFLATE / GZIP streams.
pub struct InflateReader
{
    src: Box<Reader + 'static>,
    state: InflateState,
    strm: ZStream,

    inbuf: Vec<u8>,
    next_in: uint,

    /// Set true when 'src' reports EOF.
    src_eof: bool,
}

impl InflateReader
{
    /// Creates a new InflateReader which uses `src` as its input stream.
    pub fn new(
        inbufsize: uint,
        wrap: u32,
        src: Box<Reader + 'static>) -> InflateReader
    {
        let inbufsize = max(inbufsize, 0x1000);

        InflateReader {
            src: src,
            inbuf: Vec::with_capacity(inbufsize),
            next_in: 0,
            src_eof: false,
            state: InflateState::new(WINDOW_BITS_DEFAULT, wrap),
            strm: ZStream::new(),
        }
    }
}

impl InflateReader {
    fn fill_buffer(&mut self) -> IoResult<()> {
        self.inbuf.clear();
        let result = self.src.push(self.inbuf.capacity(), &mut self.inbuf);
        match result {
            Ok(count) => {
                self.next_in = 0;
                debug!("next_in=0 inbuf.len()={}", self.inbuf.len());
                Ok(())
            }
            Err(err) => {
                self.src_eof = true;
                Err(err)
            }
        }
    }

    pub fn inner_mut<'a>(&'a mut self) -> &'a InflateState
    {
        &mut self.state
    }
}

impl Reader for InflateReader {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<uint> {
        let mut outpos: uint = 0;

        if buf.len() == 0 {
            // not really
            // Err::<uint>(io::standard_error(io::EndOfFile));
            panic!();
        }

        while outpos < buf.len() {
            if self.next_in == self.inbuf.len() && !self.src_eof {
                match self.fill_buffer() {
                    Err(err) => {
                        // TODO: if err is EOF, then return Ok(outpos), not an error.
                        return  Err(err);
                    }
                    Ok(_) => ()
                }
            }

            let inbuf = self.inbuf.slice(self.next_in, self.inbuf.len());
            let buflen = buf.len();
            debug!("InflateReader: calling inflate, in_len={} out_len={}", inbuf.len(), buflen - outpos);
            match self.state.inflate(&mut self.strm, None, inbuf, buf.slice_mut(outpos, buflen)) {
                InflateResult::Decoded(in_bytes, out_bytes) => {
                    self.next_in += in_bytes;
                    outpos += out_bytes;
                }
                InflateResult::Eof(_) => {
                    return Err(io::standard_error(io::EndOfFile));
                }
                _ => {
                    unimplemented!();
                }
            }
        }

        if outpos == 0 {
            Err(io::standard_error(io::EndOfFile))
        }
        else {
            Ok(outpos)
        }
    }
}
