
#ifndef __XEDDSA_H__
#define __XEDDSA_H__

/* returns 0 on success */
int xed25519_sign(unsigned char* signature_out, /* 64 bytes */
                  const unsigned char* curve25519_privkey, /* 32 bytes */
                  const unsigned char* msg, const unsigned long msg_len, /* <= 256 bytes */
                  const unsigned char* random); /* 64 bytes */

/* returns 0 on success */
int xed25519_verify(const unsigned char* signature, /* 64 bytes */
                    const unsigned char* curve25519_pubkey, /* 32 bytes */
                    const unsigned char* msg, const unsigned long msg_len); /* <= 256 bytes */

#endif
