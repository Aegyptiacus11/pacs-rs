# CMake toolchain file for Windows builds with `-C target-feature=+crt-static`.
# CharLS (built by the charls-sys crate) declares CMake policies up to 3.25, so CMake picks the
# MSVC runtime itself (CMP0091) and ignores the /MT flag the cmake crate passes: it links the
# DLL runtime (MSVCRTD in debug), and the link fails on __imp__CrtDbgReport. Rust links the
# static release runtime in every profile, so CharLS must use that same one.
# Used through the CMAKE_TOOLCHAIN_FILE environment variable, which the cmake crate forwards.
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded")
