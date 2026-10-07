cmake_minimum_required(VERSION 3.20)

project(trex-guard LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include(GNUInstallDirs)

add_executable(trex-guard
    src/main.cpp
    src/rules.cpp
    src/config.cpp
)

target_link_libraries(trex-guard PRIVATE util)

enable_testing()

add_executable(test_rules
    tests/test_rules.cpp
    src/rules.cpp
    src/config.cpp
)

add_test(NAME test_rules COMMAND test_rules)

# Installation targets
install(TARGETS trex-guard
    DESTINATION ${CMAKE_INSTALL_BINDIR}
)

install(PROGRAMS scripts/tg
    DESTINATION ${CMAKE_INSTALL_BINDIR}
)

install(FILES
    examples/config.ini
    scripts/shell-integration.bash
    DESTINATION ${CMAKE_INSTALL_DATADIR}/trex-guard
)

# Uninstall target
configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/cmake/cmake_uninstall.cmake.in"
    "${CMAKE_CURRENT_BINARY_DIR}/cmake_uninstall.cmake"
    IMMEDIATE @ONLY
)

add_custom_target(uninstall
    COMMAND ${CMAKE_COMMAND} -P ${CMAKE_CURRENT_BINARY_DIR}/cmake_uninstall.cmake
)
