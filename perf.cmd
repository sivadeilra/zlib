@echo off

setlocal
set RUST_LOG=

echo Building Rust impl
cargo build --verbose --jobs 1 
if errorlevel 1 exit /b

rem echo Running C zlib
rem x64\debug\zlibtest.exe tests\hamlet.tar.gz >reftrace.txt 2>&1
rem if errorlevel 1 exit /b

echo Running Rust zlib
set RUST_LOG=
rem target\rs_inflate_perf.exe zlib-1.2.8.tar.gz >ptrace.txt 2>&1
target\rs_inflate_perf.exe tests\hamlet.tar.gz >ptrace.txt 2>&1
if errorlevel 1 (echo rs_inflate_perf failed & exit /b 1)
