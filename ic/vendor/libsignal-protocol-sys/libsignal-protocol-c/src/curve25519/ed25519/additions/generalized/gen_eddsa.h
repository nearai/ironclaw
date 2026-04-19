#ifndef __GEN_EDDSA_H__
#define __GEN_EDDSA_H__

#include "ge.h"

/* B: base point 
   R: commitment (point), 
   r: private nonce (scalar)
   K: encoded public key
   k: private key (scalar)
   Z: 32-bytes random
   M: buffer containing message, message starts at M_start, continues for M_len

   r = hash(B || labelset || Z || pad1 || k || pad2 || labelset || K || extra || M) (mod q)
*/
int generalized_commit(unsigned char* R_bytes, unsigned char* r_scalar,
            const unsigned char* labelset, const unsigned long labelset_len,
            const unsigned char* extra, const unsigned long extra_len,
            const unsigned char* K_bytes, const unsigned char* k_scalar, 
            const unsigned char* Z,
            unsigned char* M_buf, const unsigned long M_start, const unsigned long M_len);

/* if is_labelset_empty(labelset):
       return hash(R || K || M) (mod q)
   else:
       return hash(B || labelset || R || labelset || K || extra || M) (mod q)
*/
int generalized_challenge(unsigned char* h_scalar,
              const unsigned char* labelset, const unsigned long labelset_len,
              const unsigned char* extra, const unsigned long extra_len,
              const unsigned char* R_bytes,
              const unsigned char* K_bytes,
              unsigned char* M_buf, const unsigned long M_start, const unsigned long M_len);

/* return r + kh (mod q) */
int generalized_prove(unsigned char* out_scalar, 
    const unsigned char* r_scalar, 
    const unsigned char* k_scalar, 
    const unsigned char* h_scalar);

/* R = B^s / K^h */
int generalized_solve_commitment(unsigned char* R_bytes_out,  ge_p3* K_point_out, 
                     const ge_p3* B_point, const unsigned char* s_scalar,
                     const unsigned char* K_bytes, const unsigned char* h_scalar);
  

int generalized_eddsa_25519_sign(
                  unsigned char* signature_out,
                  const unsigned char* eddsa_25519_pubkey_bytes,
                  const unsigned char* eddsa_25519_privkey_scalar,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* random,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len);

int generalized_eddsa_25519_verify(
                  const unsigned char* signature,
                  const unsigned char* eddsa_25519_pubkey,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len);

#endif
