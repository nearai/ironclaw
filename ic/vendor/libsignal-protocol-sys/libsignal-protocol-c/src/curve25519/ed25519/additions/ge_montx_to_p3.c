#include "fe.h"
#include "ge.h"
#include "assert.h"
#include "crypto_additions.h"
#include "utility.h"

/* sqrt(-(A+2)) */
static unsigned char A_bytes[32] = {
  0x06, 0x7e, 0x45, 0xff, 0xaa, 0x04, 0x6e, 0xcc, 
  0x82, 0x1a, 0x7d, 0x4b, 0xd1, 0xd3, 0xa1, 0xc5, 
  0x7e, 0x4f, 0xfc, 0x03, 0xdc, 0x08, 0x7b, 0xd2, 
  0xbb, 0x06, 0xa0, 0x60, 0xf4, 0xed, 0x26, 0x0f
};

void ge_montx_to_p3(ge_p3* p, const fe u, const unsigned char ed_sign_bit)
{
  fe x, y, A, v, v2, iv, nx;

  fe_frombytes(A, A_bytes); 

  /* given u, recover edwards y */
  /* given u, recover v */
  /* given u and v, recover edwards x */

  fe_montx_to_edy(y, u);       /* y = (u - 1) / (u + 1) */

  fe_mont_rhs(v2, u);          /* v^2 = u(u^2 + Au + 1) */
  fe_sqrt(v, v2);              /* v = sqrt(v^2) */

  fe_mul(x, u, A);             /* x = u * sqrt(-(A+2)) */
  fe_invert(iv, v);            /* 1/v */
  fe_mul(x, x, iv);            /* x = (u/v) * sqrt(-(A+2)) */

  fe_neg(nx, x);               /* negate x to match sign bit */
  fe_cmov(x, nx, fe_isnegative(x) ^ ed_sign_bit);

  fe_copy(p->X, x);
  fe_copy(p->Y, y);
  fe_1(p->Z);
  fe_mul(p->T, p->X, p->Y);

 /* POSTCONDITION: check that p->X and p->Y satisfy the Ed curve equation */
 /* -x^2 + y^2 = 1 + dx^2y^2 */
#ifndef NDEBUG
  {
  fe one, d, x2, y2, x2y2, dx2y2;

  unsigned char dbytes[32] = {
  0xa3, 0x78, 0x59, 0x13, 0xca, 0x4d, 0xeb, 0x75,
  0xab, 0xd8, 0x41, 0x41, 0x4d, 0x0a, 0x70, 0x00,
  0x98, 0xe8, 0x79, 0x77, 0x79, 0x40, 0xc7, 0x8c,
  0x73, 0xfe, 0x6f, 0x2b, 0xee, 0x6c, 0x03, 0x52
  };
  
  fe_frombytes(d, dbytes);
  fe_1(one);
  fe_sq(x2, p->X);                /* x^2 */
  fe_sq(y2, p->Y);                /* y^2 */

  fe_mul(dx2y2, x2, y2);           /* x^2y^2 */
  fe_mul(dx2y2, dx2y2, d);         /* dx^2y^2 */
  fe_add(dx2y2, dx2y2, one);       /* dx^2y^2 + 1 */

  fe_neg(x2y2, x2);                /* -x^2 */
  fe_add(x2y2, x2y2, y2);          /* -x^2 + y^2 */

  assert(fe_isequal(x2y2, dx2y2));
  }
#endif
}
