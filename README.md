zlib
====

This is an incomplete port of the well-known zlib library to the Rust language.

There are several reasons for porting zlib to Rust:

    * To provide an implementation of a useful, common library.  zlib is used in many
      applications and libraries.

    * To provide a real-world example of a performance-sensitive component, as a way
      of validating the performance objectives of Rust.  Rust aims to provide a safe
      and efficient programming language.  If a version of zlib can be built that is
      safe and efficient, then the community gains a safe, efficient zlib and Rust
      gains a new piece of real-world example code.

# Functionality

The first goal (functionality) has largely been met.  The decompressor works, although
it does have a few limitations.  The decompressor can be used in two ways.  First, you
can use it directly, by creating an instance of the  `Inflater` struct.  The `Inflater`
struct provides an API that is very similar to the zlib `inflate()` API.  The main
difference is that `Inflater` uses slices instead of raw pointers.  Also, the `z_stream`
abstraction has been removed entirely.  The application using `Inflater` simply calls
`Inflater::inflate()` as many times as is necessary in order to decompress all of the data.

You can also use the `InflateReader` struct.  This struct implements the `Reader` trait,
and so you can easily insert a zlib decompressor into a pipeline of `Reader`-based code.

# Performance

The performance goals have not yet been reached.  There are several reasons for that:

* This is brand-new code.  I mean the ported code is new; obviously zlib has been
  around for a very long time.  But this code is new, and I have not yet begun to
  do any serious performance optimization.  The C-based zlib has been optimized
  over many years.

* Rust does not support `goto`.  The C zlib relies heavily on `goto` to implement
  an efficient resumable state machine.  To port this code, I used an `enum` to
  simulate the `goto`-based state machine.  This causes direct jump instructions
  to be replaced with variable stores / loads and an indirect table-based job
  (if you're lucky) or an if/else ladder (if you're not lucky).  I speculate that
  this is the source of the main difference in performance.

* Bounds checking.  Bounds checking *can* be done efficiently, mainly by hoisting
  bounds checks above loops.  That is, a well-written inner loop can provide
  enough information to a compiler to allow the compiler to perform a single
  bounds check at the start of the loop, rather than checking bounds on every
  iteration of a loop.  (Microsoft's C# / CLR does a decent job on bounds-check
  elimination and hoisting, for example.)  Bounds-check elimination and hoisting
  in Rust/LLVM is weak to non-existent.  It is a known deficiency in Rust, and it
  is one that will certainly be addressed in time.  LLVM evolved to support the
  needs of languages that do not require bounds checks (such as C++); LLVM and
  Rust will need to implement existing well-known algorithsm for bounds-check
  elimination and hoisting in order to provide competitive performance.

* Goofs on my part during the port.  It is entirely possible that I broke
  something that affects performance when I ported the code from C to Rust.

* Miscellaneous bad code-gen from Rust.  Rust is a new language; it will take
  time for Rust to reach the same level of high-quality code generation as
  in existing systems programming languages, such as C/C++.  I have confidence
  that Rust will get there.  In fact, the purpose of this experiment with
  porting zlib is to provide a useful piece of code for optimizing Rust *itself*.

# License

zlib is a well-known, universally-used open source algorithm.  I hereby contribute
my work on porting zlib to Rust to the open source community, using the permissive
MIT license.  I do not claim any rights to zlib itself whatsoever!  I only did the
port to Rust.  Also, I disclaim any warrantee on the Rust port.  If you use it for
any purpose whatsoever, then you do so at your own risk.  I believe this code is a
faithful port of the C code, but at the same time I am doing this solely as a side
project.

This is the copyright from the original zlib README file:

     (C) 1995-2013 Jean-loup Gailly and Mark Adler

      This software is provided 'as-is', without any express or implied
      warranty.  In no event will the authors be held liable for any damages
      arising from the use of this software.

      Permission is granted to anyone to use this software for any purpose,
      including commercial applications, and to alter it and redistribute it
      freely, subject to the following restrictions:

      1. The origin of this software must not be misrepresented; you must not
         claim that you wrote the original software. If you use this software
         in a product, an acknowledgment in the product documentation would be
         appreciated but is not required.
      2. Altered source versions must be plainly marked as such, and must not be
         misrepresented as being the original software.
      3. This notice may not be removed or altered from any source distribution.

      Jean-loup Gailly        Mark Adler
      jloup@gzip.org          madler@alumni.caltech.edu

I (the author of the Rust port) grant everyone similar rights to the Rust port
of zlib, and I also similarly disclaim any liability for damages arising from
the use of this software.

# Feedback

I welcome feedback!  Feel free to contact me through Github at
https://github.com/sivadeilra.  Also, if you see any bugs / problems in this
code, feel free to simply open an issue on Github at https://github.com/sivadeilra/zlib .

If your feedback is "wow, this is slow!" -- yeah, I know that already.  If you
are interested in helping with performance analysis and improvement, then feel
free to contact me.  If you just want to let me know that this work sucks and
I should never have bothered in the first place -- just keep it to yourself, 
thanks.
