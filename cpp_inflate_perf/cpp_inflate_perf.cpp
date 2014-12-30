// cpp_inflate_perf.cpp : Defines the entry point for the console application.
//

#include "stdafx.h"

static const long long MAX_FILE_SIZE = 1 << 30; // 1 GB


class Stopwatch {
public:
    LARGE_INTEGER cpu_time_start;
    LARGE_INTEGER cpu_time_end;
    LARGE_INTEGER cpu_resolution;

    Stopwatch() {
        QueryPerformanceFrequency(&this->cpu_resolution);
        cpu_time_start.QuadPart = 0;
        cpu_time_end.QuadPart = 0;
    }

    void start() {
        QueryPerformanceCounter(&cpu_time_start);
        cpu_time_end.QuadPart = 0;
    }

    void stop() {
        QueryPerformanceCounter(&cpu_time_end);
    }
};


int main(int argc, char* argv[])
{
    if (argc != 2) {
        fprintf(stderr, "expected input filename\n");
        return 1;
    }

    auto filename = argv[1];

    int iter_count = 10;
    int input_buffer_size = 0x10000;
    int output_buffer_size = 1 << 20;

    FILE* f = fopen(filename, "rb");
    if (f == nullptr) {
        fprintf(stderr, "failed to open input file\n");
        return 1;
    }

    fseek(f, 0, SEEK_END);
    long long file_len_64 = _ftelli64(f);

    fprintf(stderr, "file_len = %I64d\n", file_len_64);

    if (file_len_64 > MAX_FILE_SIZE) {
        fprintf(stderr, "file is way too big!\n");
        return 1;
    }

    int file_len = (int)file_len_64;

    fseek(f, 0, SEEK_SET);
    uint8_t* input_data = new uint8_t[file_len];
    size_t all_bytes_read = fread(input_data, 1, file_len, f);
    if (all_bytes_read < file_len) {
        fprintf(stderr, "failed to read all input bytes.\n");
        return 1;
    }

    uint8_t* inbuf = new uint8_t[input_buffer_size];
    uint8_t* outbuf = new uint8_t[output_buffer_size];

    Stopwatch watch;

    fprintf(stderr, "starting...\n");

    int inbuf_pos = 0;          // position within inbuf of next read
    int inbuf_length = 0;         // number of bytes in inbuf
    bool inbuf_eof = false;

    z_stream strm;
    memset(&strm, 0, sizeof(z_stream));
    int err = inflateInit2(&strm, 0x20 | MAX_WBITS); // 0x20 means "use gzip header"
    assert(err == Z_OK);

    for (int iter = 0; iter < iter_count; ++iter) {
        fseek(f, 0, SEEK_SET);

        inflateReset(&strm);

        watch.start();

        strm.next_in = input_data;
        strm.avail_in = file_len;

        while (true) {

#if 0
            if (inbuf_pos == inbuf_length && !inbuf_eof) {
                size_t bytes_read = fread(inbuf, 1, input_buffer_size, f);
                inbuf_length = (int)bytes_read;
                inbuf_pos = 0;
                if (bytes_read == 0) {
                    inbuf_eof = true;
                }
            }
#endif

            // strm.next_in = &inbuf[inbuf_pos];
            // strm.avail_in = inbuf_length - inbuf_pos;

            strm.next_out = outbuf;
            strm.avail_out = output_buffer_size;

            int zerr = inflate(&strm, 0);

            if (zerr == Z_OK) {
                // good!
                // int new_inbuf_pos = (int)(strm.next_in - inbuf);
                // inbuf_pos = new_inbuf_pos;
            }
            else if (zerr == Z_STREAM_END) {
                // fprintf(stderr, "Z_STREAM_END\n");
                break;
            }
            else {
                fprintf(stderr, "inflate() returned error: %d %s\n", zerr, strm.msg);
                return 1;
            }
        }    
    
        watch.stop();

        fprintf(stderr, "iteration #%d done.  cycles: %I64u\n", iter, watch.cpu_time_end.QuadPart - watch.cpu_time_start.QuadPart);
    }

	return 0;
}

