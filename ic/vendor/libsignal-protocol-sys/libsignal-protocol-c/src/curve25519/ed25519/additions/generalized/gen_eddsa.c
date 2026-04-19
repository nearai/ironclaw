#include <string.h>
#include "gen_eddsa.h"
#include "gen_labelset.h"
#include "gen_constants.h"
#include "gen_crypto_additions.h"
#include "crypto_hash_sha512.h"
#include "crypto_verify_32.h"
#include "zeroize.h"
#include "ge.h"
#include "sc.h"
#include "crypto_additions.h"
#include "utility.h"

/* B: base point 
 * R: commitment (point), 
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
            unsigned char* M_buf, const unsigned long M_start, const unsigned long M_len)
{
  ge_p3 R_point;
  unsigned char hash[HASHLEN];
  unsigned char* bufstart = NULL;
  unsigned char* bufptr = NULL;
  unsigned char* bufend = NULL;
  unsigned long prefix_len = 0;

  if (labelset_validate(labelset, labelset_len) != 0)
    goto err;
  if (R_bytes == NULL || r_scalar == NULL || 
      K_bytes == NULL || k_scalar == NULL || 
      Z == NULL || M_buf == NULL)
    goto err;
  if (extra == NULL && extra_len != 0)
    goto err;
  if (extra != NULL && extra_len == 0)
    goto err;
  if (extra != NULL && labelset_is_empty(labelset, labelset_len))
    goto err;
  if (HASHLEN != 64)
    goto err;

  prefix_len = 0;
  prefix_len += POINTLEN + labelset_len + RANDLEN;
  prefix_len += ((BLOCKLEN - (prefix_len % BLOCKLEN)) % BLOCKLEN);
  prefix_len += SCALARLEN;
  prefix_len += ((BLOCKLEN - (prefix_len % BLOCKLEN)) % BLOCKLEN);
  prefix_len += labelset_len + POINTLEN + extra_len;
  if (prefix_len > M_start)
    goto err;

  bufstart = M_buf + M_start - prefix_len;
  bufptr = bufstart;
  bufend = M_buf + M_start;
  bufptr = buffer_add(bufptr, bufend, B_bytes, POINTLEN);
  bufptr = buffer_add(bufptr, bufend, labelset, labelset_len);
  bufptr = buffer_add(bufptr, bufend, Z, RANDLEN);
  bufptr = buffer_pad(bufstart, bufptr, bufend);
  bufptr = buffer_add(bufptr, bufend, k_scalar, SCALARLEN);
  bufptr = buffer_pad(bufstart, bufptr, bufend);
  bufptr = buffer_add(bufptr, bufend, labelset, labelset_len);
  bufptr = buffer_add(bufptr, bufend, K_bytes, POINTLEN);
  bufptr = buffer_add(bufptr, bufend, extra, extra_len);
  if (bufptr != bufend || bufptr != M_buf + M_start || bufptr - bufstart != prefix_len)
    goto err;

  crypto_hash_sha512(hash, M_buf + M_start - prefix_len, prefix_len + M_len);
  sc_reduce(hash);
  ge_scalarmult_base(&R_point, hash);
  ge_p3_tobytes(R_bytes, &R_point);
  memcpy(r_scalar, hash, SCALARLEN);

  zeroize(hash, HASHLEN);
  zeroize(bufstart, prefix_len);
  return 0;

err:
  zeroize(hash, HASHLEN);
  zeroize(M_buf, M_start);
  return -1;
}

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
              unsigned char* M_buf, const unsigned long M_start, const unsigned long M_len)
{
  unsigned char hash[HASHLEN];
  unsigned char* bufstart = NULL;
  unsigned char* bufptr = NULL;
  unsigned char* bufend = NULL;
  unsigned long prefix_len = 0;

  if (h_scalar == NULL)
    goto err;
  memset(h_scalar, 0, SCALARLEN);

  if (labelset_validate(labelset, labelset_len) != 0)
    goto err;
  if (R_bytes == NULL || K_bytes == NULL || M_buf == NULL)
    goto err;
  if (extra == NULL && extra_len != 0)
    goto err;
  if (extra != NULL && extra_len == 0)
    goto err;
  if (extra != NULL && labelset_is_empty(labelset, labelset_len))
    goto err;
  if (HASHLEN != 64)
    goto err;

  if (labelset_is_empty(labelset, labelset_len)) {
    if (2*POINTLEN > M_start)
      goto err;
    if (extra != NULL || extra_len != 0)
      goto err;
    memcpy(M_buf + M_start - (2*POINTLEN),  R_bytes, POINTLEN);
    memcpy(M_buf + M_start - (1*POINTLEN),  K_bytes, POINTLEN);
    prefix_len = 2*POINTLEN;
  } else {
    prefix_len = 3*POINTLEN + 2*labelset_len + extra_len; 
    if (prefix_len > M_start)
      goto err;

    bufstart = M_buf + M_start - prefix_len;
    bufptr = bufstart;
    bufend = M_buf + M_start;
    bufptr = buffer_add(bufptr, bufend, B_bytes, POINTLEN);
    bufptr = buffer_add(bufptr, bufend, labelset, labelset_len);
    bufptr = buffer_add(bufptr, bufend, R_bytes, POINTLEN);
    bufptr = buffer_add(bufptr, bufend, labelset, labelset_len);
    bufptr = buffer_add(bufptr, bufend, K_bytes, POINTLEN);
    bufptr = buffer_add(bufptr, bufend, extra, extra_len);

    if (bufptr == NULL)
      goto err;
    if (bufptr != bufend || bufptr != M_buf + M_start || bufptr - bufstart != prefix_len)
      goto err;
  }

  crypto_hash_sha512(hash, M_buf + M_start - prefix_len, prefix_len + M_len);
  sc_reduce(hash);
  memcpy(h_scalar, hash, SCALARLEN);
  return 0;

err:
  return -1;
}

/* return r + kh (mod q) */
int generalized_prove(unsigned char* out_scalar, 
    const unsigned char* r_scalar, const unsigned char* k_scalar, const unsigned char* h_scalar)
{
  sc_muladd(out_scalar, h_scalar, k_scalar, r_scalar);
  zeroize_stack();
  return 0;
}

/* R = s*B - h*K */
int generalized_solve_commitment(unsigned char* R_bytes_out,  ge_p3* K_point_out, 
                                 const ge_p3* B_point, const unsigned char* s_scalar,
                                 const unsigned char* K_bytes, const unsigned char* h_scalar)
{

  ge_p3 Kneg_point;
  ge_p2 R_calc_point_p2;

  ge_p3 sB;
  ge_p3 hK;
  ge_p3 R_calc_point_p3;

  if (ge_frombytes_negate_vartime(&Kneg_point, K_bytes) != 0) 
    return -1;

  if (B_point == NULL) {
    ge_double_scalarmult_vartime(&R_calc_point_p2, h_scalar, &Kneg_point, s_scalar);
    ge_tobytes(R_bytes_out, &R_calc_point_p2); 
  }
  else {
    // s * Bv
    ge_scalarmult(&sB, s_scalar, B_point);

    // h * -K
    ge_scalarmult(&hK, h_scalar, &Kneg_point);

    // R = sB - hK
    ge_p3_add(&R_calc_point_p3, &sB, &hK);
    ge_p3_tobytes(R_bytes_out, &R_calc_point_p3);
  }

  if (K_point_out) {
    ge_neg(K_point_out, &Kneg_point);
  }

  return 0;
}
    

int generalized_eddsa_25519_sign(
                  unsigned char* signature_out,
                  const unsigned char* eddsa_25519_pubkey_bytes,
                  const unsigned char* eddsa_25519_privkey_scalar,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* random,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len)
{
  unsigned char labelset[LABELSETMAXLEN];
  unsigned long labelset_len = 0;
  unsigned char R_bytes[POINTLEN];
  unsigned char r_scalar[SCALARLEN];
  unsigned char h_scalar[SCALARLEN];
  unsigned char s_scalar[SCALARLEN];
  unsigned char* M_buf = NULL;

  if (signature_out == NULL)
    goto err;
  memset(signature_out, 0, SIGNATURELEN);

  if (eddsa_25519_pubkey_bytes == NULL)
    goto err;
  if (eddsa_25519_privkey_scalar == NULL)
    goto err;
  if (msg == NULL)
    goto err;
  if (customization_label == NULL && customization_label_len != 0)
    goto err;
  if (customization_label_len > LABELMAXLEN)
    goto err;
  if (msg_len > MSGMAXLEN)
    goto err;

  if ((M_buf = malloc(msg_len + MSTART)) == 0)
    goto err;
  memcpy(M_buf + MSTART, msg, msg_len);

  if (labelset_new(labelset, &labelset_len, LABELSETMAXLEN, NULL, 0, 
                   customization_label, customization_label_len) != 0)
    goto err;

  if (generalized_commit(R_bytes, r_scalar, labelset, labelset_len, NULL, 0, 
                         eddsa_25519_pubkey_bytes, eddsa_25519_privkey_scalar, 
                         random, M_buf, MSTART, msg_len) != 0)
    goto err;

  if (generalized_challenge(h_scalar, labelset, labelset_len, NULL, 0, 
                            R_bytes, eddsa_25519_pubkey_bytes, M_buf, MSTART, msg_len) != 0)
    goto err;

  if (generalized_prove(s_scalar, r_scalar, eddsa_25519_privkey_scalar, h_scalar) != 0)
    goto err;

  memcpy(signature_out, R_bytes, POINTLEN);
  memcpy(signature_out + POINTLEN, s_scalar, SCALARLEN);

  zeroize(r_scalar, SCALARLEN);
  zeroize_stack();
  free(M_buf);
  return 0;

err:
  zeroize(r_scalar, SCALARLEN);
  zeroize_stack();
  free(M_buf);
  return -1;
}

int generalized_eddsa_25519_verify(
                  const unsigned char* signature,
                  const unsigned char* eddsa_25519_pubkey_bytes,
                  const unsigned char* msg, 
                  const unsigned long msg_len,
                  const unsigned char* customization_label,
                  const unsigned long customization_label_len)
{
  unsigned char labelset[LABELSETMAXLEN];
  unsigned long labelset_len = 0;
  const unsigned char* R_bytes = NULL;
  const unsigned char* s_scalar = NULL;
  unsigned char h_scalar[SCALARLEN];
  unsigned char* M_buf = NULL;
  unsigned char R_calc_bytes[POINTLEN];

  if (signature == NULL)
    goto err;
  if (eddsa_25519_pubkey_bytes == NULL)
    goto err;
  if (msg == NULL)
    goto err;
  if (customization_label == NULL && customization_label_len != 0)
    goto err;
  if (customization_label_len > LABELMAXLEN)
    goto err;
  if (msg_len > MSGMAXLEN)
    goto err;

  if ((M_buf = malloc(msg_len + MSTART)) == 0)
    goto err;
  memcpy(M_buf + MSTART, msg, msg_len);

  if (labelset_new(labelset, &labelset_len, LABELSETMAXLEN, NULL, 0, 
                   customization_label, customization_label_len) != 0)
    goto err;

  R_bytes = signature;
  s_scalar = signature + POINTLEN;

  if (!point_isreduced(eddsa_25519_pubkey_bytes))
    goto err;
  if (!point_isreduced(R_bytes))
    goto err;
  if (!sc_isreduced(s_scalar))
    goto err;

  if (generalized_challenge(h_scalar, labelset, labelset_len, 
                            NULL, 0, R_bytes, eddsa_25519_pubkey_bytes, M_buf, MSTART, msg_len) != 0)
    goto err;

  if (generalized_solve_commitment(R_calc_bytes, NULL, NULL, 
                                   s_scalar, eddsa_25519_pubkey_bytes, h_scalar) != 0)
    goto err;

  if (crypto_verify_32(R_bytes, R_calc_bytes) != 0)
    goto err;

  free(M_buf);
  return 0;

err:
  free(M_buf);
  return -1;
}
