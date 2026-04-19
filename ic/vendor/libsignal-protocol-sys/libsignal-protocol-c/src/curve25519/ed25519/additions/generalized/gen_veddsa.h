#ifndef __GEN_VEDDSA_H__
#define __GEN_VEDDSA_H__

int generalized_veddsa_25519_sign(
                  unsigned char* signature_out,
                  const unsigned char* eddsa_25519_pubkey_bytes,
                  const unsigned char* eddsa_25519_privkey_scalar,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* random,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len);

int generalized_veddsa_25519_verify(
                  unsigned char* vrf_out,
                  const unsigned char* signature,
                  const unsigned char* eddsa_25519_pubkey_bytes,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len);

#endif
