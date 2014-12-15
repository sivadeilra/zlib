rustports
=========

Ports of common open source libraries to the Rust language

As an exercise in learning and evaluating Rust, I intend to port several common open source libraries to Rust.  I have several goals for this work:

* Evaluate the performance of well-known algorithms, such as JPEG, GZIP/DEFLATE, etc., in Rust.  Rust aims at providing both safety and performance, and I want to see how far I can take this language on both.

* Evaluate other correctness gains in algorithms, due to Rust.  Rust does not permit NULL pointers, for example.  How easy is it to write code without NULL pointers?  How many defects does this prevent?  Etc.

* Is it fun?  Rust looks like a fun language.  Will it still be fun after writing tens of KLOC in it?
  
I intend to choose common, well-known open source libraries.  I want to choose well-known libraries, such as JPEG decoding, because they are useful and because their performance has already been evaluated and improved many times.

I intend to respect the letter and intent of all open source licenses.  I won't work with projects that do not permit derivative works.  I am willing to work with LGPL or GPL code, since I intend to release the results of the ports for all to use.  I am also perfectly willing to work with MIT / BSD / Apache licenses.

Another goal of mine is to contribute highly reliable, safe implementations of these languages to other projects that want to use them.  For example, if a JPEG decoder can be implemented that cannot violate memory safety, and which has performance equivalent to the original C implementation, then this should be available to the community for adoption in browsers, imaging toolkits, etc.

