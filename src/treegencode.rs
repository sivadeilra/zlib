#![feature(globs)]
extern crate core;

use std::io;
use std::os;
// use treedefs::*;
use treegen::*;

mod treegen;
mod treedefs;

fn write_array<T : core::fmt::Show>(s :&mut String, columns :uint, data :&[T], name: &str, dtype: &str)
{
    s.push_str(format!("pub static {} :[{}; {}] = [\n", name, dtype, data.len()).as_slice());
    for i in range(0, data.len()) {
        s.push_str(format!("{:4}", data[i]).as_slice());
        let last = i == data.len() - 1;
        if !last {
            s.push_str(",");
        }
        if last || (i % columns) == (columns - 1) {
            s.push_str("\n");
        }
    }
    s.push_str("];\n\n");
}


fn gen_trees_header() -> String {
    let st = tr_static_init();

    let mut s :String = String::new();

    s.push_str("// DO NOT EDIT -- THIS IS A GENERATED SOURCE FILE\n");
    s.push_str("// source file created automatically by treegencode.rs\n\n");

    write_array(&mut s, 10, &st.static_ltree_lengths, "STATIC_LTREE_LENGTHS", "u8");
    write_array(&mut s, 10, &st.static_ltree_codes, "STATIC_LTREE_CODES", "u16");
    write_array(&mut s, 10, &st.static_dtree_lengths, "STATIC_DTREE_LENGTHS", "u8");
    write_array(&mut s, 10, &st.static_dtree_codes, "STATIC_DTREE_CODES", "u16");
    write_array(&mut s, 10, &st.dist_code, "DIST_CODE", "u8");
    write_array(&mut s, 10, &st.length_code, "LENGTH_CODE", "u8");
    write_array(&mut s, 10, &st.base_length, "BASE_LENGTH", "u8");
    write_array(&mut s, 10, &st.base_dist, "BASE_DIST", "u16");

    s
}

fn main() {
    let args = os::args();
    if args.len() != 2 {
        println!("Please specify the output filename.");
        return;
    }

    let out_filename = &args[1];
    let out_path = Path::new(out_filename);
    let mut out_file = io::File::create(&out_path);
    let out_code = gen_trees_header();
    out_file.write_str(out_code.as_slice()).unwrap();
}
