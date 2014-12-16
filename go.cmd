@echo off

setlocal

echo Running C zlib
debug\cpp_test.exe tests\hamlet.tar.gz reftrace.txt

echo Running Rust zlib
set RUST_LOG=debug
target\ztrace.exe > ztrace-raw.txt 2>&1 

echo Filtering debug prefixes out of ztrace-raw.txt
perl filter-ztrace.pl < ztrace-raw.txt > ztrace.txt


echo Diffing
windiff reftrace.txt ztrace.txt
