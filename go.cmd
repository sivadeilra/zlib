@echo off

setlocal
set RUST_LOG=

rem set _input=tests\hamlet.tar.gz
set _input=zlib-1.2.8.tar.gz

echo Building Rust impl
cargo build --verbose --jobs 1
if errorlevel 1 exit /b

echo Running C zlib
x64\debug\zlibtest.exe -ib:64 -ob:64 -v %_input% >reftrace.txt 2>&1
if errorlevel 1 exit /b

echo Running Rust zlib
set RUST_LOG=debug
target\ztrace.exe -v -vv -ib:64 -ob:64 %_input% 2>&1 | perl filter-ztrace.pl > ztrace.txt

rem echo Diffing
rem start windiff reftrace.txt ztrace.txt
perl firstdiff.pl >fd
echo Read 'fd' file for awesomeness
