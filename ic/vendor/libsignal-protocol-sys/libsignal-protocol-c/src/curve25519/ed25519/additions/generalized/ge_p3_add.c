#include "ge.h"

/*
r = p + q
*/

void ge_p3_add(ge_p3 *r, const ge_p3 *p, const ge_p3 *q)
{
  ge_cached p_cached;
  ge_p1p1 r_p1p1;

  ge_p3_to_cached(&p_cached, p);
  ge_add(&r_p1p1, q, &p_cached);
  ge_p1p1_to_p3(r, &r_p1p1);
}
