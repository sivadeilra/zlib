@echo off

setlocal

echo Building Rust impl
cargo build --verbose --jobs 1
if errorlevel 1 exit /b

echo Running C zlib
debug\zlibtest.exe tests\hamlet.tar.gz >reftrace.txt 2>&1
if errorlevel 1 exit /b

echo Running Rust zlib
set RUST_LOG=debug
target\ztrace.exe >ztrace-raw.txt 2>&1 

echo Filtering debug prefixes out of ztrace-raw.txt
perl filter-ztrace.pl < ztrace-raw.txt > ztrace.txt


rem echo Diffing
rem start windiff reftrace.txt ztrace.txt

perl firstdiff.pl
