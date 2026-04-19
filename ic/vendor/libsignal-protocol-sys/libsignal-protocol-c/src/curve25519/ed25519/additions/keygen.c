#include "ge.h"
#include "keygen.h"
#include "crypto_additions.h"

void curve25519_keygen(unsigned char* curve25519_pubkey_out,
                       const unsigned char* curve25519_privkey_in)
{
  /* Perform a fixed-base multiplication of the Edwards base point,
     (which is efficient due to precalculated tables), then convert
     to the Curve25519 montgomery-format public key.

     NOTE: y=1 is converted to u=0 since fe_invert is mod-exp
  */

  ge_p3 ed; /* Ed25519 pubkey point */
  fe u;

  ge_scalarmult_base(&ed, curve25519_privkey_in);
  ge_p3_to_montx(u, &ed);
  fe_tobytes(curve25519_pubkey_out, u);
}
