#ifndef __GEN_X_H
#define __GEN_X_H

int generalized_xeddsa_25519_sign(unsigned char* signature_out, /* 64 bytes */
                              const unsigned char* x25519_privkey_scalar, /* 32 bytes */
                              const unsigned char* msg, const unsigned long msg_len,
                              const unsigned char* random, /* 32 bytes */
                              const unsigned char* customization_label,
                              const unsigned long customization_label_len);

int generalized_xeddsa_25519_verify(
                  const unsigned char* signature, /* 64 bytes */
                  const unsigned char* x25519_pubkey_bytes, /* 32 bytes */
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len);

int generalized_xveddsa_25519_sign(
                  unsigned char* signature_out, /* 96 bytes */
                  const unsigned char* x25519_privkey_scalar, /* 32 bytes */
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* random, /* 32 bytes */
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len);

int generalized_xveddsa_25519_verify(
                  unsigned char* vrf_out, /* 32 bytes */
                  const unsigned char* signature, /* 96 bytes */
                  const unsigned char* x25519_pubkey_bytes, /* 32 bytes */
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len);

#endif
