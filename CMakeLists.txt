cmake_minimum_required(VERSION 3.20)

project(trex-guard LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_executable(trex-guard
    src/main.cpp
)

target_link_libraries(trex-guard PRIVATE util)
