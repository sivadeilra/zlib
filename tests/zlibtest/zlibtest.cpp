// zlibtest.cpp : Defines the entry point for the console application.
//

#include "stdafx.h"

typedef unsigned __int8 BYTE;

typedef std::basic_string<char> string;

#if DEBUG
extern "C" int z_verbose;
#endif

void print_block(BYTE* data, int len)
{
    static const char hex[] = "0123456789abcdef";

    fprintf(stderr, "print_block: len=%d\n", len);

    int width = 32;

    for (int pos = 0; pos < len; ++pos) {
        BYTE b = data[pos];
        fprintf(stderr, " %c%c", hex[b >> 4], hex[b & 0xf]);

        if (((pos + 1) % width) == 0) {
            fprintf(stderr, "\n");
        }
    }

    if ((len % width) != 0) {
        fprintf(stderr, "\n");
    }
}

void usage()
{
    fprintf(stderr, "\nusage: zlibtest <input-file-path>\n"
        "\n"
        "    -v            enable verbose logging in zlib\n"
        "    -i:<nnn>      set iteration count; default is 1\n"
        "    -ib:<nnn>     set size of input buffer\n"
        "    -ob:<nnn>     set size of output buffer\n"
        );
}

static const int DEFAULT_INPUT_BUFFER_SIZE = 0x10000;
static const int DEFAULT_OUTPUT_BUFFER_SIZE = 0x10000;


int main(int argc, char* argv[])
{
    string filename;

    int iter_count = 1;
    int input_buffer_size = DEFAULT_INPUT_BUFFER_SIZE;
    int output_buffer_size = DEFAULT_OUTPUT_BUFFER_SIZE;
    bool read_entire_file = false;
    bool verbose = false;

    if (argc == 1) {
        usage();
        return 1;
    }

    for (int i = 1; i < argc; ++i) {
        string arg = argv[i];
        if (arg.length() == 0) {
            continue;
        }
        if (arg[0] == '-') {
            int pos = (int)arg.find_first_of(':', 1);

            string name = (pos < 0) ? arg.substr(1) : arg.substr(1, pos - 1);
            string value = (pos < 0) ? string() : arg.substr(pos + 1);

            if (name == "i") {
                iter_count = atoi(value.c_str());
                if (iter_count == 0) {
                    printf("invalid iteration count.\n");
                    return 1;
                }
            }
            else if (name == "v") {
#if DEBUG
                verbose = true;
                z_verbose = 2;
#else
                fprintf(stderr, "warning: -v is ignored in 'release' builds.\n");
#endif
            }
            else if (name == "ib") {
                input_buffer_size = atoi(value.c_str());
                if (input_buffer_size <= 0) {
                    fprintf(stderr, "invalid input buffer size.\n");
                    return 1;
                }
            }
            else if (name == "ob") {
                output_buffer_size = atoi(value.c_str());
                if (output_buffer_size <= 0) {
                    fprintf(stderr, "invalid output buffer size.\n");
                    return 1;
                }
            }
            else if (name == "F") {
                read_entire_file = true;
            }
            else {
                fprintf(stderr, "invalid argument: %s\n", arg);
                return 1;
            }           
        }
        else {
            if (filename.length() != 0) {
                fprintf(stderr, "error: input filename specified more than once.\n");
                usage();
                return 1;
            }
            filename = arg;
        }
    }

    if (filename.length() == 0) {
        fprintf(stderr, "error: input filename was not specified.\n");
        usage();
        return 1;
    }

    FILE* f;
    errno_t ferr = fopen_s(&f, filename.c_str(), "rb");
    if (ferr != 0) {
        printf("failed to open input file\n");
        return 1;
    }

    // Allocate and initialize the decompressor.

    z_stream strm;
    memset(&strm, 0, sizeof(z_stream));

    int err = inflateInit2(&strm, 0x20 | MAX_WBITS); // 0x20 means "use gzip header"
    assert(err == Z_OK);

    BYTE* input_buffer = new BYTE[input_buffer_size];

    int input_pos = 0;

    if (read_entire_file) {
        fseek(f, 0, SEEK_END);
        int file_size = ftell(f);
        fprintf(stderr, "input file size: %d\n", file_size);

        input_buffer_size = file_size;

        if (file_size == 0) {
            fprintf(stderr, "input file is empty!\n");
            return 1;
        }

        input_buffer = new BYTE[file_size];

        fseek(f, 0, SEEK_SET);
        int inpos = 0;
        while (inpos < file_size) {
            size_t bytes_read = fread(&input_buffer[inpos], 1, file_size - inpos, f);
            if (bytes_read == 0) {
                fprintf(stderr, "error: failed to read all data for input file\n");
                return 1;
            }
            inpos += (int)bytes_read;
        }

        strm.next_in = &input_buffer[0];
        strm.avail_in = file_size;
    }
    else {
        fprintf(stderr, "using buffered mode.\n");
        fprintf(stderr, "    input buffer size: 0x%x %d\n", input_buffer_size, input_buffer_size);
        fprintf(stderr, "    output buffer size: 0x%x %d\n", output_buffer_size, output_buffer_size);
        input_buffer = new BYTE[input_buffer_size];

        strm.next_in = input_buffer;
        strm.avail_in = 0;

        input_pos = 0;
    }

    // Allocate the output buffer.  We will write data into this buffer, but ignore it.
    BYTE* output_buffer = new BYTE[output_buffer_size];

    // Used only for buffered mode.
    bool input_eof = false;

    int cycle = 0;

    size_t inbuf_valid_len = 0;

    for (int iter = 0; iter < iter_count; ++iter) {
        fprintf(stderr, "starting iteration #%d\n", iter);

        fseek(f, 0, SEEK_SET);

        inflateReset(&strm);

        while (true) {
            if (verbose) {
                fprintf(stderr, "cycle = %d\n", cycle);
            }

            if (strm.avail_in == 0 && !input_eof && !read_entire_file) {
                input_pos = 0;
                strm.next_in = input_buffer;
                size_t bytes_read = fread(input_buffer, 1, input_buffer_size, f);
                if (bytes_read == 0) {
                    fprintf(stderr, "input stream EOF\n");
                    input_eof = true;
                }
                else {
                    if (verbose) {
                        fprintf(stderr, "zlibtest: loaded %d input bytes\n", (int)bytes_read);
                    }
                    strm.avail_in = (int)bytes_read;
                    inbuf_valid_len = bytes_read;
                }
            }

            strm.next_out = output_buffer;
            strm.avail_out = output_buffer_size;

            BYTE* old_next_in = strm.next_in;

            if (verbose) {
                fprintf(stderr, "calling inflate, cycle = %d, input_pos = %d, input_buffer.len = %d\n", cycle, (int)(strm.next_in - input_buffer), inbuf_valid_len);
            }
            err = inflate(&strm, /*flush*/0);

            if (err == Z_OK || err == Z_STREAM_END) {
                if (verbose) {
                    int input_bytes_read = (int)(strm.next_in - old_next_in);
                    int output_bytes_written = (int)(strm.next_out - output_buffer);
                    fprintf(stderr, "zlibtest: cycle = %d, input_bytes_read = %d, output_bytes_written = %d\n", cycle, input_bytes_read, output_bytes_written);
                    fprintf(stderr, "total_in = %d\n", strm.total_in);
                    print_block(output_buffer, output_bytes_written);
                }

                if (err == Z_STREAM_END) {
                    if (verbose) {
                        fprintf(stderr, "zlib says Z_STREAM_END\n");
                    }
                    break;
                }
            }
            else if (err == Z_STREAM_ERROR) {
                fprintf(stderr, "oh no, Z_STREAM_ERROR\n");
                break;
            }
            else if (err == Z_DATA_ERROR) {
                fprintf(stderr, "oh no, Z_DATA_ERROR\n");
                break;
            }
            else {
                fprintf(stderr, "zerr is unrecognized: %d\n", err);
                break;
            }

            cycle++;
        }
    }

    fclose(f);
    
    return 0;
}

