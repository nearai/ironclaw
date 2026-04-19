#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include "crypto_hash_sha512.h"
#include "keygen.h"
#include "curve_sigs.h"
#include "xeddsa.h"
#include "crypto_additions.h"
#include "ge.h"
#include "utility.h"
#include "gen_crypto_additions.h"
#include "gen_x.h"
#include "internal_fast_tests.h"
#include <assert.h>


#define ERROR(...) do {if (!silent) { printf(__VA_ARGS__); abort(); } else return -1; } while (0)
#define INFO(...) do {if (!silent) printf(__VA_ARGS__);} while (0)

#define TEST(msg, cond) \
  do {  \
    if ((cond)) { \
      INFO("%s good\n", msg); \
    } \
    else { \
      ERROR("%s BAD!!!\n", msg); \
    } \
  } while (0)


int sha512_fast_test(int silent)
{
  unsigned char sha512_input[112] =   
    "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
  unsigned char sha512_correct_output[64] =
    {
    0x8E, 0x95, 0x9B, 0x75, 0xDA, 0xE3, 0x13, 0xDA,
    0x8C, 0xF4, 0xF7, 0x28, 0x14, 0xFC, 0x14, 0x3F,
    0x8F, 0x77, 0x79, 0xC6, 0xEB, 0x9F, 0x7F, 0xA1,
    0x72, 0x99, 0xAE, 0xAD, 0xB6, 0x88, 0x90, 0x18,
    0x50, 0x1D, 0x28, 0x9E, 0x49, 0x00, 0xF7, 0xE4,
    0x33, 0x1B, 0x99, 0xDE, 0xC4, 0xB5, 0x43, 0x3A,
    0xC7, 0xD3, 0x29, 0xEE, 0xB6, 0xDD, 0x26, 0x54,
    0x5E, 0x96, 0xE5, 0x5B, 0x87, 0x4B, 0xE9, 0x09
    };
  unsigned char sha512_actual_output[64];

  crypto_hash_sha512(sha512_actual_output, sha512_input, sizeof(sha512_input));
  TEST("SHA512 #1", memcmp(sha512_actual_output, sha512_correct_output, 64) == 0);

  sha512_input[111] ^= 1;

  crypto_hash_sha512(sha512_actual_output, sha512_input, sizeof(sha512_input));
  TEST("SHA512 #2", memcmp(sha512_actual_output, sha512_correct_output, 64) != 0);

  return 0;
}

int strict_fast_test(int silent)
{
  unsigned char unreduced1[32] = {
  0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F,
  };
  unsigned char unreduced2[32] = {
  0xED, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F,
  };
  unsigned char unreduced3[32] = {
  0xEC, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
  0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F,
  };

  unsigned char q[32] = {
  0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 
  0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 
  };
  unsigned char qminus1[32] = {
  0xec, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 
  0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 
  };
  unsigned char qplus1[32] = {
  0xee, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 
  0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 
  };

  TEST("fe_isreduced", 
      (fe_isreduced(unreduced1) == 0) && 
      (fe_isreduced(unreduced2) == 0) &&
      (fe_isreduced(unreduced3) == 1)
      );

  TEST("sc_isreduced", 
      (sc_isreduced(q) == 0) && 
      (sc_isreduced(qminus1) == 1) &&
      (sc_isreduced(qplus1) == 0)
      );
  return 0;
}

int ge_fast_test(int silent)
{

  const unsigned char B_bytes[] = {
    0x58, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
    0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
    0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
    0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
  };

  const unsigned char misc_bytes[] = {
    0x57, 0x17, 0xfa, 0xce, 0xca, 0xb9, 0xdf, 0x0e, 
    0x90, 0x67, 0xaa, 0x46, 0xba, 0x83, 0x2f, 0xeb, 
    0x1c, 0x49, 0xd0, 0x21, 0xb1, 0x33, 0xff, 0x11, 
    0xc9, 0x7a, 0xb8, 0xcf, 0xe3, 0x29, 0x46, 0x17,
  };

  unsigned char q_scalar[32] = {
  0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 
  0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 
  };

  unsigned char c_scalar[32] = {
  0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00 
  };

  unsigned char neutral_bytes[] = {
  0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00 
  };

/*  unsigned char one_scalar[32] = {
  0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 
  };

  const unsigned char B_bytes[] = {
    0x58, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
    0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
    0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 
    0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
  };
  */

  ge_p3 point1, point2, B_point, misc_point, miscneg_point;

  unsigned char output1[32], output2[32];

  if (ge_frombytes_negate_vartime(&B_point, B_bytes) != 0)
    TEST("Failure to parse point #1", 0);
  if (ge_frombytes_negate_vartime(&miscneg_point, misc_bytes) != 0)
    TEST("Failure to parse point #2", 0);
  ge_neg(&B_point, &B_point);
  ge_neg(&misc_point, &miscneg_point);

  /* q*B == neutral */
  ge_scalarmult_base(&point1,  q_scalar);
  ge_scalarmult(&point2, q_scalar, &B_point);
  ge_p3_tobytes(output1, &point1);
  ge_p3_tobytes(output2, &point2);

  TEST("qB == qB", memcmp(output1, output2, 32) == 0 && memcmp(output1, neutral_bytes, 32) == 0);
  TEST("qB isneutral", ge_isneutral(&point1 ) && ge_isneutral(&point2) && !ge_isneutral(&B_point));

  /* cB == cB, cX == cX */
  ge_scalarmult_cofactor(&point1, &B_point);
  ge_scalarmult_base(&point2, c_scalar);
  ge_p3_tobytes(output1, &point1);
  ge_p3_tobytes(output2, &point2);
  TEST("cB == cB", memcmp(output1, output2, 32) == 0);
  ge_scalarmult_cofactor(&point1, &misc_point);
  ge_scalarmult(&point2, c_scalar, &misc_point);
  ge_p3_tobytes(output1, &point1);
  ge_p3_tobytes(output2, &point2);
  TEST("cX == cX", memcmp(output1, output2, 32) == 0);

  /* */
  ge_p3_add(&point1, &misc_point, &miscneg_point);
  TEST("X + -X isneutral", ge_isneutral(&point1));

  return 0;
}

int elligator_fast_test(int silent)
{
  unsigned char elligator_correct_output[32] = 
  {
  0x5f, 0x35, 0x20, 0x00, 0x1c, 0x6c, 0x99, 0x36, 
  0xa3, 0x12, 0x06, 0xaf, 0xe7, 0xc7, 0xac, 0x22, 
  0x4e, 0x88, 0x61, 0x61, 0x9b, 0xf9, 0x88, 0x72, 
  0x44, 0x49, 0x15, 0x89, 0x9d, 0x95, 0xf4, 0x6e
  };

  unsigned char hashtopoint_correct_output1[32] = 
  {
  0xce, 0x89, 0x9f, 0xb2, 0x8f, 0xf7, 0x20, 0x91,
  0x5e, 0x14, 0xf5, 0xb7, 0x99, 0x08, 0xab, 0x17,
  0xaa, 0x2e, 0xe2, 0x45, 0xb4, 0xfc, 0x2b, 0xf6,
  0x06, 0x36, 0x29, 0x40, 0xed, 0x7d, 0xe7, 0xed
  };

  unsigned char hashtopoint_correct_output2[32] = 
  {
  0xa0, 0x35, 0xbb, 0xa9, 0x4d, 0x30, 0x55, 0x33, 
  0x0d, 0xce, 0xc2, 0x7f, 0x83, 0xde, 0x79, 0xd0, 
  0x89, 0x67, 0x72, 0x4c, 0x07, 0x8d, 0x68, 0x9d, 
  0x61, 0x52, 0x1d, 0xf9, 0x2c, 0x5c, 0xba, 0x77
  };

  int count;
  fe in, out;
  unsigned char bytes[32];
  fe_0(in);
  fe_0(out);
  for (count = 0; count < 32; count++) {
    bytes[count] = count;
  }
  fe_frombytes(in, bytes);
  elligator(out, in);
  fe_tobytes(bytes, out);
  TEST("Elligator vector", memcmp(bytes, elligator_correct_output, 32) == 0);

  /* Elligator(0) == 0 test */
  fe_0(in);
  elligator(out, in);
  TEST("Elligator(0) == 0", memcmp(in, out, 32) == 0);

  /* ge_montx_to_p3(0) -> order2 point test */
  fe one, negone, zero;
  fe_1(one);
  fe_0(zero);
  fe_sub(negone, zero, one);
  ge_p3 p3;
  ge_montx_to_p3(&p3, zero, 0);
  TEST("ge_montx_to_p3(0) == order 2 point", 
      fe_isequal(p3.X, zero) &&
      fe_isequal(p3.Y, negone) &&
      fe_isequal(p3.Z, one) && 
      fe_isequal(p3.T, zero));

  /* Hash to point vector test */
  unsigned char htp[32];
  
  for (count=0; count < 32; count++) {
    htp[count] = count;
  }

  hash_to_point(&p3, htp, 32);
  ge_p3_tobytes(htp, &p3);
  TEST("hash_to_point #1", memcmp(htp, hashtopoint_correct_output1, 32) == 0);

  for (count=0; count < 32; count++) {
    htp[count] = count+1;
  }

  hash_to_point(&p3, htp, 32);
  ge_p3_tobytes(htp, &p3);
  TEST("hash_to_point #2", memcmp(htp, hashtopoint_correct_output2, 32) == 0);

  return 0;
}

int curvesigs_fast_test(int silent)
{
  unsigned char signature_correct[64] = {
    0xcf, 0x87, 0x3d, 0x03, 0x79, 0xac, 0x20, 0xe8, 
    0x89, 0x3e, 0x55, 0x67, 0xee, 0x0f, 0x89, 0x51, 
    0xf8, 0xdb, 0x84, 0x0d, 0x26, 0xb2, 0x43, 0xb4, 
    0x63, 0x52, 0x66, 0x89, 0xd0, 0x1c, 0xa7, 0x18, 
    0xac, 0x18, 0x9f, 0xb1, 0x67, 0x85, 0x74, 0xeb, 
    0xdd, 0xe5, 0x69, 0x33, 0x06, 0x59, 0x44, 0x8b, 
    0x0b, 0xd6, 0xc1, 0x97, 0x3f, 0x7d, 0x78, 0x0a, 
    0xb3, 0x95, 0x18, 0x62, 0x68, 0x03, 0xd7, 0x82,
  };
  const int MSG_LEN  = 200;
  unsigned char privkey[32];
  unsigned char pubkey[32];
  unsigned char signature[64];
  unsigned char msg[MSG_LEN];
  unsigned char random[64];

  memset(privkey, 0, 32);
  memset(pubkey, 0, 32);
  memset(signature, 0, 64);
  memset(msg, 0, MSG_LEN);
  memset(random, 0, 64);

  privkey[8] = 189; /* just so there's some bits set */
  sc_clamp(privkey);
  
  /* Signature vector test */
  curve25519_keygen(pubkey, privkey);

  curve25519_sign(signature, privkey, msg, MSG_LEN, random);

  TEST("Curvesig sign", memcmp(signature, signature_correct, 64) == 0);
  TEST("Curvesig verify #1", curve25519_verify(signature, pubkey, msg, MSG_LEN) == 0);
  signature[0] ^= 1;
  TEST("Curvesig verify #2", curve25519_verify(signature, pubkey, msg, MSG_LEN) != 0);
  return 0;
}

int xeddsa_fast_test(int silent)
{
  unsigned char signature_correct[64] = {
  0x11, 0xc7, 0xf3, 0xe6, 0xc4, 0xdf, 0x9e, 0x8a, 
  0x51, 0x50, 0xe1, 0xdb, 0x3b, 0x30, 0xf9, 0x2d, 
  0xe3, 0xa3, 0xb3, 0xaa, 0x43, 0x86, 0x56, 0x54, 
  0x5f, 0xa7, 0x39, 0x0f, 0x4b, 0xcc, 0x7b, 0xb2, 
  0x6c, 0x43, 0x1d, 0x9e, 0x90, 0x64, 0x3e, 0x4f, 
  0x0e, 0xaa, 0x0e, 0x9c, 0x55, 0x77, 0x66, 0xfa, 
  0x69, 0xad, 0xa5, 0x76, 0xd6, 0x3d, 0xca, 0xf2, 
  0xac, 0x32, 0x6c, 0x11, 0xd0, 0xb9, 0x77, 0x02,
  };
  const int MSG_LEN  = 200;
  unsigned char privkey[32];
  unsigned char pubkey[32];
  unsigned char signature[64];
  unsigned char msg[MSG_LEN];
  unsigned char random[64];

  memset(privkey, 0, 32);
  memset(pubkey, 0, 32);
  memset(signature, 0, 64);
  memset(msg, 0, MSG_LEN);
  memset(random, 0, 64);

  privkey[8] = 189; /* just so there's some bits set */
  sc_clamp(privkey);
  
  /* Signature vector test */
  curve25519_keygen(pubkey, privkey);

  xed25519_sign(signature, privkey, msg, MSG_LEN, random);
  TEST("XEdDSA sign", memcmp(signature, signature_correct, 64) == 0);
  TEST("XEdDSA verify #1", xed25519_verify(signature, pubkey, msg, MSG_LEN) == 0);
  signature[0] ^= 1;
  TEST("XEdDSA verify #2", xed25519_verify(signature, pubkey, msg, MSG_LEN) != 0);
  memset(pubkey, 0xFF, 32);
  TEST("XEdDSA verify #3", xed25519_verify(signature, pubkey, msg, MSG_LEN) != 0);
  return 0;
}

int generalized_xeddsa_fast_test(int silent)
{
  unsigned char signature1[64];
  unsigned char signature2[64];
  unsigned char privkey[32];
  unsigned char pubkey[32];
  unsigned char msg1[1000];
  unsigned char msg2[1000];
  unsigned char random[64];

  memset(signature1, 0, 64);
  memset(signature2, 0, 64);
  memset(privkey, 0xF0, 32);
  memset(pubkey, 2, 32);
  memset(msg1, 0x10, 1000);
  memset(msg2, 0x20, 1000);
  memset(random, 0xBC, 64);

  sc_clamp(privkey);
  curve25519_keygen(pubkey, privkey);

  msg2[0] = 1;
  TEST("generalized xeddsa sign #1", generalized_xeddsa_25519_sign(signature1, privkey, msg1, 100, random, NULL, 0) == 0);
  TEST("generalized xeddsa sign #2", generalized_xeddsa_25519_sign(signature2, privkey, msg2, 100, random, NULL, 0) == 0);

  TEST("generalized (old) xeddsa verify #1", xed25519_verify(signature1, pubkey, msg1, 100) == 0);
  TEST("generalized (old) xeddsa verify #2", xed25519_verify(signature2, pubkey, msg2, 100) == 0);
  TEST("generalized (old) xeddsa verify #3", xed25519_verify(signature1, pubkey, msg2, 100) != 0);
  TEST("generalized (old) xeddsa verify #4", xed25519_verify(signature2, pubkey, msg1, 100) != 0);

  TEST("generalized xeddsa verify #1", generalized_xeddsa_25519_verify(signature1, pubkey, msg1, 100, NULL, 0) == 0);
  TEST("generalized xeddsa verify #2", generalized_xeddsa_25519_verify(signature2, pubkey, msg2, 100, NULL, 0) == 0);
  TEST("generalized xeddsa verify #3", generalized_xeddsa_25519_verify(signature1, pubkey, msg2, 100, NULL, 0) != 0);
  TEST("generalized xeddsa verify #4", generalized_xeddsa_25519_verify(signature2, pubkey, msg1, 100, NULL, 0) != 0);
  return 0;
}

int generalized_xveddsa_fast_test(int silent)
{
  unsigned char signature1[96];
  unsigned char signature2[96];
  unsigned char privkey[32];
  unsigned char pubkey[32];
  unsigned char msg1[1000];
  unsigned char msg2[1000];
  unsigned char random[64];
  unsigned char vrf[32];

  memset(signature1, 0, 64);
  memset(signature2, 0, 64);
  memset(privkey, 1, 32);
  memset(pubkey, 2, 32);
  memset(msg1, 0x11, 1000);
  memset(msg2, 0x22, 1000);
  memset(random, 0xAB, 64);

  sc_clamp(privkey);
  curve25519_keygen(pubkey, privkey);

  msg2[0] ^= 1;
  TEST("generalized xveddsa sign #1", generalized_xveddsa_25519_sign(signature1, privkey, msg1, 100, random, NULL, 0) == 0);
  TEST("generalized xveddsa sign #2", generalized_xveddsa_25519_sign(signature2, privkey, msg2, 100, random, (unsigned char*)"abc", 3) == 0);

  TEST("generalized xveddsa verify #1", generalized_xveddsa_25519_verify(vrf, signature1, pubkey, msg1, 100, NULL, 0) == 0);
  TEST("generalized xveddsa verify #2", generalized_xveddsa_25519_verify(vrf, signature2, pubkey, msg2, 100, (unsigned char*)"abc", 3) == 0);
  TEST("generalized xveddsa verify #3", generalized_xveddsa_25519_verify(vrf, signature1, pubkey, msg2, 100, NULL, 0) != 0);
  TEST("generalized xveddsa verify #4", generalized_xveddsa_25519_verify(vrf, signature2, pubkey, msg1, 100, (unsigned char*)"abc", 3) != 0);


  unsigned char signature3[96];
  unsigned char vrf3[96];
  random[0] ^= 1;
  TEST("generalized xveddsa sign #3", generalized_xveddsa_25519_sign(signature3, privkey, msg1, 100, random, NULL, 0) == 0);
  TEST("generalized xveddsa verify #5", generalized_xveddsa_25519_verify(vrf, signature1, pubkey, msg1, 100, NULL, 0) == 0);
  TEST("generalized xveddsa verify #6", generalized_xveddsa_25519_verify(vrf3, signature3, pubkey, msg1, 100, NULL, 0) == 0);
  TEST("generalized xveddsa VRFs equal", memcmp(vrf, vrf3, 32) == 0);
  TEST("generalized xveddsa Kv equal", memcmp(signature1+0, signature3+0, 32) == 0);
  TEST("generalized xveddsa h not equal", memcmp(signature1+32, signature3+32, 32) != 0);
  TEST("generalized xveddsa s not equal", memcmp(signature1+64, signature3+64, 32) != 0);
  return 0;
}

int all_fast_tests(int silent)
{
  int result;
  if ((result = sha512_fast_test(silent)) != 0)
    return -1;
  if ((result = strict_fast_test(silent)) != 0)
    return -2;
  if ((result = ge_fast_test(silent)) != 0)
    return -3;
  if ((result = elligator_fast_test(silent)) != 0)
    return -3;
  if ((result = curvesigs_fast_test(silent)) != 0)
    return -4;
  if ((result = xeddsa_fast_test(silent)) != 0)
    return -5;
  if ((result = generalized_xeddsa_fast_test(silent)) != 0)
    return -6;
  if ((result = generalized_xveddsa_fast_test(silent)) != 0)
    return -7;

  return 0;
}

