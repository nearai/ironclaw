#include "fe.h"

void fe_mont_rhs(fe v2, fe u) {
  fe A, one;
  fe u2, Au, inner;

  fe_1(one);
  fe_0(A);
  A[0] = 486662;                     /* A = 486662 */

  fe_sq(u2, u);                      /* u^2 */
  fe_mul(Au, A, u);                  /* Au */
  fe_add(inner, u2, Au);             /* u^2 + Au */
  fe_add(inner, inner, one);         /* u^2 + Au + 1 */
  fe_mul(v2, u, inner);              /* u(u^2 + Au + 1) */
}

