
#ifndef __CRYPTO_ADDITIONS__
#define __CRYPTO_ADDITIONS__

#include "crypto_uint32.h"
#include "fe.h"
#include "ge.h"

#define MAX_MSG_LEN 256

void sc_neg(unsigned char *b, const unsigned char *a);
void sc_cmov(unsigned char* f, const unsigned char* g, unsigned char b);

int fe_isequal(const fe f, const fe g);
int fe_isreduced(const unsigned char* s);
void fe_mont_rhs(fe v2, const fe u);
void fe_montx_to_edy(fe y, const fe u);
void fe_sqrt(fe b, const fe a);

int ge_isneutral(const ge_p3* q);
void ge_neg(ge_p3* r, const ge_p3 *p);
void ge_montx_to_p3(ge_p3* p, const fe u, const unsigned char ed_sign_bit);
void ge_p3_to_montx(fe u, const ge_p3 *p);
void ge_scalarmult(ge_p3 *h, const unsigned char *a, const ge_p3 *A);
void ge_scalarmult_cofactor(ge_p3 *q, const ge_p3 *p);

void elligator(fe u, const fe r);
void hash_to_point(ge_p3* p, const unsigned char* msg, const unsigned long in_len);

int crypto_sign_modified(
  unsigned char *sm,
  const unsigned char *m,unsigned long long mlen,
  const unsigned char *sk, /* Curve/Ed25519 private key */
  const unsigned char *pk, /* Ed25519 public key */
  const unsigned char *random /* 64 bytes random to hash into nonce */
  );

int crypto_sign_open_modified(
  unsigned char *m,
  const unsigned char *sm,unsigned long long smlen,
  const unsigned char *pk
  );


#endif
