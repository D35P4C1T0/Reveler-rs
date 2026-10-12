cmake_minimum_required(VERSION 3.24)
project(RevelerRSAU VERSION 0.1.0 LANGUAGES C CXX OBJC OBJCXX)
if(NOT APPLE)
    message(FATAL_ERROR "Audio Units can only be built on macOS")
endif()
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CLAP_BUNDLE "" CACHE PATH "Absolute path to the already built Reveler-rs.clap bundle")
if(NOT EXISTS "${CLAP_BUNDLE}/Contents/Info.plist")
    message(FATAL_ERROR "Set CLAP_BUNDLE to a built Reveler-rs.clap bundle")
endif()
set(CLAP_WRAPPER_DOWNLOAD_DEPENDENCIES ON CACHE BOOL "Fetch pinned SDK dependencies")
set(CLAP_WRAPPER_COPY_AFTER_BUILD OFF CACHE BOOL "Do not install into the user's plugin directory")
include(FetchContent)
FetchContent_Declare(clap_wrapper
    GIT_REPOSITORY https://github.com/free-audio/clap-wrapper.git
    GIT_TAG 1cca996e96f29ab2be7ae9f8cfe532bbc92e1dd6 # v0.16.0
    GIT_SHALLOW FALSE
)
FetchContent_MakeAvailable(clap_wrapper)
add_library(reveler_au MODULE)
target_add_auv2_wrapper(
    TARGET reveler_au
    OUTPUT_NAME "Reveler-rs"
    BUNDLE_IDENTIFIER "org.reveler-rs.loudness-leveler.auv2"
    BUNDLE_VERSION "${PROJECT_VERSION}"
    MANUFACTURER_NAME "Reveler-rs Project"
    MANUFACTURER_CODE "RvRs"
    SUBTYPE_CODE "Rv01"
    INSTRUMENT_TYPE "aufx"
    MACOS_EMBEDDED_CLAP_LOCATION "${CLAP_BUNDLE}"
)
