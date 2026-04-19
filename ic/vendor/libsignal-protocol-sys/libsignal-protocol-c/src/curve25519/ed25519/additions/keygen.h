
#ifndef __KEYGEN_H__
#define __KEYGEN_H__

/* Sets and clears bits to make a random 32 bytes into a private key */
void sc_clamp(unsigned char* a);

/* The private key should be 32 random bytes "clamped" by sc_clamp() */
void curve25519_keygen(unsigned char* curve25519_pubkey_out, /* 32 bytes */
                       const unsigned char* curve25519_privkey_in); /* 32 bytes */

#endif
