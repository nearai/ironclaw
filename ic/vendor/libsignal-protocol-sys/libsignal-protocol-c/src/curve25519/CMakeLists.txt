IF(CMAKE_COMPILER_IS_GNUCC OR CMAKE_C_COMPILER_ID MATCHES "Clang")
    SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wno-unused-variable -Wno-unused-function -Wno-shadow")
ENDIF(CMAKE_COMPILER_IS_GNUCC OR CMAKE_C_COMPILER_ID MATCHES "Clang")

IF(CMAKE_COMPILER_IS_GNUCC)
    SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wno-sign-compare")
    IF(GCC_WARN_SIGN_CONVERSION)
        SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wno-sign-conversion")
    ENDIF(GCC_WARN_SIGN_CONVERSION)
ENDIF(CMAKE_COMPILER_IS_GNUCC)

IF(CMAKE_C_COMPILER_ID MATCHES "Clang")
    SET(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wno-shorten-64-to-32")
ENDIF(CMAKE_C_COMPILER_ID MATCHES "Clang")

include_directories(
    ed25519/nacl_includes
    ed25519/additions
    ed25519/additions/generalized
    ed25519/sha512
    ed25519/tests
    ed25519
)

set(curve25519_SRCS
    curve25519-donna.c
)

set(ed25519_SRCS
    ed25519/fe_0.c
    ed25519/fe_1.c
    ed25519/fe_add.c
    ed25519/fe_cmov.c
    ed25519/fe_copy.c
    ed25519/fe_frombytes.c
    ed25519/fe_invert.c
    ed25519/fe_isnegative.c
    ed25519/fe_isnonzero.c
    ed25519/fe_mul.c
    ed25519/fe_neg.c
    ed25519/fe_pow22523.c
    ed25519/fe_sq.c
    ed25519/fe_sq2.c
    ed25519/fe_sub.c
    ed25519/fe_tobytes.c
    ed25519/ge_add.c
    ed25519/ge_double_scalarmult.c
    ed25519/ge_frombytes.c
    ed25519/ge_madd.c
    ed25519/ge_msub.c
    ed25519/ge_p1p1_to_p2.c
    ed25519/ge_p1p1_to_p3.c
    ed25519/ge_p2_0.c
    ed25519/ge_p2_dbl.c
    ed25519/ge_p3_0.c
    ed25519/ge_p3_dbl.c
    ed25519/ge_p3_to_cached.c
    ed25519/ge_p3_to_p2.c
    ed25519/ge_p3_tobytes.c
    ed25519/ge_precomp_0.c
    ed25519/ge_scalarmult_base.c
    ed25519/ge_sub.c
    ed25519/ge_tobytes.c
    ed25519/open.c
    ed25519/sc_muladd.c
    ed25519/sc_reduce.c
    ed25519/sign.c
    ed25519/additions/compare.c
    ed25519/additions/curve_sigs.c
    ed25519/additions/elligator.c
    ed25519/additions/fe_isequal.c
    ed25519/additions/fe_isreduced.c
    ed25519/additions/fe_mont_rhs.c
    ed25519/additions/fe_montx_to_edy.c
    ed25519/additions/fe_sqrt.c
    ed25519/additions/ge_isneutral.c
    ed25519/additions/ge_montx_to_p3.c
    ed25519/additions/ge_neg.c
    ed25519/additions/ge_p3_to_montx.c
    ed25519/additions/ge_scalarmult.c
    ed25519/additions/ge_scalarmult_cofactor.c
    ed25519/additions/keygen.c
    ed25519/additions/open_modified.c
    ed25519/additions/sc_clamp.c
    ed25519/additions/sc_cmov.c
    ed25519/additions/sc_neg.c
    ed25519/additions/sign_modified.c
    ed25519/additions/utility.c
    ed25519/additions/generalized/ge_p3_add.c
    ed25519/additions/generalized/gen_eddsa.c
    ed25519/additions/generalized/gen_labelset.c
    ed25519/additions/generalized/gen_veddsa.c
    ed25519/additions/generalized/gen_x.c
    ed25519/additions/generalized/point_isreduced.c
    ed25519/additions/generalized/sc_isreduced.c
    ed25519/additions/xeddsa.c
    ed25519/additions/zeroize.c
    ed25519/nacl_sha512/blocks.c
    ed25519/nacl_sha512/hash.c
    ed25519/tests/internal_fast_tests.c
)

add_library(curve25519 OBJECT ${curve25519_SRCS} ${ed25519_SRCS})

# Add -fPIC flag
if(BUILD_SHARED_LIBS)
    set_property(TARGET curve25519 PROPERTY POSITION_INDEPENDENT_CODE ON)
endif()
