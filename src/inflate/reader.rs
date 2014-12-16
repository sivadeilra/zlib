use std::io;
use std::io::{Reader, IoResult};
use inflate::{InflateState,InflateResult};
use ZStream;
use WINDOW_BITS_DEFAULT;
use std::cmp::max;

pub struct InflateReader
{
    src: Box<Reader + 'static>,
    state: InflateState,
    strm: ZStream,

    inbuf: Vec<u8>,
    avail_in: uint,
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
            inbuf: {
                let mut v: Vec<u8> = Vec::with_capacity(inbufsize);
                v.grow(inbufsize, 0u8);
                v
            },
            avail_in: 0,
            next_in: 0,
            src_eof: false,
            state: InflateState::new(WINDOW_BITS_DEFAULT, wrap),
            strm: ZStream::new(),
        }
    }
}

impl InflateReader {
    fn fill_buffer(&mut self) -> IoResult<()> {
        let buf = self.inbuf.as_mut_slice();
        let result = self.src.read(buf);
        match result {
            Ok(count) => {
                self.avail_in = count;
                self.next_in = 0;
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
            if self.avail_in == 0 && !self.src_eof {
                match self.fill_buffer() {
                    Err(err) => { return  Err(err); }
                    Ok(_) => ()
                }
            }

            let inbuf = self.inbuf.slice(self.next_in, self.avail_in);

            let mut strm: ZStream = ZStream::new();
            strm.next_out = 0;
            strm.avail_out = buf.len() - outpos;

            let buflen = buf.len();
            match self.state.inflate(&mut strm, None, inbuf, buf.slice_mut(outpos, buflen)) {
                InflateResult::Decoded(in_bytes, out_bytes) => {
                    self.avail_in -= in_bytes;
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
