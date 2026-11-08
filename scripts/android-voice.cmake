# transcribe-cpp-sys 0.4.1's static link manifest treats Android as desktop Linux:
# it asks Rust to link libstdc++ and libpthread, neither of which supplies Android's
# C++ runtime. Fix only the generated manifest, leaving the registry sources intact.
# Deferred until the root project has written the manifest; install then copies it.
#
# The runtime archives are copied into a directory of their own: the -sys build script turns
# each path into a link search dir, and the NDK's own sysroot dir also holds libc.a/libm.a/
# libdl.a, which the linker then picks over libc.so (a static libc crashes the app on launch).
if(ANDROID AND CMAKE_CURRENT_SOURCE_DIR STREQUAL CMAKE_SOURCE_DIR)
    function(oah_android_voice_manifest)
        set(manifest "${CMAKE_BINARY_DIR}/transcribe-link.json")
        if(NOT EXISTS "${manifest}")
            return()
        endif()
        file(READ "${manifest}" json)
        string(JSON shared GET "${json}" shared)
        if(shared)
            message(FATAL_ERROR "oah Android voice expects a static transcribe.cpp build")
        endif()
        string(JSON json SET "${json}" system_libs "[\"m\", \"dl\", \"log\"]")
        set(compiler_args)
        if(CMAKE_CXX_COMPILER_TARGET)
            list(APPEND compiler_args "--target=${CMAKE_CXX_COMPILER_TARGET}")
        endif()
        if(CMAKE_SYSROOT)
            list(APPEND compiler_args "--sysroot=${CMAKE_SYSROOT}")
        endif()
        set(runtime "${CMAKE_BINARY_DIR}/oah-android-runtime")
        file(MAKE_DIRECTORY "${runtime}")
        foreach(lib libc++_static.a libc++abi.a libunwind.a)
            execute_process(COMMAND "${CMAKE_CXX_COMPILER}" ${compiler_args} "-print-file-name=${lib}"
                OUTPUT_VARIABLE path OUTPUT_STRIP_TRAILING_WHITESPACE RESULT_VARIABLE result)
            if(NOT result EQUAL 0 OR NOT IS_ABSOLUTE "${path}" OR NOT EXISTS "${path}")
                message(FATAL_ERROR "Could not locate Android runtime ${lib}: ${path}")
            endif()
            file(COPY "${path}" DESTINATION "${runtime}")
            string(JSON count LENGTH "${json}" library_paths)
            string(JSON json SET "${json}" library_paths ${count} "\"${runtime}/${lib}\"")
        endforeach()
        file(WRITE "${manifest}" "${json}\n")
    endfunction()
    cmake_language(DEFER CALL oah_android_voice_manifest)
endif()
