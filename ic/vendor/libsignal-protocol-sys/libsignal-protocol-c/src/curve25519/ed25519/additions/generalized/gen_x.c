#include <string.h>
#include "crypto_additions.h"
#include "gen_x.h"
#include "gen_constants.h"
#include "gen_eddsa.h"
#include "gen_veddsa.h"
#include "gen_crypto_additions.h"
#include "zeroize.h"

static int convert_25519_pubkey(unsigned char* ed_pubkey_bytes, const unsigned char* x25519_pubkey_bytes) {
  fe u;
  fe y;

  /* Convert the X25519 public key into an Ed25519 public key.

     y = (u - 1) / (u + 1)

     NOTE: u=-1 is converted to y=0 since fe_invert is mod-exp
  */
  if (!fe_isreduced(x25519_pubkey_bytes))
      return -1;
  fe_frombytes(u, x25519_pubkey_bytes);
  fe_montx_to_edy(y, u);
  fe_tobytes(ed_pubkey_bytes, y);
  return 0;
}

static int calculate_25519_keypair(unsigned char* K_bytes, unsigned char* k_scalar, 
                            const unsigned char* x25519_privkey_scalar)
{
  unsigned char kneg[SCALARLEN];
  ge_p3 ed_pubkey_point;
  unsigned char sign_bit = 0;

  if (SCALARLEN != 32)
    return -1;

  /* Convert the Curve25519 privkey to an Ed25519 public key */
  ge_scalarmult_base(&ed_pubkey_point, x25519_privkey_scalar);
  ge_p3_tobytes(K_bytes, &ed_pubkey_point);

  /* Force Edwards sign bit to zero */
  sign_bit = (K_bytes[31] & 0x80) >> 7;
  memcpy(k_scalar, x25519_privkey_scalar, 32);
  sc_neg(kneg, k_scalar);
  sc_cmov(k_scalar, kneg, sign_bit); 
  K_bytes[31] &= 0x7F;

  zeroize(kneg, SCALARLEN);
  return 0;
}

int generalized_xeddsa_25519_sign(unsigned char* signature_out,
                              const unsigned char* x25519_privkey_scalar,
                              const unsigned char* msg, const unsigned long msg_len,
                              const unsigned char* random,
                              const unsigned char* customization_label,
                              const unsigned long customization_label_len)
{
  unsigned char K_bytes[POINTLEN];
  unsigned char k_scalar[SCALARLEN];
  int retval = -1;

  if (calculate_25519_keypair(K_bytes, k_scalar, x25519_privkey_scalar) != 0)
    return -1;

  retval = generalized_eddsa_25519_sign(signature_out, 
                                        K_bytes, k_scalar,
                                        msg, msg_len, random, 
                                        customization_label, customization_label_len);
  zeroize(k_scalar, SCALARLEN);
  return retval;
}

int generalized_xveddsa_25519_sign(
                  unsigned char* signature_out,
                  const unsigned char* x25519_privkey_scalar,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* random,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len)
{
  unsigned char K_bytes[POINTLEN];
  unsigned char k_scalar[SCALARLEN];
  int retval = -1;

  if (calculate_25519_keypair(K_bytes, k_scalar, x25519_privkey_scalar) != 0)
    return -1;

  retval = generalized_veddsa_25519_sign(signature_out, K_bytes, k_scalar, 
                                         msg, msg_len, random, 
                                         customization_label, customization_label_len);
  zeroize(k_scalar, SCALARLEN);
  return retval;
}

int generalized_xeddsa_25519_verify(
                  const unsigned char* signature,
                  const unsigned char* x25519_pubkey_bytes,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len)
{
  unsigned char K_bytes[POINTLEN];

  if (convert_25519_pubkey(K_bytes, x25519_pubkey_bytes) != 0)
      return -1;

  return generalized_eddsa_25519_verify(signature, K_bytes, msg, msg_len, 
                                        customization_label, customization_label_len);
}

int generalized_xveddsa_25519_verify(
                  unsigned char* vrf_out,
                  const unsigned char* signature,
                  const unsigned char* x25519_pubkey_bytes,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len)
{
  unsigned char K_bytes[POINTLEN];

  if (convert_25519_pubkey(K_bytes, x25519_pubkey_bytes) != 0)
      return -1;

  return generalized_veddsa_25519_verify(vrf_out, signature, K_bytes, msg, msg_len, 
                                         customization_label, customization_label_len);
}
