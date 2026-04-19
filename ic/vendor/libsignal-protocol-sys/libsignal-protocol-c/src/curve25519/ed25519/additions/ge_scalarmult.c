#include "crypto_uint32.h"
#include "ge.h"
#include "crypto_additions.h"

static unsigned char equal(signed char b,signed char c)
{
  unsigned char ub = b;
  unsigned char uc = c;
  unsigned char x = ub ^ uc; /* 0: yes; 1..255: no */
  crypto_uint32 y = x; /* 0: yes; 1..255: no */
  y -= 1; /* 4294967295: yes; 0..254: no */
  y >>= 31; /* 1: yes; 0: no */
  return y;
}

static unsigned char negative(signed char b)
{
  unsigned long long x = b; /* 18446744073709551361..18446744073709551615: yes; 0..255: no */
  x >>= 63; /* 1: yes; 0: no */
  return x;
}

static void cmov(ge_cached *t,const ge_cached *u,unsigned char b)
{
  fe_cmov(t->YplusX,u->YplusX,b);
  fe_cmov(t->YminusX,u->YminusX,b);
  fe_cmov(t->Z,u->Z,b);
  fe_cmov(t->T2d,u->T2d,b);
}

static void select(ge_cached *t,const ge_cached *pre, signed char b)
{
  ge_cached minust;
  unsigned char bnegative = negative(b);
  unsigned char babs = b - (((-bnegative) & b) << 1);

  fe_1(t->YplusX);
  fe_1(t->YminusX);
  fe_1(t->Z);
  fe_0(t->T2d);

  cmov(t,pre+0,equal(babs,1));
  cmov(t,pre+1,equal(babs,2));
  cmov(t,pre+2,equal(babs,3));
  cmov(t,pre+3,equal(babs,4));
  cmov(t,pre+4,equal(babs,5));
  cmov(t,pre+5,equal(babs,6));
  cmov(t,pre+6,equal(babs,7));
  cmov(t,pre+7,equal(babs,8));
  fe_copy(minust.YplusX,t->YminusX);
  fe_copy(minust.YminusX,t->YplusX);
  fe_copy(minust.Z,t->Z);
  fe_neg(minust.T2d,t->T2d);
  cmov(t,&minust,bnegative);
}

/*
h = a * B
where a = a[0]+256*a[1]+...+256^31 a[31]
B is the Ed25519 base point (x,4/5) with x positive.

Preconditions:
  a[31] <= 127
*/

void ge_scalarmult(ge_p3 *h, const unsigned char *a, const ge_p3 *A)
{
  signed char e[64];
  signed char carry;
  ge_p1p1 r;
  ge_p2 s;
  ge_p3 t0, t1, t2;
  ge_cached t, pre[8];
  int i;

  for (i = 0;i < 32;++i) {
    e[2 * i + 0] = (a[i] >> 0) & 15;
    e[2 * i + 1] = (a[i] >> 4) & 15;
  }
  /* each e[i] is between 0 and 15 */
  /* e[63] is between 0 and 7 */

  carry = 0;
  for (i = 0;i < 63;++i) {
    e[i] += carry;
    carry = e[i] + 8;
    carry >>= 4;
    e[i] -= carry << 4;
  }
  e[63] += carry;
  /* each e[i] is between -8 and 8 */

  // Precomputation:
  ge_p3_to_cached(pre+0, A); // A

  ge_p3_dbl(&r, A);
  ge_p1p1_to_p3(&t0, &r);
  ge_p3_to_cached(pre+1, &t0); // 2A

  ge_add(&r, A, pre+1);
  ge_p1p1_to_p3(&t1, &r);
  ge_p3_to_cached(pre+2, &t1); // 3A

  ge_p3_dbl(&r, &t0);
  ge_p1p1_to_p3(&t0, &r);
  ge_p3_to_cached(pre+3, &t0); // 4A

  ge_add(&r, A, pre+3);
  ge_p1p1_to_p3(&t2, &r);
  ge_p3_to_cached(pre+4, &t2); // 5A

  ge_p3_dbl(&r, &t1);
  ge_p1p1_to_p3(&t1, &r);
  ge_p3_to_cached(pre+5, &t1); // 6A

  ge_add(&r, A, pre+5);
  ge_p1p1_to_p3(&t1, &r);
  ge_p3_to_cached(pre+6, &t1); // 7A

  ge_p3_dbl(&r, &t0);
  ge_p1p1_to_p3(&t0, &r);
  ge_p3_to_cached(pre+7, &t0); // 8A

  ge_p3_0(h);

  for (i = 63;i > 0; i--) {
    select(&t,pre,e[i]);
    ge_add(&r, h, &t);
    ge_p1p1_to_p2(&s,&r);

    ge_p2_dbl(&r,&s); ge_p1p1_to_p2(&s,&r);
    ge_p2_dbl(&r,&s); ge_p1p1_to_p2(&s,&r);
    ge_p2_dbl(&r,&s); ge_p1p1_to_p2(&s,&r);
    ge_p2_dbl(&r,&s); ge_p1p1_to_p3(h,&r);

  }
  select(&t,pre,e[0]);
  ge_add(&r, h, &t);
  ge_p1p1_to_p3(h,&r);
}
