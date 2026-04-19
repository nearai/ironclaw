cmake_minimum_required(VERSION 3.5)
set(CMAKE_MODULE_PATH ${CMAKE_MODULE_PATH} "${CMAKE_SOURCE_DIR}/CMakeModules/")
project(signal-protocol-c)

if(POLICY CMP0042)
	cmake_policy(SET CMP0042 NEW)
endif()

SET(SIGNAL_PROTOCOL_C_VERSION_MAJOR 2)
SET(SIGNAL_PROTOCOL_C_VERSION_MINOR 3)
SET(SIGNAL_PROTOCOL_C_VERSION_PATCH 2)
SET(SIGNAL_PROTOCOL_C_VERSION ${SIGNAL_PROTOCOL_C_VERSION_MAJOR}.${SIGNAL_PROTOCOL_C_VERSION_MINOR}.${SIGNAL_PROTOCOL_C_VERSION_PATCH})

SET(LIB_SUFFIX "" CACHE STRING "Define suffix of directory name (32/64)")
SET(BIN_INSTALL_DIR "${CMAKE_INSTALL_PREFIX}/bin" CACHE STRING "The directory the binaries are installed in")
SET(LIB_INSTALL_DIR "${CMAKE_INSTALL_PREFIX}/lib${LIB_SUFFIX}" CACHE STRING "The directory the libraries are installed in")
SET(INCLUDE_INSTALL_DIR "${CMAKE_INSTALL_PREFIX}/include" CACHE PATH "The directory the headers are installed in")
SET(INSTALL_PKGCONFIG_DIR "${LIB_INSTALL_DIR}/pkgconfig" CACHE PATH "Installation directory for pkgconfig (.pc) files")

INCLUDE(CheckSymbolExists)
INCLUDE(CheckCCompilerFlag)
INCLUDE(TestBigEndian)

CHECK_SYMBOL_EXISTS(memset_s "string.h" HAVE_MEMSET_S)

IF(CMAKE_SYSTEM_NAME MATCHES "Windows")
	CHECK_SYMBOL_EXISTS(SecureZeroMemory "Windows.h;WinBase.h" HAVE_SECUREZEROMEMORY)
ENDIF(CMAKE_SYSTEM_NAME MATCHES "Windows")

IF(BUILD_TESTING)
	enable_testing()
ENDIF(BUILD_TESTING)

IF(CMAKE_COMPILER_IS_GNUCC OR CMAKE_C_COMPILER_ID MATCHES "Clang")
	SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -fmessage-length=0 -Wall -Wmissing-field-initializers -Wno-missing-braces -Wparentheses")
ENDIF(CMAKE_COMPILER_IS_GNUCC OR CMAKE_C_COMPILER_ID MATCHES "Clang")

IF(CMAKE_COMPILER_IS_GNUCC)
	CHECK_C_COMPILER_FLAG("-Wsign-conversion" GCC_WARN_SIGN_CONVERSION)
	SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wsign-compare")
	IF(GCC_WARN_SIGN_CONVERSION)
		SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wsign-conversion")
	ENDIF(GCC_WARN_SIGN_CONVERSION)
ENDIF(CMAKE_COMPILER_IS_GNUCC)

IF(CMAKE_C_COMPILER_ID MATCHES "Clang")
	SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wswitch -Wunused-variable -Wunused-value -Wshadow -Wint-conversion -Wpointer-sign -Wprotocol -Wshorten-64-to-32")
ENDIF(CMAKE_C_COMPILER_ID MATCHES "Clang")

IF(HAVE_MEMSET_S)
	SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -DHAVE_MEMSET_S=1")
ENDIF(HAVE_MEMSET_S)

TEST_BIG_ENDIAN(WORDS_BIGENDIAN)
IF(WORDS_BIGENDIAN)
	ADD_DEFINITIONS(-DWORDS_BIGENDIAN)
ENDIF(WORDS_BIGENDIAN)

IF(COVERAGE)
	SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -fprofile-arcs -ftest-coverage")
	SET(LINK_FLAGS "${LINK_FLAGS} -fprofile-arcs -ftest-coverage")

	add_custom_command(OUTPUT run_coverage
			COMMAND ctest
			COMMAND lcov -q --capture --directory src --output-file coverage.info.total
			COMMAND lcov -q --remove coverage.info.total 'vpool.*' 'ut*.h' '*.pb-c.*' 'protobuf-c/*' 'curve25519/*' --output-file coverage.info
			COMMAND genhtml -q coverage.info --output-directory coverage
			COMMENT Collecting and creating coverage information
	)
	add_custom_target( coverage DEPENDS run_coverage )
ENDIF(COVERAGE)

add_subdirectory(src)

IF(BUILD_TESTING)
	add_subdirectory(tests)
ENDIF(BUILD_TESTING)
