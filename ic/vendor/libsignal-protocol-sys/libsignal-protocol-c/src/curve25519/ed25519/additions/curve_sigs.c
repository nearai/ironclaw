#include <stdlib.h>
#include <string.h>
#include "ge.h"
#include "curve_sigs.h"
#include "crypto_sign.h"
#include "crypto_additions.h"

int curve25519_sign(unsigned char* signature_out,
                    const unsigned char* curve25519_privkey,
                    const unsigned char* msg, const unsigned long msg_len,
                    const unsigned char* random)
{
  ge_p3 ed_pubkey_point; /* Ed25519 pubkey point */
  unsigned char ed_pubkey[32]; /* Ed25519 encoded pubkey */
  unsigned char *sigbuf; /* working buffer */
  unsigned char sign_bit = 0;

  if ((sigbuf = malloc(msg_len + 128)) == 0) {
    memset(signature_out, 0, 64);
    return -1;
  }

  /* Convert the Curve25519 privkey to an Ed25519 public key */
  ge_scalarmult_base(&ed_pubkey_point, curve25519_privkey);
  ge_p3_tobytes(ed_pubkey, &ed_pubkey_point);
  sign_bit = ed_pubkey[31] & 0x80;

  /* Perform an Ed25519 signature with explicit private key */
  crypto_sign_modified(sigbuf, msg, msg_len, curve25519_privkey,
                       ed_pubkey, random);
  memmove(signature_out, sigbuf, 64);

  /* Encode the sign bit into signature (in unused high bit of S) */
   signature_out[63] &= 0x7F; /* bit should be zero already, but just in case */
   signature_out[63] |= sign_bit;

   free(sigbuf);
   return 0;
}

int curve25519_verify(const unsigned char* signature,
                      const unsigned char* curve25519_pubkey,
                      const unsigned char* msg, const unsigned long msg_len)
{
  fe u;
  fe y;
  unsigned char ed_pubkey[32];
  unsigned char *verifybuf  = NULL; /* working buffer */
  unsigned char *verifybuf2 = NULL; /* working buffer #2 */
  int result;

  if ((verifybuf = malloc(msg_len + 64)) == 0) {
   result = -1;
   goto err;
  }

  if ((verifybuf2 = malloc(msg_len + 64)) == 0) {
    result = -1;
    goto err;
  }

  /* Convert the Curve25519 public key into an Ed25519 public key.  In
     particular, convert Curve25519's "montgomery" x-coordinate (u) into an
     Ed25519 "edwards" y-coordinate:

     y = (u - 1) / (u + 1)

     NOTE: u=-1 is converted to y=0 since fe_invert is mod-exp

     Then move the sign bit into the pubkey from the signature.
  */
  fe_frombytes(u, curve25519_pubkey);
  fe_montx_to_edy(y, u);
  fe_tobytes(ed_pubkey, y);

  /* Copy the sign bit, and remove it from signature */
  ed_pubkey[31] &= 0x7F;  /* bit should be zero already, but just in case */
  ed_pubkey[31] |= (signature[63] & 0x80);
  memmove(verifybuf, signature, 64);
  verifybuf[63] &= 0x7F;

  memmove(verifybuf+64, msg, msg_len);

  /* Then perform a normal Ed25519 verification, return 0 on success */
  /* The below call has a strange API: */
  /* verifybuf = R || S || message */
  /* verifybuf2 = internal to next call gets a copy of verifybuf, S gets 
     replaced with pubkey for hashing */
  result = crypto_sign_open_modified(verifybuf2, verifybuf, 64 + msg_len, ed_pubkey);

  err:

  if (verifybuf != NULL) {
    free(verifybuf);
  }

  if (verifybuf2 != NULL) {
    free(verifybuf2);
  }

  return result;
}
