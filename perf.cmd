@echo off

setlocal
set RUST_LOG=
set _input=zlib-1.2.8.tar.gz

echo Building Rust impl
cargo build --verbose --jobs 1 --release %*
if errorlevel 1 exit /b

rem echo Running C zlib
rem x64\debug\zlibtest.exe tests\hamlet.tar.gz >reftrace.txt 2>&1
rem if errorlevel 1 exit /b

echo Running Rust zlib
set RUST_LOG=warn
target\release\ztrace.exe -F -i:100 -p %_input% >rs_perf.txt 2>&1
rem if errorlevel 1 (echo ztrace failed & exit /b 1)

type rs_perf.txt