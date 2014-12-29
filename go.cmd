@echo off

setlocal

rem set _input=tests\hamlet.tar.gz
set _input=zlib-1.2.8.tar.gz

echo Building Rust impl
cargo build --verbose --jobs 1
if errorlevel 1 exit /b

echo Running C zlib
x64\debug\zlibtest.exe -v %_input% >reftrace.txt 2>&1
if errorlevel 1 exit /b

echo Running Rust zlib
set RUST_LOG=debug
target\ztrace.exe -v %_input% >ztrace-raw.txt 2>&1 

echo Filtering debug prefixes out of ztrace-raw.txt
perl filter-ztrace.pl < ztrace-raw.txt > ztrace.txt

rem echo Diffing
rem start windiff reftrace.txt ztrace.txt
perl firstdiff.pl > fd
echo Read 'fd' file for awesomeness
