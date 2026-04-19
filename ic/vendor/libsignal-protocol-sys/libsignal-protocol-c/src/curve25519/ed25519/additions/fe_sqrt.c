#include <assert.h>
#include "fe.h"
#include "crypto_additions.h"

/* sqrt(-1) */
static unsigned char i_bytes[32] = {
  0xb0, 0xa0, 0x0e, 0x4a, 0x27, 0x1b, 0xee, 0xc4, 
  0x78, 0xe4, 0x2f, 0xad, 0x06, 0x18, 0x43, 0x2f, 
  0xa7, 0xd7, 0xfb, 0x3d, 0x99, 0x00, 0x4d, 0x2b, 
  0x0b, 0xdf, 0xc1, 0x4f, 0x80, 0x24, 0x83, 0x2b
};

/* Preconditions: a is square or zero */

void fe_sqrt(fe out, const fe a)
{
  fe exp, b, b2, bi, i;
#ifndef NDEBUG
  fe legendre, zero, one;
#endif

  fe_frombytes(i, i_bytes); 
  fe_pow22523(exp, a);             /* b = a^(q-5)/8        */

  /* PRECONDITION: legendre symbol == 1 (square) or 0 (a == zero) */
#ifndef NDEBUG
  fe_sq(legendre, exp);            /* in^((q-5)/4) */ 
  fe_sq(legendre, legendre);       /* in^((q-5)/2) */
  fe_mul(legendre, legendre, a);   /* in^((q-3)/2) */
  fe_mul(legendre, legendre, a);   /* in^((q-1)/2) */
  
  fe_0(zero);
  fe_1(one);
  assert(fe_isequal(legendre, zero) || fe_isequal(legendre, one));
#endif

  fe_mul(b, a, exp);       /* b = a * a^(q-5)/8    */
  fe_sq(b2, b);            /* b^2 = a * a^(q-1)/4  */

  /* note b^4 == a^2, so b^2 == a or -a
   * if b^2 != a, multiply it by sqrt(-1) */
  fe_mul(bi, b, i);  
  fe_cmov(b, bi, 1 ^ fe_isequal(b2, a));
  fe_copy(out, b);

  /* PRECONDITION: out^2 == a */ 
#ifndef NDEBUG
  fe_sq(b2, out);
  assert(fe_isequal(a, b2));
#endif
}
